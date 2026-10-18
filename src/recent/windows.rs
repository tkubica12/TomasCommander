use super::*;
use ::windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_IO_INCOMPLETE, ERROR_NOTIFY_ENUM_DIR, HANDLE},
        Storage::FileSystem::{
            FILE_ATTRIBUTE_TAG_INFO, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
            FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FileAttributeTagInfo,
            GetFileInformationByHandleEx, ReadDirectoryChangesW,
        },
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize, IPersistFile, STGM_READ,
            },
            IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
            Threading::{CreateEventW, ResetEvent},
        },
        UI::Shell::{
            FOLDERID_Recent, IShellLinkW, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath, ShellLink,
        },
    },
    core::{Interface, PCWSTR},
};
use std::os::windows::{
    ffi::{OsStrExt, OsStringExt},
    fs::OpenOptionsExt,
    io::AsRawHandle,
};

pub(super) struct Apartment;
impl Apartment {
    pub(super) fn new() -> FileResult<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|error| format!("Initialize Windows Recent: {error}"))?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

pub(super) fn default_roots() -> FileResult<Vec<PathBuf>> {
    let mut roots = vec![PathBuf::from(r"D:\")];
    for variable in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        if let Some(path) = std::env::var_os(variable) {
            roots.push(path.into());
        }
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        for entry in
            fs::read_dir(&profile).map_err(|error| format!("Find OneDrive roots: {error}"))?
        {
            let entry = entry.map_err(|error| format!("Find OneDrive root: {error}"))?;
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if (name == "onedrive" || name.starts_with("onedrive - "))
                && entry
                    .file_type()
                    .map_err(|error| format!("Inspect OneDrive root: {error}"))?
                    .is_dir()
            {
                roots.push(entry.path());
            }
        }
    }
    Ok(roots)
}

pub(super) fn recent_folder() -> FileResult<PathBuf> {
    let pointer = unsafe { SHGetKnownFolderPath(&FOLDERID_Recent, KF_FLAG_DONT_VERIFY, None) }
        .map_err(|error| format!("Locate Windows Recent: {error}"))?;
    let text = unsafe { pointer.to_string() };
    unsafe { CoTaskMemFree(Some(pointer.0.cast())) };
    text.map(PathBuf::from)
        .map_err(|error| format!("Read Windows Recent location: {error}"))
}

pub(super) fn cloud_directory(path: &Path) -> FileResult<bool> {
    let handle = OpenOptions::new()
        .access_mode(0)
        .share_mode(7)
        .custom_flags(0x0220_0000)
        .open(path)
        .map_err(|error| format!("Inspect Recent folder reparse point: {error}"))?;
    let mut information = FILE_ATTRIBUTE_TAG_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(handle.as_raw_handle()),
            FileAttributeTagInfo,
            (&mut information as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of_val(&information) as u32,
        )
    }
    .map_err(|error| format!("Read Recent folder tag: {error}"))?;
    Ok(information.ReparseTag & 0xffff_0fff == 0x9000_001a)
}

pub(super) struct Watch {
    pub root: PathBuf,
    file: fs::File,
    buffer: Vec<u32>,
    overlapped: Box<OVERLAPPED>,
    recursive: bool,
    pending: bool,
    failed: bool,
}

impl Watch {
    pub(super) fn new(root: &Path, recursive: bool) -> FileResult<Self> {
        let file = OpenOptions::new()
            .access_mode(1)
            .share_mode(7)
            .custom_flags(0x4200_0000)
            .open(root)
            .map_err(|error| {
                format!("Monitor Recent root {}: {error}", files::display_path(root))
            })?;
        let event = unsafe { CreateEventW(None, true, false, None) }
            .map_err(|error| format!("Create Recent notification event: {error}"))?;
        let mut watch = Self {
            root: root.into(),
            file,
            buffer: vec![0; 16_384],
            overlapped: Box::new(OVERLAPPED {
                hEvent: event,
                ..Default::default()
            }),
            recursive,
            pending: false,
            failed: false,
        };
        watch.arm()?;
        Ok(watch)
    }
    fn arm(&mut self) -> FileResult<()> {
        unsafe {
            ResetEvent(self.overlapped.hEvent)
                .map_err(|error| format!("Reset Recent event: {error}"))?;
            ReadDirectoryChangesW(
                HANDLE(self.file.as_raw_handle()),
                self.buffer.as_mut_ptr().cast(),
                (self.buffer.len() * 4) as u32,
                self.recursive,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_DIR_NAME
                    | FILE_NOTIFY_CHANGE_LAST_WRITE
                    | FILE_NOTIFY_CHANGE_SIZE,
                None,
                Some(&mut *self.overlapped),
                None,
            )
        }
        .map_err(|error| {
            format!(
                "Watch Recent changes {}: {error}",
                files::display_path(&self.root)
            )
        })?;
        self.pending = true;
        Ok(())
    }
    pub(super) fn poll(&mut self) -> FileResult<Option<Vec<(PathBuf, bool)>>> {
        if self.failed {
            return Ok(None);
        }
        let mut bytes = 0;
        match unsafe {
            GetOverlappedResult(
                HANDLE(self.file.as_raw_handle()),
                &*self.overlapped,
                &mut bytes,
                false,
            )
        } {
            Err(error) if error.code() == ERROR_IO_INCOMPLETE.to_hresult() => return Ok(None),
            Err(error) if error.code() == ERROR_NOTIFY_ENUM_DIR.to_hresult() => {
                self.pending = false;
                if let Err(error) = self.arm() {
                    self.failed = true;
                    return Err(error);
                }
                return Err(
                    "Recent notification buffer overflow; bounded reconciliation requested.".into(),
                );
            }
            Err(error) => {
                self.failed = true;
                return Err(format!(
                    "Recent monitoring stopped for {}: {error}",
                    files::display_path(&self.root)
                ));
            }
            Ok(()) => self.pending = false,
        }
        let result = if bytes == 0 || bytes as usize > self.buffer.len() * 4 {
            Err("Recent notification buffer overflow; bounded reconciliation requested.".into())
        } else {
            let buffer = unsafe {
                std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), bytes as usize)
            };
            parse_notifications(&self.root, buffer).map(Some)
        };
        if let Err(error) = self.arm() {
            self.failed = true;
            return Err(error);
        }
        result
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if self.pending {
            // Cancellation is asynchronous; keep the buffer and OVERLAPPED alive until completion.
            unsafe {
                let _ = CancelIoEx(HANDLE(self.file.as_raw_handle()), Some(&*self.overlapped));
                let mut bytes = 0;
                let _ = GetOverlappedResult(
                    HANDLE(self.file.as_raw_handle()),
                    &*self.overlapped,
                    &mut bytes,
                    true,
                );
            }
        }
        unsafe {
            let _ = CloseHandle(self.overlapped.hEvent);
        }
    }
}

fn parse_notifications(root: &Path, mut bytes: &[u8]) -> FileResult<Vec<(PathBuf, bool)>> {
    let mut events = Vec::new();
    loop {
        if bytes.len() < 12 {
            return Err("Truncated Recent notification.".into());
        }
        let number = |index| {
            u32::from_le_bytes(
                bytes[index..index + 4]
                    .try_into()
                    .expect("Checked notification header"),
            )
        };
        let next = number(0) as usize;
        let action = number(4);
        let length = number(8) as usize;
        if !length.is_multiple_of(2) || length > bytes.len() - 12 {
            return Err("Invalid Recent notification filename.".into());
        }
        let text: Vec<_> = bytes[12..12 + length]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let relative = PathBuf::from(std::ffi::OsString::from_wide(&text));
        if relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
            && !relative.as_os_str().is_empty()
        {
            events.push((root.join(relative), matches!(action, 2 | 4)));
        } else {
            return Err("Recent notification escaped its root.".into());
        }
        if next == 0 {
            return Ok(events);
        }
        if next < 12 + length || next >= bytes.len() || !next.is_multiple_of(4) {
            return Err("Invalid Recent notification chain.".into());
        }
        bytes = &bytes[next..];
    }
}

pub(super) fn import_link(
    link_path: &Path,
    roots: &[PathBuf],
    snapshot: &mut Snapshot,
) -> FileResult<()> {
    if roots.is_empty() {
        return Ok(());
    }
    if !link_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
    {
        return Ok(());
    }
    let metadata = match fs::metadata(link_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Inspect Windows Recent shortcut: {error}")),
    };
    if metadata.len() > 1_048_576 {
        return Err("Windows Recent shortcut exceeds its size limit.".into());
    }
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .map_err(|error| format!("Read Windows Recent shortcut: {error}"))?;
    let persistent: IPersistFile = link
        .cast()
        .map_err(|error| format!("Read Recent shortcut interface: {error}"))?;
    let shell_path = crate::platform::shell_path(link_path)?;
    let value: Vec<_> = shell_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe { persistent.Load(PCWSTR(value.as_ptr()), STGM_READ) }
        .map_err(|error| format!("Load Windows Recent shortcut: {error}"))?;
    let mut value = vec![0; 32768];
    // GetPath reads the stored local target; never Resolve, execute, or hydrate it.
    unsafe { link.GetPath(&mut value, std::ptr::null_mut(), 0) }
        .map_err(|error| format!("Read Windows Recent target: {error}"))?;
    let length = value
        .iter()
        .position(|value| *value == 0)
        .ok_or("Unterminated Windows Recent target.")?;
    let path = PathBuf::from(std::ffi::OsString::from_wide(&value[..length]));
    if !roots
        .iter()
        .any(|root| key(&path).starts_with(&(key(root).trim_end_matches('\\').to_owned() + "\\")))
    {
        return Ok(());
    }
    if let Some(item) = inspect(
        &path,
        Activity::WindowsRecent,
        Some(milliseconds(metadata.modified().map_err(|error| {
            format!("Read Windows Recent time: {error}")
        })?)),
        Some(roots),
    )? {
        snapshot.record(item);
    }
    Ok(())
}

pub(super) fn import_shell(
    folder: &Path,
    roots: &[PathBuf],
    snapshot: &mut Snapshot,
    stop: &AtomicBool,
) -> FileResult<()> {
    let started = Instant::now();
    for (index, entry) in fs::read_dir(folder)
        .map_err(|error| format!("Read Windows Recent: {error}"))?
        .enumerate()
    {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if index >= 2000 || started.elapsed() >= Duration::from_secs(2) {
            snapshot
                .warn("Windows Recent startup import is partial (2,000 links / 2 seconds).".into());
            break;
        }
        let entry = entry.map_err(|error| format!("Read Windows Recent entry: {error}"))?;
        if let Err(error) = import_link(&entry.path(), roots, snapshot) {
            snapshot.warn(error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(name: &str, action: u32) -> Vec<u8> {
        let name: Vec<_> = name.encode_utf16().collect();
        let mut bytes = Vec::new();
        bytes.extend(0_u32.to_le_bytes());
        bytes.extend(action.to_le_bytes());
        bytes.extend(((name.len() * 2) as u32).to_le_bytes());
        for character in name {
            bytes.extend(character.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn recent_native_notification_parser_checks_bounds_chains_unicode_and_containment() {
        let root = PathBuf::from(r"D:\synthetic");
        let mut first = notification("first.md", 2);
        while !first.len().is_multiple_of(4) {
            first.push(0);
        }
        let next = first.len() as u32;
        first[..4].copy_from_slice(&next.to_le_bytes());
        first.extend(notification("Příliš\\žluťoučký.xlsx", 5));
        assert_eq!(
            parse_notifications(&root, &first).unwrap(),
            vec![
                (root.join("first.md"), true),
                (root.join("Příliš\\žluťoučký.xlsx"), false),
            ]
        );
        assert!(parse_notifications(&root, &notification("..\\escape.md", 1)).is_err());
        assert!(parse_notifications(&root, &notification(r"C:\escape.md", 1)).is_err());
        assert!(parse_notifications(&root, &[0; 11]).is_err());
        let mut invalid = notification("note.md", 1);
        invalid[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_notifications(&root, &invalid).is_err());
        assert!(parse_notifications(&root, &first[..first.len() - 1]).is_err());
    }

    #[test]
    fn recent_reads_owned_shell_shortcut_without_registering_system_history_or_executing() {
        let fixture = super::super::tests::Fixture::new();
        let _apartment = Apartment::new().unwrap();
        let target = fixture.write("owned presentation.pptx", b"synthetic");
        let shortcut = fixture.root.join("owned.lnk");
        let link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.unwrap();
        let target_shell = crate::platform::shell_path(&target).unwrap();
        let path: Vec<_> = target_shell
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe { link.SetPath(PCWSTR(path.as_ptr())) }.unwrap();
        let persistent: IPersistFile = link.cast().unwrap();
        let shortcut_shell = crate::platform::shell_path(&shortcut).unwrap();
        let path: Vec<_> = shortcut_shell
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe { persistent.Save(PCWSTR(path.as_ptr()), true) }.unwrap();
        let mut snapshot = Snapshot::default();
        import_link(
            &shortcut,
            std::slice::from_ref(&fixture.root),
            &mut snapshot,
        )
        .unwrap();
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].path, target);
        assert_eq!(snapshot.items[0].activity, Activity::WindowsRecent);
        assert_eq!(fs::read(&target).unwrap(), b"synthetic");
        let mut outside = Snapshot::default();
        import_link(
            &shortcut,
            &[fixture.root.join("not-approved")],
            &mut outside,
        )
        .unwrap();
        assert!(outside.items.is_empty());
    }
}

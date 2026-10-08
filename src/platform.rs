use crate::files::FileResult;
use std::{path::Path, process::Command};

#[cfg(windows)]
pub fn file_identity(file: &std::fs::File) -> FileResult<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
    };
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information) }
        .map_err(|e| format!("Read Windows file identity: {e}"))?;
    Ok((
        information.dwVolumeSerialNumber,
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    ))
}

#[cfg(windows)]
pub fn path_identity(path: &Path) -> FileResult<(u32, u64)> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(7)
        .custom_flags(0x0220_0000)
        .open(path)
        .map_err(|e| {
            format!(
                "Open identity handle {}: {e}",
                crate::files::display_path(path)
            )
        })?;
    file_identity(&file)
}

pub fn show_error(message: &str) {
    eprintln!("Tomas Commander: {message}");
    #[cfg(windows)]
    {
        use windows::{
            Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
            core::{PCWSTR, w},
        };
        let value: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(value.as_ptr()),
                w!("Tomas Commander startup error"),
                MB_OK | MB_ICONERROR,
            );
        };
    }
}

#[cfg(windows)]
fn wide(path: &Path) -> FileResult<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err("Path contains a null character.".into());
    }
    value.push(0);
    Ok(value)
}

#[cfg(windows)]
fn shell_path(path: &Path) -> FileResult<std::path::PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let value: Vec<u16> = path.as_os_str().encode_wide().collect();
    let normalized = if let Some(unc) = value.strip_prefix(&[92, 92, 63, 92, 85, 78, 67, 92]) {
        [vec![92, 92], unc.to_vec()].concat()
    } else if let Some(local) = value.strip_prefix(&[92, 92, 63, 92]) {
        if local.get(1) != Some(&58) || local.get(2) != Some(&92) {
            return Err("This extended path cannot be passed safely to the Windows Shell.".into());
        }
        local.to_vec()
    } else {
        value
    };
    Ok(std::ffi::OsString::from_wide(&normalized).into())
}

#[cfg(windows)]
pub fn open_file(path: &Path) -> FileResult<()> {
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::{PCWSTR, w},
    };
    let value = wide(&shell_path(path)?)?;
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(value.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    let status = result.0 as isize;
    if status <= 32 {
        return Err(format!(
            "Windows could not open {} using its associated application (Shell error {status}).",
            crate::files::display_path(path)
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn open_file(_path: &Path) -> FileResult<()> {
    Err("Opening files with their associated application requires Windows.".into())
}

#[cfg(windows)]
pub fn move_no_replace(source: &Path, target: &Path) -> FileResult<()> {
    let parent = target.parent().ok_or("Move target has no parent folder.")?;
    move_verified(
        source,
        target,
        &crate::files::fingerprint(source)?,
        path_identity(parent)?,
    )
}

#[cfg(windows)]
pub fn move_verified(
    source: &Path,
    target: &Path,
    expected_source: &crate::files::Fingerprint,
    expected_parent: (u32, u64),
) -> FileResult<()> {
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{FILE_RENAME_INFO, FileRenameInfo, SetFileInformationByHandle},
    };
    let parent = target.parent().ok_or("Move target has no parent folder.")?;
    let mut guards = Vec::new();
    let ancestors: std::collections::BTreeSet<_> = source
        .parent()
        .ok_or("Move source has no parent folder.")?
        .ancestors()
        .chain(parent.ancestors())
        .collect();
    for ancestor in ancestors {
        let handle = std::fs::OpenOptions::new()
            .access_mode(0)
            .share_mode(3)
            .custom_flags(0x0220_0000)
            .open(ancestor)
            .map_err(|e| {
                format!(
                    "Lock move ancestor {}: {e}",
                    crate::files::display_path(ancestor)
                )
            })?;
        if crate::files::reparse(
            &handle
                .metadata()
                .map_err(|e| format!("Inspect move ancestor: {e}"))?,
        ) {
            return Err("Move ancestor changed to a link; no move performed.".into());
        }
        guards.push(handle);
    }
    let destination = std::fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(3)
        .custom_flags(0x0220_0000)
        .open(parent)
        .map_err(|e| format!("Open move destination handle: {e}"))?;
    let source_handle = std::fs::OpenOptions::new()
        .access_mode(0x0001_0080)
        .share_mode(1)
        .custom_flags(0x0220_0000)
        .open(source)
        .map_err(|e| format!("Open approved move handle: {e}"))?;
    let metadata = source_handle
        .metadata()
        .map_err(|e| format!("Inspect approved move handle: {e}"))?;
    if file_identity(&source_handle)? != expected_source.file_id
        || file_identity(&destination)? != expected_parent
        || crate::files::reparse(&metadata)
    {
        return Err("Move identity changed while opening handles; no move performed.".into());
    }
    if metadata.is_dir() != expected_source.directory
        || metadata.len() != expected_source.size
        || metadata
            .modified()
            .map_err(|e| format!("Read approved move modification time: {e}"))?
            != expected_source.modified
        || metadata
            .created()
            .map_err(|e| format!("Read approved move creation time: {e}"))?
            != expected_source.created
    {
        return Err("Move source changed while opening its handle; no move performed.".into());
    }
    let name: Vec<u16> = shell_path(target)?.as_os_str().encode_wide().collect();
    if name.contains(&0) {
        return Err("Move filename contains a null character.".into());
    }
    let name_bytes = name
        .len()
        .checked_mul(2)
        .ok_or("Move filename is too long.")?;
    let bytes = std::mem::size_of::<FILE_RENAME_INFO>()
        .checked_add(name_bytes)
        .ok_or("Move buffer is too large.")?;
    let length = u32::try_from(bytes).map_err(|_| "Move buffer exceeds Windows limits.")?;
    // A vector of the native structure provides its required alignment and flexible-array storage.
    let mut buffer =
        vec![FILE_RENAME_INFO::default(); bytes.div_ceil(std::mem::size_of::<FILE_RENAME_INFO>())];
    let information = buffer.as_mut_ptr();
    unsafe {
        (*information).Anonymous.ReplaceIfExists = false;
        (*information).RootDirectory = HANDLE::default();
        (*information).FileNameLength = u32::try_from(name_bytes).map_err(|_| "Move filename exceeds Windows limits.")?;
        std::ptr::copy_nonoverlapping(name.as_ptr(), std::ptr::addr_of_mut!((*information).FileName).cast::<u16>(), name.len());
        SetFileInformationByHandle(HANDLE(source_handle.as_raw_handle()), FileRenameInfo, information.cast(), length)
    }.map_err(|e| {
        format!(
            "Move approved handle {} -> {} without replacement: {e}. Cross-volume moves are not supported.",
            crate::files::display_path(source),
            crate::files::display_path(target)
        )
    })
}

#[cfg(not(windows))]
pub fn move_no_replace(_source: &Path, _target: &Path) -> FileResult<()> {
    Err("Non-overwriting native moves currently require Windows.".into())
}

#[cfg(windows)]
pub fn recycle(path: &Path) -> FileResult<()> {
    use windows::{
        Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        Win32::UI::Shell::{
            FOF_ALLOWUNDO, FOF_NO_CONNECTED_ELEMENTS, FOF_NOCONFIRMATION, FOF_NOERRORUI,
            FOF_SILENT, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FileOperation, IFileOperation,
            IFileOperationProgressSink, IShellItem, SHCreateItemFromParsingName,
        },
        core::PCWSTR,
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    let value = wide(&shell_path(path)?)?;
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| format!("Initialize Windows recycling: {e}"))?;
        let _apartment = Apartment;
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("Create Windows recycle operation: {e}"))?;
        let item: IShellItem =
            SHCreateItemFromParsingName(PCWSTR(value.as_ptr()), None).map_err(|e| {
                format!(
                    "Resolve recycle target {}: {e}",
                    crate::files::display_path(path)
                )
            })?;
        operation
            .SetOperationFlags(
                FOF_ALLOWUNDO
                    | FOF_NOERRORUI
                    | FOF_NOCONFIRMATION
                    | FOF_SILENT
                    | FOF_NO_CONNECTED_ELEMENTS
                    | FOFX_RECYCLEONDELETE
                    | FOFX_EARLYFAILURE,
            )
            .map_err(|e| format!("Configure recycle-only operation: {e}"))?;
        let guard: IFileOperationProgressSink = recycle_guard::RecycleOnly.into();
        operation
            .DeleteItem(&item, &guard)
            .map_err(|e| format!("Queue recycle: {e}"))?;
        operation
            .PerformOperations()
            .map_err(|e| format!("Recycle {}: {e}", crate::files::display_path(path)))?;
        if operation
            .GetAnyOperationsAborted()
            .map_err(|e| format!("Read recycling outcome: {e}"))?
            .as_bool()
        {
            return Err("Windows aborted recycling. Inspect the source before retrying.".into());
        }
    }
    Ok(())
}

#[cfg(windows)]
mod recycle_guard {
    use windows::{
        Win32::{
            Foundation::E_ABORT,
            UI::Shell::{
                IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem,
                TSF_DELETE_RECYCLE_IF_POSSIBLE,
            },
        },
        core::{HRESULT, PCWSTR, Ref, Result, implement},
    };

    #[implement(IFileOperationProgressSink)]
    pub struct RecycleOnly;

    #[allow(non_snake_case)]
    impl IFileOperationProgressSink_Impl for RecycleOnly_Impl {
        fn StartOperations(&self) -> Result<()> {
            Ok(())
        }
        fn FinishOperations(&self, result: HRESULT) -> Result<()> {
            result.ok()
        }
        fn PreRenameItem(&self, _flags: u32, _item: Ref<IShellItem>, _name: &PCWSTR) -> Result<()> {
            Err(E_ABORT.into())
        }
        fn PostRenameItem(
            &self,
            _flags: u32,
            _item: Ref<IShellItem>,
            _name: &PCWSTR,
            result: HRESULT,
            _created: Ref<IShellItem>,
        ) -> Result<()> {
            result.ok()
        }
        fn PreMoveItem(
            &self,
            _flags: u32,
            _item: Ref<IShellItem>,
            _destination: Ref<IShellItem>,
            _name: &PCWSTR,
        ) -> Result<()> {
            Err(E_ABORT.into())
        }
        fn PostMoveItem(
            &self,
            _flags: u32,
            _item: Ref<IShellItem>,
            _destination: Ref<IShellItem>,
            _name: &PCWSTR,
            result: HRESULT,
            _created: Ref<IShellItem>,
        ) -> Result<()> {
            result.ok()
        }
        fn PreCopyItem(
            &self,
            _flags: u32,
            _item: Ref<IShellItem>,
            _destination: Ref<IShellItem>,
            _name: &PCWSTR,
        ) -> Result<()> {
            Err(E_ABORT.into())
        }
        fn PostCopyItem(
            &self,
            _flags: u32,
            _item: Ref<IShellItem>,
            _destination: Ref<IShellItem>,
            _name: &PCWSTR,
            result: HRESULT,
            _created: Ref<IShellItem>,
        ) -> Result<()> {
            result.ok()
        }
        fn PreDeleteItem(&self, flags: u32, _item: Ref<IShellItem>) -> Result<()> {
            if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0 {
                return Err(E_ABORT.into());
            }
            Ok(())
        }
        fn PostDeleteItem(
            &self,
            _flags: u32,
            _item: Ref<IShellItem>,
            result: HRESULT,
            _created: Ref<IShellItem>,
        ) -> Result<()> {
            result.ok()
        }
        fn PreNewItem(
            &self,
            _flags: u32,
            _destination: Ref<IShellItem>,
            _name: &PCWSTR,
        ) -> Result<()> {
            Err(E_ABORT.into())
        }
        fn PostNewItem(
            &self,
            _flags: u32,
            _destination: Ref<IShellItem>,
            _name: &PCWSTR,
            _template: &PCWSTR,
            _attributes: u32,
            result: HRESULT,
            _created: Ref<IShellItem>,
        ) -> Result<()> {
            result.ok()
        }
        fn UpdateProgress(&self, _total: u32, _done: u32) -> Result<()> {
            Ok(())
        }
        fn ResetTimer(&self) -> Result<()> {
            Ok(())
        }
        fn PauseTimer(&self) -> Result<()> {
            Ok(())
        }
        fn ResumeTimer(&self) -> Result<()> {
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn guard_refuses_permanent_deletion_flags() {
            let guard: IFileOperationProgressSink = RecycleOnly.into();
            unsafe {
                assert!(guard.PreDeleteItem(0, None).is_err());
                assert!(
                    guard
                        .PreDeleteItem(TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32, None)
                        .is_ok()
                );
            }
        }
    }
}

#[cfg(not(windows))]
pub fn recycle(_path: &Path) -> FileResult<()> {
    Err("Recycle Bin support currently requires Windows; permanent deletion is never used.".into())
}

pub fn open_vscode(path: &Path) -> FileResult<()> {
    let mut candidates = Vec::new();
    for (variable, suffix) in [
        ("LOCALAPPDATA", "Programs\\Microsoft VS Code\\Code.exe"),
        ("ProgramFiles", "Microsoft VS Code\\Code.exe"),
        ("ProgramFiles(x86)", "Microsoft VS Code\\Code.exe"),
    ] {
        if let Some(root) = std::env::var_os(variable) {
            candidates.push(std::path::PathBuf::from(root).join(suffix));
        }
    }
    let executable = candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or("VS Code executable not found in standard locations. No shell fallback was used.")?;
    Command::new(executable)
        .arg("--new-window")
        .arg(path)
        .spawn()
        .map_err(|e| {
            format!(
                "Launch VS Code for {}: {e}",
                crate::files::display_path(path)
            )
        })?;
    Ok(())
}

pub fn find_copilot(
    paths: impl IntoIterator<Item = std::path::PathBuf>,
) -> FileResult<std::path::PathBuf> {
    paths.into_iter().filter(|path| path.is_absolute())
                .map(|path| path.join("copilot.exe")).find(|path| path.is_file())
                .ok_or_else(|| "GitHub Copilot CLI (copilot.exe) was not found on PATH. Install the native CLI and restart Tomas Commander.".into())
}

#[cfg(windows)]
pub fn open_copilot(path: &Path, app: bool) -> FileResult<()> {
    use std::{
        os::windows::process::CommandExt,
        process::Stdio,
        time::{Duration, Instant},
    };
    let path_variable =
        std::env::var_os("PATH").ok_or("PATH is unavailable; cannot locate Copilot CLI.")?;
    let executable = find_copilot(std::env::split_paths(&path_variable))?;
    if !app {
        use windows::{
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
            core::{PCWSTR, w},
        };
        let executable = wide(&shell_path(&executable)?)?;
        let directory = wide(&shell_path(path)?)?;
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(executable.as_ptr()),
                w!("--yolo"),
                PCWSTR(directory.as_ptr()),
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize <= 32 {
            return Err(format!(
                "Windows could not open the Copilot CLI terminal (Shell error {}).",
                result.0 as isize
            ));
        }
        return Ok(());
    }
    let mut command = Command::new(executable);
    command.current_dir(path).stdin(Stdio::null());
    let windows = std::env::var_os("WINDIR")
        .ok_or("WINDIR is unavailable; cannot check Copilot app registration.")?;
    let registered = Command::new(
        std::path::PathBuf::from(windows)
            .join("System32")
            .join("reg.exe"),
    )
    .args(["query", "HKCR\\ghapp", "/v", "URL Protocol"])
    .creation_flags(0x0800_0000)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .map_err(|e| format!("Check GitHub Copilot app registration: {e}"))?;
    if !registered.success() {
        return Err("GitHub Copilot app is not registered on this machine. Install or repair the app before using this command; no browser/download fallback was opened.".into());
    }
    let mut child = command
        .arg("app")
        .creation_flags(0x0800_0000)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Launch Copilot app: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("Read Copilot app launch outcome: {e}"))?
        {
            if status.success() {
                return Ok(());
            }
            use std::io::Read;
            let mut detail = String::new();
            if let Some(stderr) = child.stderr.take() {
                stderr.take(8192).read_to_string(&mut detail).map_err(|e| {
                    format!("Copilot app launch failed ({status}); reading the error failed: {e}")
                })?;
            }
            return Err(format!(
                "Copilot app launch failed ({status}): {}. Check that your CLI supports 'copilot app' and the app is installed.",
                detail.trim()
            ));
        }
        if Instant::now() >= deadline {
            child.kill().map_err(|e| {
                format!("Copilot app launch timed out; stopping launcher failed: {e}")
            })?;
            child
                .wait()
                .map_err(|e| format!("Copilot app launch timed out; read-back failed: {e}"))?;
            return Err("Copilot app launch timed out. Inspect the app before retrying.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(not(windows))]
pub fn open_copilot(_path: &Path, _app: bool) -> FileResult<()> {
    Err("Copilot terminal/app launch currently requires Windows.".into())
}

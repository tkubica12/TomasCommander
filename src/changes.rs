use crate::{
    files::{self, FileResult, Fingerprint, Outcome, Scope},
    journal::Journal,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::SystemTime,
};

pub const TEXT_LIMIT: usize = 1_048_576;

#[derive(Clone)]
pub struct Snapshot {
    pub path: PathBuf,
    pub(crate) identity: Fingerprint,
    pub(crate) bytes: Vec<u8>,
}

impl Snapshot {
    pub fn read(scope: &Scope, path: &Path, limit: usize) -> FileResult<Self> {
        let path = scope.mutation_source(path)?;
        let identity = files::fingerprint(&path)?;
        if identity.directory || identity.size > limit as u64 {
            return Err("Source is not a file or exceeds the operation input limit.".into());
        }
        let mut bytes = Vec::new();
        File::open(&path)
            .and_then(|file| file.take(limit as u64 + 1).read_to_end(&mut bytes))
            .map_err(|error| format!("Read source snapshot: {error}"))?;
        if bytes.len() > limit || files::fingerprint(&path)? != identity {
            return Err("Source changed while reading its snapshot.".into());
        }
        Ok(Self {
            path,
            identity,
            bytes,
        })
    }

    pub(crate) fn validate(&self, scope: &Scope) -> FileResult<File> {
        if scope.mutation_source(&self.path)? != self.path
            || files::fingerprint(&self.path)? != self.identity
        {
            return Err("Source changed since the exact plan was prepared.".into());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let mut file = options
            .open(&self.path)
            .map_err(|error| format!("Lock approved source: {error}"))?;
        let mut actual = Vec::new();
        Read::by_ref(&mut file)
            .take(self.bytes.len() as u64 + 1)
            .read_to_end(&mut actual)
            .map_err(|error| format!("Validate approved source bytes: {error}"))?;
        if actual != self.bytes || files::fingerprint(&self.path)? != self.identity {
            return Err("Source bytes changed since the exact plan was prepared.".into());
        }
        Ok(file)
    }
}

#[derive(Clone)]
pub struct Editor {
    original: Snapshot,
    initial: String,
    pub text: String,
    bom: bool,
    crlf: bool,
}

impl std::fmt::Debug for Editor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Editor")
            .field("path", &self.original.path)
            .field("dirty", &self.dirty())
            .finish_non_exhaustive()
    }
}

impl Editor {
    pub fn open(scope: &Scope, path: &Path) -> FileResult<Self> {
        if !editable(path) {
            return Err(
                "Editing is supported only for UTF-8 text, configuration and Markdown/HTML source."
                    .into(),
            );
        }
        let original = Snapshot::read(scope, path, TEXT_LIMIT)?;
        if fs::metadata(&original.path)
            .map_err(|error| format!("Inspect editor permissions: {error}"))?
            .permissions()
            .readonly()
        {
            return Err("This file is read-only; editing/Save is refused.".into());
        }
        let raw = std::str::from_utf8(&original.bytes)
            .map_err(|_| "Editor requires UTF-8; no transcoding is performed.")?;
        let bom = raw.starts_with('\u{feff}');
        let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
        let crlf = raw.contains("\r\n");
        let initial = raw.replace("\r\n", "\n");
        if initial.contains('\r')
            || (crlf && raw.replace("\r\n", "").contains('\n'))
            || raw.contains('\0')
        {
            return Err("Mixed/legacy newlines or binary text are not supported for editing; original bytes are unchanged.".into());
        }
        crate::viewer::validate_text(&initial)?;
        Ok(Self {
            original,
            text: initial.clone(),
            initial,
            bom,
            crlf,
        })
    }

    pub fn dirty(&self) -> bool {
        self.text != self.initial
    }

    pub fn plan(&self, id: u64) -> FileResult<Plan> {
        if !self.dirty() {
            return Err("No edited changes to save.".into());
        }
        if self.text.len() > TEXT_LIMIT || self.text.contains(['\0', '\r']) {
            return Err(
                "Edited text exceeds 1 MiB or contains unsupported NUL/CR characters.".into(),
            );
        }
        crate::viewer::validate_text(&self.text)?;
        if self
            .original
            .path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            serde_json::from_str::<serde_json::Value>(&self.text)
                .map_err(|error| format!("Refuse invalid JSON Save: {error}"))?;
        }
        let mut text = if self.crlf {
            self.text.replace('\n', "\r\n")
        } else {
            self.text.clone()
        };
        if self.bom {
            text.insert(0, '\u{feff}');
        }
        if text.len() > TEXT_LIMIT {
            return Err("Encoded Save exceeds 1 MiB after preserving BOM/newlines.".into());
        }
        Plan::replace(
            id,
            self.original.clone(),
            text.into_bytes(),
            "Save edited source",
        )
    }
}

pub fn editable(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || matches!(
            extension.as_str(),
            "txt"
                | "md"
                | "markdown"
                | "html"
                | "htm"
                | "json"
                | "yaml"
                | "yml"
                | "csv"
                | "toml"
                | "ini"
                | "env"
                | "log"
                | "xml"
                | "rs"
                | "py"
                | "ps1"
                | "sh"
                | "js"
                | "ts"
                | "css"
        )
}

#[derive(Clone)]
pub struct Plan {
    pub id: u64,
    pub label: &'static str,
    key: String,
    original: Snapshot,
    bytes: Vec<u8>,
    stage: PathBuf,
}

impl Plan {
    pub(crate) fn replace(
        id: u64,
        original: Snapshot,
        bytes: Vec<u8>,
        label: &'static str,
    ) -> FileResult<Self> {
        let key = identity()?;
        let stage = original
            .path
            .parent()
            .ok_or("Source has no parent folder.")?
            .join(format!(".tomas-commander-{key}.stage"));
        Ok(Self {
            id,
            label,
            key,
            original,
            bytes,
            stage,
        })
    }

    pub fn source(&self) -> &Path {
        &self.original.path
    }
    pub fn summary(&self) -> String {
        format!(
            "{}\nExact source/target: {}\n{} -> {} bytes\nUTF-8 encoding and original newline convention retained for text Save.\nAtomic Windows replacement preserves original ACL/attributes; no merge-error suppression.\nStaging path: {}\nCancel/error may retain this exact staging file; inspect Activity before retrying.",
            self.label,
            files::display_path(self.source()),
            self.original.bytes.len(),
            self.bytes.len(),
            files::display_path(&self.stage)
        )
    }

    pub fn execute(
        &self,
        scope: &Scope,
        journal: &Journal,
        approved: bool,
        cancelled: &AtomicBool,
    ) -> Outcome {
        let mut outcome = Outcome {
            id: self.id,
            completed: Vec::new(),
            created: Vec::new(),
            incomplete: vec![self.original.path.clone()],
            error: None,
            cancelled: false,
            ..Default::default()
        };
        if !approved || cancelled.load(Ordering::Acquire) {
            outcome.cancelled = cancelled.load(Ordering::Acquire);
            outcome.error = Some(
                "Explicit exact-plan approval is required and cancellation must be clear.".into(),
            );
            return outcome;
        }
        let snapshot = format!(
            "operation={}\nid={}\nsource={}\nidentity={:?}\ntarget={}\nreplacement_bytes={}\n",
            self.label,
            self.id,
            crate::journal::encode_path(self.source()),
            self.original.identity,
            crate::journal::encode_path(&self.stage),
            self.bytes.len()
        );
        let record = match journal.claim_snapshot(
            &self.key,
            &snapshot,
            &[self.original.path.clone(), self.stage.clone()],
        ) {
            Ok(record) => record,
            Err(error) => {
                outcome.error = Some(error);
                return outcome;
            }
        };
        let result = (|| {
            let source = self.original.validate(scope)?;
            if fs::metadata(self.source())
                .map_err(|error| format!("Inspect Save source: {error}"))?
                .permissions()
                .readonly()
            {
                return Err("Approved source is now read-only.".into());
            }
            let parent = scope.resolve(self.stage.parent().ok_or("Staging parent is missing.")?)?;
            if parent
                != self
                    .original
                    .path
                    .parent()
                    .ok_or("Source parent is missing.")?
            {
                return Err("Staging folder no longer binds to source folder.".into());
            }
            let mut stage = secure_stage(self.source(), &self.stage)?;
            outcome.created.push(self.stage.clone());
            for chunk in self.bytes.chunks(64 * 1024) {
                if cancelled.load(Ordering::Acquire) {
                    return Err("Save cancelled before replacement; staging file retained.".into());
                }
                stage
                    .write_all(chunk)
                    .map_err(|error| format!("Write staged source: {error}"))?;
            }
            stage
                .sync_all()
                .map_err(|error| format!("Flush staged source: {error}"))?;
            drop(stage);
            let mut staged_bytes = Vec::new();
            File::open(&self.stage)
                .and_then(|file| {
                    file.take(self.bytes.len() as u64 + 1)
                        .read_to_end(&mut staged_bytes)
                })
                .map_err(|error| format!("Verify staged source: {error}"))?;
            if staged_bytes != self.bytes {
                return Err("Staged source verification failed; no replacement performed.".into());
            }
            if cancelled.load(Ordering::Acquire) {
                return Err("Save cancelled before replacement; staging file retained.".into());
            }
            drop(source);
            if files::fingerprint(self.source())? != self.original.identity {
                return Err(
                    "Source changed immediately before replacement; staging file retained.".into(),
                );
            }
            replace(self.source(), &self.stage)?;
            outcome.created.clear();
            let saved = Snapshot::read(scope, self.source(), self.bytes.len())?;
            if saved.bytes != self.bytes {
                return Err("Replacement read-back differs; outcome is uncertain, inspect source before retrying.".into());
            }
            outcome.completed.push(self.original.path.clone());
            outcome.incomplete.clear();
            Ok(())
        })();
        outcome.cancelled = cancelled.load(Ordering::Acquire) && outcome.completed.is_empty();
        if let Err(error) = result {
            outcome.error = Some(error);
        }
        Journal::finish(record, outcome)
    }
}

pub(crate) fn identity() -> FileResult<String> {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| format!("Create change identity: {error}"))?
        .as_nanos();
    Ok(format!(
        "{time}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(windows)]
pub(crate) fn secure_stage(security_source: &Path, stage: &Path) -> FileResult<File> {
    use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
    use windows::Win32::{
        Foundation::{ERROR_INSUFFICIENT_BUFFER, GENERIC_READ, GENERIC_WRITE, GetLastError},
        Security::{
            DACL_SECURITY_INFORMATION, GetFileSecurityW, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
            SECURITY_ATTRIBUTES, SetSecurityDescriptorControl,
        },
        Storage::FileSystem::{CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ},
    };
    let source: Vec<u16> = security_source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let stage: Vec<u16> = stage.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut needed = 0;
    let result = unsafe {
        GetFileSecurityW(
            windows_core::PCWSTR(source.as_ptr()),
            DACL_SECURITY_INFORMATION.0,
            None,
            0,
            &mut needed,
        )
    };
    let error = unsafe { GetLastError() };
    if result.as_bool() || error != ERROR_INSUFFICIENT_BUFFER || needed == 0 || needed > 65_536 {
        return Err(format!(
            "Cannot bind source ACL for private staging: {error:?}; {needed} bytes."
        ));
    }
    let mut descriptor = vec![0_u32; (needed as usize).div_ceil(4)];
    let pointer = PSECURITY_DESCRIPTOR(descriptor.as_mut_ptr().cast());
    if !unsafe {
        GetFileSecurityW(
            windows_core::PCWSTR(source.as_ptr()),
            DACL_SECURITY_INFORMATION.0,
            Some(pointer),
            needed,
            &mut needed,
        )
    }
    .as_bool()
    {
        return Err(format!(
            "Read staging ACL: {}",
            windows_core::Error::from_thread()
        ));
    }
    // Keep parent inheritance from widening the source's effective access.
    unsafe { SetSecurityDescriptorControl(pointer, SE_DACL_PROTECTED, SE_DACL_PROTECTED) }
        .map_err(|error| format!("Protect staging ACL: {error}"))?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: pointer.0,
        bInheritHandle: false.into(),
    };
    let handle = unsafe {
        CreateFileW(
            windows_core::PCWSTR(stage.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ,
            Some(&attributes),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|error| format!("Create staging file with bound ACL/no overwrite: {error}"))?;
    Ok(unsafe { File::from_raw_handle(handle.0) })
}

#[cfg(not(windows))]
pub(crate) fn secure_stage(_security_source: &Path, _stage: &Path) -> FileResult<File> {
    Err("Secure staging currently requires Windows.".into())
}

#[cfg(windows)]
pub(crate) fn replace(target: &Path, stage: &Path) -> FileResult<()> {
    use std::os::windows::ffi::OsStrExt;
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let stage: Vec<u16> = stage.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        windows::Win32::Storage::FileSystem::ReplaceFileW(
            windows_core::PCWSTR(target.as_ptr()),
            windows_core::PCWSTR(stage.as_ptr()),
            windows_core::PCWSTR::null(),
            windows::Win32::Storage::FileSystem::REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    }
    .map_err(|error| {
        format!(
            "Atomic replacement failed; inspect exact source/stage paths before retrying: {error}"
        )
    })
}

#[cfg(not(windows))]
pub(crate) fn replace(_target: &Path, _stage: &Path) -> FileResult<()> {
    Err("Atomic approved source replacement currently requires Windows.".into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn staging_binds_source_dacl_and_refuses_existing_paths_before_writing() {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Security::{
            DACL_SECURITY_INFORMATION, GetFileSecurityW, PSECURITY_DESCRIPTOR,
        };
        fn descriptor(path: &Path) -> Vec<u8> {
            let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut needed = 0;
            assert!(
                !unsafe {
                    GetFileSecurityW(
                        windows_core::PCWSTR(path.as_ptr()),
                        DACL_SECURITY_INFORMATION.0,
                        None,
                        0,
                        &mut needed,
                    )
                }
                .as_bool()
            );
            assert!(needed > 20 && needed <= 65_536);
            let mut memory = vec![0_u32; (needed as usize).div_ceil(4)];
            assert!(
                unsafe {
                    GetFileSecurityW(
                        windows_core::PCWSTR(path.as_ptr()),
                        DACL_SECURITY_INFORMATION.0,
                        Some(PSECURITY_DESCRIPTOR(memory.as_mut_ptr().cast())),
                        needed,
                        &mut needed,
                    )
                }
                .as_bool()
            );
            unsafe { std::slice::from_raw_parts(memory.as_ptr().cast::<u8>(), needed as usize) }
                .to_vec()
        }
        fn dacl(bytes: &[u8]) -> &[u8] {
            let offset = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
            assert!(offset >= 20);
            let size =
                u16::from_le_bytes(bytes[offset + 2..offset + 4].try_into().unwrap()) as usize;
            &bytes[offset..offset + size]
        }
        let root = std::env::temp_dir().join(format!("tc-stage-acl-{}", identity().unwrap()));
        fs::create_dir(&root).unwrap();
        let source = root.join(".env");
        let stage = root.join("owned.stage");
        fs::write(&source, b"synthetic fixture only").unwrap();
        let before = descriptor(&source);
        drop(secure_stage(&source, &stage).unwrap());
        let staged = descriptor(&stage);
        assert_eq!(dacl(&before), dacl(&staged));
        assert_ne!(
            u16::from_le_bytes(staged[2..4].try_into().unwrap()) & 0x1000,
            0
        );
        assert_eq!(descriptor(&source), before);
        assert!(fs::read(&stage).unwrap().is_empty());
        assert!(secure_stage(&source, &stage).is_err());
        assert!(fs::read(&stage).unwrap().is_empty());
        fs::remove_file(stage).unwrap();
        fs::remove_file(source).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn editing_refuses_unsupported_encoding_readonly_invalid_json_and_precancel() {
        let root = std::env::temp_dir().join(format!("tc-edit-guards-{}", identity().unwrap()));
        fs::create_dir(&root).unwrap();
        let path = root.join("source.txt");
        let scope = Scope::fixture(&root).unwrap();
        for bytes in [b"a\nb\r\n".as_slice(), b"legacy\r", b"nul\0", &[0xff]] {
            fs::write(&path, bytes).unwrap();
            assert!(Editor::open(&scope, &path).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::write(&path, b"original\n").unwrap();
        let original_permissions = fs::metadata(&path).unwrap().permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions.clone()).unwrap();
        assert!(Editor::open(&scope, &path).is_err());
        fs::set_permissions(&path, original_permissions).unwrap();
        let mut editor = Editor::open(&scope, &path).unwrap();
        editor.text = "modified\n".into();
        let plan = editor.plan(1).unwrap();
        let cancelled = AtomicBool::new(true);
        assert!(
            plan.execute(
                &scope,
                &Journal::new(root.join("journal")),
                true,
                &cancelled
            )
            .cancelled
        );
        assert_eq!(fs::read(&path).unwrap(), b"original\n");
        assert!(!plan.stage.exists());
        let json = root.join("source.json");
        fs::write(&json, b"{\"k\":1}").unwrap();
        let mut editor = Editor::open(&scope, &json).unwrap();
        editor.text = "{invalid".into();
        assert!(editor.plan(2).is_err());
        assert_eq!(fs::read(&json).unwrap(), b"{\"k\":1}");
        editor.text = "x".repeat(16_385);
        assert!(editor.plan(3).is_err());
        editor.text = "\n".repeat(20_001);
        assert!(editor.plan(4).is_err());
        fs::write(&json, b"\xef\xbb\xbf{\"k\":1}").unwrap();
        let mut editor = Editor::open(&scope, &json).unwrap();
        let padding = format!("{}\n", " ".repeat(16_000)).repeat(65);
        editor.text = format!(
            "{padding}{}\"a\"",
            " ".repeat(TEXT_LIMIT - padding.len() - 3)
        );
        assert_eq!(editor.text.len(), TEXT_LIMIT);
        assert!(editor.plan(5).err().unwrap().contains("Encoded Save"));
        fs::remove_file(path).unwrap();
        fs::remove_file(json).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn exact_save_preserves_bom_crlf_refuses_conflict_replay_and_unapproved_effects() {
        let root = std::env::temp_dir().join(format!("tc-edit-{}", identity().unwrap()));
        fs::create_dir(&root).unwrap();
        let path = root.join("Příliš.md");
        let original = b"\xef\xbb\xbf# Original\r\ntext\r\n";
        fs::write(&path, original).unwrap();
        let scope = Scope::fixture(&root).unwrap();
        let journal = Journal::new(root.join("journal"));
        let mut editor = Editor::open(&scope, &path).unwrap();
        assert!(!editor.dirty());
        editor.text = "# Changed\ntext\n".into();
        let plan = editor.plan(1).unwrap();
        let cancel = AtomicBool::new(false);
        assert!(
            plan.execute(&scope, &journal, false, &cancel)
                .error
                .is_some()
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        let outcome = plan.execute(&scope, &journal, true, &cancel);
        assert!(outcome.error.is_none(), "{outcome:?}");
        assert_eq!(
            fs::read(&path).unwrap(),
            b"\xef\xbb\xbf# Changed\r\ntext\r\n"
        );
        assert!(
            plan.execute(&scope, &journal, true, &cancel)
                .error
                .unwrap()
                .contains("Claim operation once")
        );
        let stale = editor.plan(2).unwrap();
        assert!(
            stale
                .execute(&scope, &journal, true, &cancel)
                .error
                .unwrap()
                .contains("Source changed")
        );
        assert!(journal.pending().unwrap().is_empty());
        fs::remove_file(&path).unwrap();
        for entry in fs::read_dir(root.join("journal")).unwrap() {
            fs::remove_file(entry.unwrap().path()).unwrap();
        }
        fs::remove_dir(root.join("journal")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}

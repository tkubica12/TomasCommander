use crate::files::FileResult;
use std::{path::Path, process::Command};

pub fn show_error(message: &str) {
    eprintln!("TomasCommander: {message}");
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
                w!("TomasCommander startup error"),
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
            path.display()
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
    use windows::{Win32::Storage::FileSystem::MoveFileW, core::PCWSTR};
    let source_wide = wide(source)?;
    let target_wide = wide(target)?;
    unsafe { MoveFileW(PCWSTR(source_wide.as_ptr()), PCWSTR(target_wide.as_ptr())) }.map_err(|e| {
        format!(
            "Move {} -> {}: {e}. Cross-volume directory moves are not supported yet.",
            source.display(),
            target.display()
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
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(value.as_ptr()), None)
            .map_err(|e| format!("Resolve recycle target {}: {e}", path.display()))?;
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
            .map_err(|e| format!("Recycle {}: {e}", path.display()))?;
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
        .map_err(|e| format!("Launch VS Code for {}: {e}", path.display()))?;
    Ok(())
}

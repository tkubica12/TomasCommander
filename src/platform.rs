use crate::files::FileResult;
use std::{path::Path, process::Command};

#[cfg(windows)]
pub mod external_drag {
    use super::*;
    use std::{cell::RefCell, path::PathBuf, rc::Rc};
    use windows::{
        Win32::{
            Foundation::{
                DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HWND, POINT,
                RECT, S_OK,
            },
            Graphics::Gdi::ScreenToClient,
            System::{
                Com::{CoTaskMemFree, IDataObject},
                Ole::{
                    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropSource, IDropSource_Impl,
                    MK_ALT, OleInitialize, OleUninitialize,
                },
                SystemServices::{MK_LBUTTON, MK_MBUTTON, MK_RBUTTON, MODIFIERKEYS_FLAGS},
            },
            UI::{
                Input::KeyboardAndMouse::{GetAsyncKeyState, GetCapture, VK_LBUTTON},
                Shell::{
                    BHID_DataObject, Common::ITEMIDLIST, SHCreateShellItemArrayFromIDLists,
                    SHDoDragDrop, SHParseDisplayName,
                },
                WindowsAndMessaging::{
                    GA_ROOT, GetAncestor, GetClientRect, GetCursorPos, GetWindowThreadProcessId,
                    IsWindow, IsWindowVisible, SW_HIDE, SW_SHOWNA, ShowWindow,
                },
            },
        },
        core::{BOOL, HRESULT, PCWSTR, implement},
    };

    pub fn pointer_outside_source() -> FileResult<bool> {
        let window = unsafe { GetCapture() };
        if window.is_invalid() || unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) } >= 0 {
            return Ok(false);
        }
        let mut cursor = POINT::default();
        let mut bounds = RECT::default();
        unsafe {
            GetCursorPos(&mut cursor).map_err(|e| format!("Read drag cursor: {e}"))?;
            GetClientRect(window, &mut bounds).map_err(|e| format!("Read drag window: {e}"))?;
            ScreenToClient(window, &mut cursor)
                .ok()
                .map_err(|e| format!("Convert drag cursor: {e}"))?;
        }
        Ok(outside(cursor, bounds))
    }

    fn outside(cursor: POINT, bounds: RECT) -> bool {
        cursor.x < bounds.left
            || cursor.y < bounds.top
            || cursor.x >= bounds.right
            || cursor.y >= bounds.bottom
    }

    struct OleApartment;

    impl OleApartment {
        fn new() -> FileResult<Self> {
            unsafe { OleInitialize(None) }.map_err(|e| format!("Initialize file drag: {e}"))?;
            Ok(Self)
        }
    }

    impl Drop for OleApartment {
        fn drop(&mut self) {
            unsafe { OleUninitialize() };
        }
    }

    struct ItemId(*mut ITEMIDLIST);

    impl Drop for ItemId {
        fn drop(&mut self) {
            unsafe { CoTaskMemFree(Some(self.0.cast())) };
        }
    }

    fn data_object(paths: &[PathBuf]) -> FileResult<IDataObject> {
        if paths.is_empty() {
            return Err("No files were selected for external dragging.".into());
        }
        let mut items = Vec::with_capacity(paths.len());
        let mut parent = None;
        for path in paths {
            if !path.is_absolute() {
                return Err("External dragging requires absolute file paths.".into());
            }
            let metadata = std::fs::symlink_metadata(path).map_err(|e| {
                format!(
                    "Read dragged file {}: {e}",
                    crate::files::display_path(path)
                )
            })?;
            if metadata.file_type().is_symlink()
                || std::os::windows::fs::MetadataExt::file_attributes(&metadata) & 0x400 != 0
                    && metadata.file_type().is_dir()
            {
                return Err(
                    "External dragging of links or directory junctions is unsupported.".into(),
                );
            }
            if parent.is_some_and(|parent| path.parent() != Some(parent)) {
                return Err("Externally dragged files must belong to the same folder.".into());
            }
            parent = path.parent();
            let name = wide(&shell_path(path)?)?;
            let mut item = ItemId(std::ptr::null_mut());
            unsafe { SHParseDisplayName(PCWSTR(name.as_ptr()), None, &mut item.0, 0, None) }
                .map_err(|e| {
                    format!(
                        "Prepare dragged file {}: {e}",
                        crate::files::display_path(path)
                    )
                })?;
            items.push(item);
        }
        let ids: Vec<_> = items.iter().map(|item| item.0.cast_const()).collect();
        let array = unsafe { SHCreateShellItemArrayFromIDLists(&ids) }
            .map_err(|e| format!("Prepare dragged file selection: {e}"))?;
        unsafe { array.BindToHandler(None, &BHID_DataObject) }
            .map_err(|e| format!("Prepare Windows file drag data: {e}"))
    }

    struct HiddenSource(HWND);

    impl HiddenSource {
        fn hide(window: HWND) -> FileResult<Self> {
            let mut process = 0;
            unsafe { GetWindowThreadProcessId(window, Some(&mut process)) };
            if window.is_invalid()
                || process != std::process::id()
                || !unsafe { IsWindowVisible(window) }.as_bool()
            {
                return Err(
                    "Could not identify the Commander window for external dragging.".into(),
                );
            }
            let guard = Self(window);
            let _ = unsafe { ShowWindow(window, SW_HIDE) };
            if unsafe { IsWindowVisible(window) }.as_bool() {
                return Err("Could not hide Commander for external dragging.".into());
            }
            Ok(guard)
        }
    }

    impl Drop for HiddenSource {
        fn drop(&mut self) {
            if unsafe { IsWindow(Some(self.0)) }.as_bool() {
                // Restore the existing maximized/normal state without stealing target focus.
                let _ = unsafe { ShowWindow(self.0, SW_SHOWNA) };
            }
        }
    }

    struct DragSourceState {
        window: HWND,
        hidden: RefCell<Option<HiddenSource>>,
        error: RefCell<Option<String>>,
    }

    impl DragSourceState {
        fn new(window: HWND) -> Self {
            Self {
                window,
                hidden: RefCell::new(None),
                error: RefCell::new(None),
            }
        }

        fn reveal(&self) -> FileResult<()> {
            if self.hidden.borrow().is_none() {
                let hidden = HiddenSource::hide(self.window)?;
                *self.hidden.borrow_mut() = Some(hidden);
            }
            Ok(())
        }

        fn restore(&self) {
            let hidden = self.hidden.borrow_mut().take();
            drop(hidden);
        }
    }

    #[implement(IDropSource)]
    struct FileDragSource {
        state: Rc<DragSourceState>,
    }

    #[allow(non_snake_case)]
    impl IDropSource_Impl for FileDragSource_Impl {
        fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
            if escape.as_bool()
                || self.state.error.borrow().is_some()
                || keys.0 & (MK_RBUTTON.0 | MK_MBUTTON.0) != 0
            {
                return DRAGDROP_S_CANCEL;
            }
            if keys.0 & MK_LBUTTON.0 == 0 {
                return DRAGDROP_S_DROP;
            }
            // OLE owns input after the pointer leaves Commander.
            if keys.0 & MK_ALT != 0
                && let Err(error) = self.state.reveal()
            {
                *self.state.error.borrow_mut() = Some(error);
                return DRAGDROP_S_CANCEL;
            }
            S_OK
        }

        fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    pub fn copy_files(paths: &[PathBuf], reveal_target: bool) -> FileResult<bool> {
        let _apartment = OleApartment::new()?;
        let data = data_object(paths)?;
        let window = unsafe { GetAncestor(GetCapture(), GA_ROOT) };
        let state = Rc::new(DragSourceState::new(window));
        if reveal_target {
            state.reveal()?;
        }
        let source: IDropSource = FileDragSource {
            state: state.clone(),
        }
        .into();
        // OLE owns the mouse loop until drop or Escape; never offer a source move.
        let result = unsafe { SHDoDragDrop(None, &data, &source, DROPEFFECT_COPY) };
        state.restore();
        if let Some(error) = state.error.borrow_mut().take() {
            return Err(error);
        }
        let effect = result.map_err(|e| format!("External file drag failed: {e}"))?;
        if effect != DROPEFFECT_COPY && effect != DROPEFFECT_NONE {
            return Err(
                "External drop returned an unsupported effect; source files were not changed."
                    .into(),
            );
        }
        Ok(effect == DROPEFFECT_COPY)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::{
            System::{
                Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL},
                Ole::ReleaseStgMedium,
            },
            UI::{
                Shell::{DragQueryFileW, HDROP},
                WindowsAndMessaging::{
                    CreateWindowExW, DestroyWindow, GetForegroundWindow, GetWindowRect,
                    WS_EX_NOACTIVATE, WS_POPUP,
                },
            },
        };

        struct OwnedWindow(HWND);

        impl OwnedWindow {
            fn new() -> Self {
                let window = unsafe {
                    CreateWindowExW(
                        WS_EX_NOACTIVATE,
                        windows::core::w!("STATIC"),
                        windows::core::w!("Commander owned drag restoration test"),
                        WS_POPUP,
                        0,
                        0,
                        1,
                        1,
                        None,
                        None,
                        None,
                        None,
                    )
                }
                .unwrap();
                let _ = unsafe { ShowWindow(window, SW_SHOWNA) };
                Self(window)
            }
        }

        impl Drop for OwnedWindow {
            fn drop(&mut self) {
                unsafe { DestroyWindow(self.0) }.unwrap();
            }
        }

        #[test]
        fn native_drag_boundary_uses_client_pixels_including_negative_coordinates() {
            let rect = RECT {
                left: 0,
                top: 0,
                right: 1240,
                bottom: 800,
            };
            for (point, expected) in [
                ((0, 0), false),
                ((1239, 799), false),
                ((-1, 300), true),
                ((200, -1), true),
                ((1240, 300), true),
                ((200, 800), true),
            ] {
                assert_eq!(
                    outside(
                        POINT {
                            x: point.0,
                            y: point.1
                        },
                        rect
                    ),
                    expected
                );
            }
        }

        #[test]
        fn shell_drag_data_contains_exact_unicode_file_paths_and_preserves_sources() {
            let _apartment = OleApartment::new().unwrap();
            let root = std::env::temp_dir().join(format!("tc-drag-{}", std::process::id()));
            std::fs::create_dir(&root).unwrap();
            let paths = vec![
                root.join("presentation intro.txt"),
                root.join("příloha.txt"),
            ];
            for path in &paths {
                std::fs::write(path, b"Synthetic drag fixture").unwrap();
            }
            let data = data_object(&paths).unwrap();
            let format = FORMATETC {
                cfFormat: 15,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            };
            let mut medium = unsafe { data.GetData(&format) }.unwrap();
            let hdrop = HDROP(unsafe { medium.u.hGlobal }.0);
            let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
            let actual: Vec<_> = (0..count)
                .map(|index| {
                    let length = unsafe { DragQueryFileW(hdrop, index, None) };
                    let mut value = vec![0; length as usize + 1];
                    assert_eq!(
                        unsafe { DragQueryFileW(hdrop, index, Some(&mut value)) },
                        length
                    );
                    PathBuf::from(String::from_utf16(&value[..length as usize]).unwrap())
                })
                .collect();
            unsafe { ReleaseStgMedium(&mut medium) };
            assert_eq!(actual, paths);
            drop(data);
            for path in &paths {
                assert_eq!(std::fs::read(path).unwrap(), b"Synthetic drag fixture");
                std::fs::remove_file(path).unwrap();
            }
            std::fs::remove_dir(root).unwrap();
        }

        #[test]
        fn zip_export_shell_data_contains_every_selected_real_member() {
            use std::io::Write;
            let _apartment = OleApartment::new().unwrap();
            let root = std::env::temp_dir().join(format!(
                "tc-zip-shell-{}",
                crate::changes::identity().unwrap()
            ));
            std::fs::create_dir(&root).unwrap();
            let archive = root.join("files.zip");
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for name in ["nested/first.txt", "second.txt"] {
                writer
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(name.as_bytes()).unwrap();
            }
            std::fs::write(&archive, writer.finish().unwrap().into_inner()).unwrap();
            let before = std::fs::read(&archive).unwrap();
            let export = crate::archive_export::prepare(
                &crate::files::Scope::fixture(&root).unwrap(),
                &archive,
                &["nested/first.txt".into(), "second.txt".into()],
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
            let data = data_object(&export.paths).unwrap();
            let format = FORMATETC {
                cfFormat: 15,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            };
            let mut medium = unsafe { data.GetData(&format) }.unwrap();
            let hdrop = HDROP(unsafe { medium.u.hGlobal }.0);
            assert_eq!(unsafe { DragQueryFileW(hdrop, u32::MAX, None) }, 2);
            for (index, expected) in export.paths.iter().enumerate() {
                let length = unsafe { DragQueryFileW(hdrop, index as u32, None) };
                let mut value = vec![0; length as usize + 1];
                unsafe { DragQueryFileW(hdrop, index as u32, Some(&mut value)) };
                assert_eq!(
                    std::fs::canonicalize(PathBuf::from(
                        String::from_utf16(&value[..length as usize]).unwrap()
                    ))
                    .unwrap(),
                    *expected
                );
            }
            unsafe { ReleaseStgMedium(&mut medium) };
            drop(data);
            assert_eq!(std::fs::read(&archive).unwrap(), before);
            export.cleanup().unwrap();
            std::fs::remove_file(archive).unwrap();
            std::fs::remove_dir(root).unwrap();
        }

        #[test]
        fn invalid_external_sources_are_explicit_errors() {
            let _apartment = OleApartment::new().unwrap();
            assert!(data_object(&[]).is_err());
            assert!(data_object(&[PathBuf::from("relative.txt")]).is_err());
        }

        #[test]
        fn external_drag_restores_its_owned_window_on_cancel_and_failure() {
            let owned = OwnedWindow::new();
            let window = owned.0;
            let foreground = unsafe { GetForegroundWindow() };
            let mut before = RECT::default();
            unsafe { GetWindowRect(window, &mut before) }.unwrap();
            for fail in [false, true] {
                let result = (|| -> FileResult<bool> {
                    let _hidden = HiddenSource::hide(window)?;
                    assert!(!unsafe { IsWindowVisible(window) }.as_bool());
                    if fail {
                        Err("Synthetic failed drop".into())
                    } else {
                        Ok(false)
                    }
                })();
                assert_eq!(result.is_err(), fail);
                assert!(unsafe { IsWindowVisible(window) }.as_bool());
                let mut after = RECT::default();
                unsafe { GetWindowRect(window, &mut after) }.unwrap();
                assert_eq!(
                    (before.left, before.top, before.right, before.bottom),
                    (after.left, after.top, after.right, after.bottom)
                );
                assert_eq!(unsafe { GetForegroundWindow() }, foreground);
            }
            assert!(HiddenSource::hide(HWND::default()).is_err());
        }

        #[test]
        fn native_drag_accepts_late_alt_and_restores_after_drop_or_escape() {
            let owned = OwnedWindow::new();
            let window = owned.0;
            for escape in [false, true] {
                let state = Rc::new(DragSourceState::new(window));
                let source: IDropSource = FileDragSource {
                    state: state.clone(),
                }
                .into();
                assert_eq!(unsafe { source.QueryContinueDrag(false, MK_LBUTTON) }, S_OK);
                assert!(unsafe { IsWindowVisible(window) }.as_bool());
                for keys in [
                    MODIFIERKEYS_FLAGS(MK_LBUTTON.0 | MK_ALT),
                    MODIFIERKEYS_FLAGS(MK_LBUTTON.0 | MK_ALT),
                    MK_LBUTTON,
                ] {
                    assert_eq!(unsafe { source.QueryContinueDrag(false, keys) }, S_OK);
                    assert!(!unsafe { IsWindowVisible(window) }.as_bool());
                }
                assert_eq!(
                    unsafe { source.GiveFeedback(DROPEFFECT_COPY) },
                    DRAGDROP_S_USEDEFAULTCURSORS
                );
                assert_eq!(
                    unsafe { source.QueryContinueDrag(escape, MODIFIERKEYS_FLAGS(MK_ALT)) },
                    if escape {
                        DRAGDROP_S_CANCEL
                    } else {
                        DRAGDROP_S_DROP
                    }
                );
                state.restore();
                assert!(unsafe { IsWindowVisible(window) }.as_bool());
                assert!(state.error.borrow().is_none());
            }
        }

        #[test]
        fn native_drag_does_not_hide_on_cancellation_or_button_release() {
            let owned = OwnedWindow::new();
            let state = Rc::new(DragSourceState::new(owned.0));
            let source: IDropSource = FileDragSource {
                state: state.clone(),
            }
            .into();
            for (escape, keys, expected) in [
                (true, MK_LBUTTON.0 | MK_ALT, DRAGDROP_S_CANCEL),
                (false, MK_ALT, DRAGDROP_S_DROP),
                (
                    false,
                    MK_LBUTTON.0 | MK_RBUTTON.0 | MK_ALT,
                    DRAGDROP_S_CANCEL,
                ),
                (
                    false,
                    MK_LBUTTON.0 | MK_MBUTTON.0 | MK_ALT,
                    DRAGDROP_S_CANCEL,
                ),
            ] {
                assert_eq!(
                    unsafe { source.QueryContinueDrag(escape, MODIFIERKEYS_FLAGS(keys)) },
                    expected
                );
                assert!(unsafe { IsWindowVisible(owned.0) }.as_bool());
                assert!(state.hidden.borrow().is_none());
                assert!(state.error.borrow().is_none());
            }
        }

        #[test]
        fn native_drag_cancels_and_reports_a_late_hide_failure() {
            let state = Rc::new(DragSourceState::new(HWND::default()));
            let source: IDropSource = FileDragSource {
                state: state.clone(),
            }
            .into();
            assert_eq!(
                unsafe {
                    source.QueryContinueDrag(false, MODIFIERKEYS_FLAGS(MK_LBUTTON.0 | MK_ALT))
                },
                DRAGDROP_S_CANCEL
            );
            assert!(state.error.borrow().as_deref().unwrap().contains("window"));
            assert_eq!(
                unsafe { source.QueryContinueDrag(false, MK_LBUTTON) },
                DRAGDROP_S_CANCEL
            );
            state.restore();
            assert!(state.hidden.borrow().is_none());
        }
    }
}

pub fn logical_drives() -> FileResult<Vec<std::path::PathBuf>> {
    #[cfg(windows)]
    {
        let mask = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        if mask == 0 {
            return Err(format!(
                "List Windows drives: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok((0..26)
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| std::path::PathBuf::from(format!("{}:\\", char::from(b'A' + bit))))
            .collect())
    }
    #[cfg(not(windows))]
    {
        Err("Drive selection is available only on Windows.".into())
    }
}

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
pub(crate) fn shell_path(path: &Path) -> FileResult<std::path::PathBuf> {
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
pub fn open_file_with(path: &Path) -> FileResult<()> {
    use windows::{
        Win32::{
            Foundation::ERROR_CANCELLED,
            System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            UI::Shell::{OAIF_EXEC, OPENASINFO, SHOpenWithDialog},
        },
        core::PCWSTR,
    };
    let value = wide(&shell_path(path)?)?;
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|error| format!("Initialize Open with: {error}"))?;
    let result = unsafe {
        SHOpenWithDialog(
            None,
            &OPENASINFO {
                pcszFile: PCWSTR(value.as_ptr()),
                pcszClass: PCWSTR::null(),
                oaifInFlags: OAIF_EXEC,
            },
        )
    };
    unsafe { CoUninitialize() };
    match result {
        Err(error) if error.code() == ERROR_CANCELLED.to_hresult() => Ok(()),
        result => result.map_err(|error| format!("Open file with another application: {error}")),
    }
}

#[cfg(not(windows))]
pub fn open_file_with(_path: &Path) -> FileResult<()> {
    Err("The system Open with dialog requires Windows.".into())
}

#[cfg(windows)]
pub fn open_web_url(url: &crate::answers::SafeUrl) -> FileResult<()> {
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::{PCWSTR, w},
    };
    let checked = crate::answers::SafeUrl::inspected(url.as_str())?;
    let value: Vec<u16> = checked.as_str().encode_utf16().chain(Some(0)).collect();
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
    if result.0 as isize <= 32 {
        return Err("Windows could not open the selected inspected web source.".into());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn open_web_url(_url: &crate::answers::SafeUrl) -> FileResult<()> {
    Err("Opening source links requires the Windows desktop.".into())
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
        (*information).FileNameLength =
            u32::try_from(name_bytes).map_err(|_| "Move filename exceeds Windows limits.")?;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*information).FileName).cast::<u16>(),
            name.len(),
        );
        SetFileInformationByHandle(
            HANDLE(source_handle.as_raw_handle()),
            FileRenameInfo,
            information.cast(),
            length,
        )
    }
    .map_err(|e| {
        format!(
            "Move approved handle {} -> {} without replacement: {e}",
            crate::files::display_path(source),
            crate::files::display_path(target)
        )
    })
}

#[cfg(not(windows))]
pub fn move_no_replace(_source: &Path, _target: &Path) -> FileResult<()> {
    Err("Non-overwriting native moves currently require Windows.".into())
}

pub fn recycle(path: &Path) -> FileResult<()> {
    recycle_many(&[path.to_owned()])
}

/// Path text equality ignoring case, using the Windows ordinal casing table.
pub fn equal_ignore_case(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
        let a: Vec<u16> = a.encode_utf16().collect();
        let b: Vec<u16> = b.encode_utf16().collect();
        unsafe { CompareStringOrdinal(&a, &b, true) == CSTR_EQUAL }
    }
    #[cfg(not(windows))]
    {
        a.to_lowercase() == b.to_lowercase()
    }
}

#[cfg(windows)]
struct Apartment;

#[cfg(windows)]
impl Apartment {
    fn new(action: &str) -> FileResult<Self> {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|e| format!("Initialize {action}: {e}"))?;
        Ok(Self)
    }
}

#[cfg(windows)]
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoUninitialize() };
    }
}

#[cfg(windows)]
mod recycle_bin {
    use super::{Apartment, equal_ignore_case, shell_path, wide};
    use crate::{
        files::{FileResult, display_path},
        undo::BinItem,
    };
    use std::{
        ffi::{OsStr, OsString},
        os::windows::ffi::OsStringExt,
        path::{Path, PathBuf},
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use windows::{
        Win32::{
            Foundation::{FILETIME, PROPERTYKEY},
            System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
            UI::Shell::{
                BHID_EnumItems, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FOF_NOERRORUI, FOF_SILENT,
                FOFX_EARLYFAILURE, FOLDERID_RecycleBinFolder, FileOperation, IEnumShellItems,
                IFileOperation, IFileOperationProgressSink, IShellItem, IShellItem2,
                KF_FLAG_DEFAULT, SHCreateItemFromParsingName, SHGetKnownFolderItem,
                SIGDN_FILESYSPATH,
            },
        },
        core::{GUID, Interface, PCWSTR, PWSTR},
    };

    // PSGUID_DISPLACED: the Recycle Bin's original location and deletion time.
    const DISPLACED: GUID = GUID::from_u128(0x9b174b33_40ff_11d2_a27e_00c04fc30871);
    const DELETED_FROM: PROPERTYKEY = PROPERTYKEY {
        fmtid: DISPLACED,
        pid: 2,
    };
    const DATE_DELETED: PROPERTYKEY = PROPERTYKEY {
        fmtid: DISPLACED,
        pid: 3,
    };
    // System.FileName keeps the extension even when Explorer hides extensions.
    const FILE_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x41cf5ae0_f75a_4806_bd87_59c7d9248eb9),
        pid: 100,
    };

    pub(super) fn take(value: windows_core::Result<PWSTR>) -> Option<OsString> {
        let value = value.ok()?;
        if value.is_null() {
            return None;
        }
        let text = OsString::from_wide(unsafe { value.as_wide() });
        unsafe { CoTaskMemFree(Some(value.0.cast())) };
        Some(text)
    }

    fn system_time(time: FILETIME) -> SystemTime {
        const UNIX: u64 = 116_444_736_000_000_000;
        let ticks = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        if ticks >= UNIX {
            UNIX_EPOCH + Duration::from_nanos((ticks - UNIX).saturating_mul(100))
        } else {
            UNIX_EPOCH - Duration::from_nanos((UNIX - ticks).saturating_mul(100))
        }
    }

    /// Visits every Recycle Bin item until `visit` returns true.
    fn enumerate(
        cancel: &AtomicBool,
        mut visit: impl FnMut(IShellItem) -> FileResult<bool>,
    ) -> FileResult<()> {
        let bin: IShellItem =
            unsafe { SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None) }
                .map_err(|e| format!("Open the Recycle Bin: {e}"))?;
        let list: IEnumShellItems = unsafe { bin.BindToHandler(None, &BHID_EnumItems) }
            .map_err(|e| format!("List the Recycle Bin: {e}"))?;
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Recycle Bin search cancelled.".into());
            }
            let mut batch: [Option<IShellItem>; 1] = [None];
            let mut fetched = 0u32;
            unsafe { list.Next(&mut batch, Some(&mut fetched)) }
                .map_err(|e| format!("Read the Recycle Bin: {e}"))?;
            let [Some(item)] = batch else {
                return Ok(());
            };
            if fetched == 0 || visit(item)? {
                return Ok(());
            }
        }
    }

    /// Original location, deletion time and stored item of a bin entry from one of `parents`.
    fn describe(item: &IShellItem, parents: &[String]) -> Option<BinItem> {
        let properties: IShellItem2 = item.cast().ok()?;
        let from = take(unsafe { properties.GetString(&DELETED_FROM) })?;
        if !parents
            .iter()
            .any(|parent| equal_ignore_case(parent, &from.to_string_lossy()))
        {
            return None;
        }
        let name = take(unsafe { properties.GetString(&FILE_NAME) })?;
        let deleted = system_time(unsafe { properties.GetFileTime(&DATE_DELETED) }.ok()?);
        let stored = PathBuf::from(take(unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) })?);
        let metadata = std::fs::symlink_metadata(&stored).ok()?;
        Some(BinItem {
            id: stored.to_string_lossy().into_owned(),
            original: PathBuf::from(from).join(name),
            deleted,
            directory: metadata.is_dir(),
            size: (!metadata.is_dir()).then_some(metadata.len()),
        })
    }

    pub fn items(originals: &[PathBuf], cancel: &AtomicBool) -> FileResult<Vec<BinItem>> {
        if originals.is_empty() {
            return Ok(Vec::new());
        }
        let _apartment = Apartment::new("Recycle Bin search")?;
        let wanted: Vec<String> = originals.iter().map(|path| display_path(path)).collect();
        let parents: Vec<String> = originals
            .iter()
            .filter_map(|path| path.parent())
            .map(display_path)
            .collect();
        let mut found = Vec::new();
        enumerate(cancel, |item| {
            if let Some(entry) = describe(&item, &parents)
                && wanted
                    .iter()
                    .any(|path| equal_ignore_case(path, &display_path(&entry.original)))
            {
                found.push(entry);
            }
            Ok(false)
        })?;
        Ok(found)
    }

    pub fn restore(entry: &BinItem, folder: &Path, name: &OsStr) -> FileResult<()> {
        let _apartment = Apartment::new("Recycle Bin restore")?;
        let parents: Vec<String> = entry
            .original
            .parent()
            .map(display_path)
            .into_iter()
            .collect();
        let mut source = None;
        enumerate(&AtomicBool::new(false), |item| {
            // The stored path is cheaper than the properties; compare it first.
            let stored = take(unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) });
            if !stored.is_some_and(|stored| equal_ignore_case(&stored.to_string_lossy(), &entry.id))
            {
                return Ok(false);
            }
            let unchanged = describe(&item, &parents).is_some_and(|current| {
                equal_ignore_case(
                    &display_path(&current.original),
                    &display_path(&entry.original),
                ) && current.deleted == entry.deleted
                    && current.directory == entry.directory
            });
            if !unchanged {
                return Err(
                    "The Recycle Bin item changed since review; it was not restored.".into(),
                );
            }
            source = Some(item);
            Ok(true)
        })?;
        let source = source.ok_or("Not in Recycle Bin (emptied or restored).")?;
        let folder_name = wide(&shell_path(folder)?)?;
        let new_name = wide(Path::new(name))?;
        unsafe {
            let destination: IShellItem =
                SHCreateItemFromParsingName(PCWSTR(folder_name.as_ptr()), None)
                    .map_err(|e| format!("Resolve restore staging folder: {e}"))?;
            let operation: IFileOperation =
                CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| format!("Create Windows restore operation: {e}"))?;
            // The staging folder is new and empty, so no overwrite prompt can be answered.
            operation
                .SetOperationFlags(
                    FOF_NOCONFIRMATION
                        | FOF_NOERRORUI
                        | FOF_SILENT
                        | FOF_NOCONFIRMMKDIR
                        | FOFX_EARLYFAILURE,
                )
                .map_err(|e| format!("Configure silent restore: {e}"))?;
            let guard: IFileOperationProgressSink =
                super::recycle_guard::Guard::restore(display_path(folder)).into();
            operation
                .MoveItem(&source, &destination, PCWSTR(new_name.as_ptr()), &guard)
                .map_err(|e| format!("Queue restore: {e}"))?;
            operation
                .PerformOperations()
                .map_err(|e| format!("Restore from the Recycle Bin: {e}"))?;
            if operation
                .GetAnyOperationsAborted()
                .map_err(|e| format!("Read restore outcome: {e}"))?
                .as_bool()
            {
                return Err(
                    "Windows aborted the restore; the item stays in the Recycle Bin.".into(),
                );
            }
        }
        Ok(())
    }
}

/// Recycle Bin items whose original path is one of `originals`, with their deletion times.
pub fn recycle_bin_items(
    originals: &[std::path::PathBuf],
    cancel: &std::sync::atomic::AtomicBool,
) -> FileResult<Vec<crate::undo::BinItem>> {
    #[cfg(windows)]
    {
        recycle_bin::items(originals, cancel)
    }
    #[cfg(not(windows))]
    {
        let _ = (originals, cancel);
        Err("Recycle Bin restore currently requires Windows.".into())
    }
}

/// Moves exactly `item` out of the Recycle Bin into the new, empty `folder` as `name`, silently
/// and without replacing anything.
pub fn restore_recycled(
    item: &crate::undo::BinItem,
    folder: &Path,
    name: &std::ffi::OsStr,
) -> FileResult<()> {
    #[cfg(windows)]
    {
        recycle_bin::restore(item, folder, name)
    }
    #[cfg(not(windows))]
    {
        let _ = (item, folder, name);
        Err("Recycle Bin restore currently requires Windows.".into())
    }
}

/// Sends every item to the Recycle Bin in one Windows operation; never deletes permanently.
#[cfg(windows)]
pub fn recycle_many(paths: &[std::path::PathBuf]) -> FileResult<()> {
    use windows::{
        Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        Win32::UI::Shell::{
            FOF_ALLOWUNDO, FOF_NO_CONNECTED_ELEMENTS, FOF_NOCONFIRMATION, FOF_NOERRORUI,
            FOF_SILENT, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FileOperation, IFileOperation,
            IFileOperationProgressSink, IShellItem, SHCreateItemFromParsingName,
        },
        core::PCWSTR,
    };
    let Some(first) = paths.first() else {
        return Ok(());
    };
    let names = paths
        .iter()
        .map(|path| wide(&shell_path(path)?))
        .collect::<FileResult<Vec<_>>>()?;
    let label = if paths.len() == 1 {
        crate::files::display_path(first)
    } else {
        format!(
            "{} items from {}",
            paths.len(),
            crate::files::display_path(first.parent().unwrap_or(first))
        )
    };
    let _apartment = Apartment::new("Windows recycling")?;
    unsafe {
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("Create Windows recycle operation: {e}"))?;
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
        let guard: IFileOperationProgressSink = recycle_guard::Guard::recycle().into();
        for (path, value) in paths.iter().zip(&names) {
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(value.as_ptr()), None)
                .map_err(|e| {
                    format!(
                        "Resolve recycle target {}: {e}",
                        crate::files::display_path(path)
                    )
                })?;
            operation
                .DeleteItem(&item, &guard)
                .map_err(|e| format!("Queue recycle: {e}"))?;
        }
        operation
            .PerformOperations()
            .map_err(|e| format!("Recycle {label}: {e}"))?;
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
                SIGDN_FILESYSPATH, TSF_DELETE_RECYCLE_IF_POSSIBLE,
            },
        },
        core::{HRESULT, PCWSTR, Ref, Result, implement},
    };

    /// Aborts every Shell action except the one it was created for: recycling (never a
    /// permanent delete) or moving a Recycle Bin item into one exact staging folder.
    #[implement(IFileOperationProgressSink)]
    pub struct Guard {
        restore_into: Option<String>,
    }

    impl Guard {
        pub fn recycle() -> Self {
            Self { restore_into: None }
        }

        pub fn restore(folder: String) -> Self {
            Self {
                restore_into: Some(folder),
            }
        }
    }

    #[allow(non_snake_case)]
    impl IFileOperationProgressSink_Impl for Guard_Impl {
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
            destination: Ref<IShellItem>,
            _name: &PCWSTR,
        ) -> Result<()> {
            let Some(folder) = &self.restore_into else {
                return Err(E_ABORT.into());
            };
            let path = super::recycle_bin::take(unsafe {
                destination.ok()?.GetDisplayName(SIGDN_FILESYSPATH)
            });
            match path {
                Some(path) if super::equal_ignore_case(&path.to_string_lossy(), folder) => Ok(()),
                _ => Err(E_ABORT.into()),
            }
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
            if self.restore_into.is_some() || flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0 {
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
            let guard: IFileOperationProgressSink = Guard::recycle().into();
            unsafe {
                assert!(guard.PreDeleteItem(0, None).is_err());
                assert!(
                    guard
                        .PreDeleteItem(TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32, None)
                        .is_ok()
                );
                assert!(guard.PreMoveItem(0, None, None, PCWSTR::null()).is_err());
            }
        }

        #[test]
        fn restore_guard_refuses_deletion_and_moves_elsewhere() {
            let guard: IFileOperationProgressSink =
                Guard::restore(r"C:\owned\.tomas-commander-restore-1".into()).into();
            unsafe {
                for flags in [0, TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32] {
                    assert!(guard.PreDeleteItem(flags, None).is_err());
                }
                assert!(guard.PreMoveItem(0, None, None, PCWSTR::null()).is_err());
                assert!(guard.PreCopyItem(0, None, None, PCWSTR::null()).is_err());
                assert!(guard.PreRenameItem(0, None, PCWSTR::null()).is_err());
            }
        }
    }
}

#[cfg(not(windows))]
pub fn recycle_many(_paths: &[std::path::PathBuf]) -> FileResult<()> {
    Err("Recycle Bin support currently requires Windows; permanent deletion is never used.".into())
}

/// Folded root of the volume holding `path` (`C:\`, a mount folder or `\\server\share\`).
#[cfg(windows)]
pub fn volume_root(path: &Path) -> FileResult<String> {
    use windows::{Win32::Storage::FileSystem::GetVolumePathNameW, core::PCWSTR};
    let value = wide(path)?;
    let mut root = vec![0u16; value.len().max(260) + 1];
    unsafe { GetVolumePathNameW(PCWSTR(value.as_ptr()), &mut root) }
        .map_err(|e| format!("Read volume of {}: {e}", crate::files::display_path(path)))?;
    let length = root
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(root.len());
    let root = std::path::PathBuf::from(String::from_utf16_lossy(&root[..length]));
    Ok(crate::files::display_path(&root).to_lowercase())
}

/// Whether Windows can recycle items from this path's drive. Only fixed local drives have a
/// Recycle Bin; removable, network, optical and RAM drives would delete permanently.
#[cfg(windows)]
pub fn recycle_bin_available(path: &Path) -> FileResult<bool> {
    use windows::{Win32::Storage::FileSystem::GetDriveTypeW, core::PCWSTR};
    const DRIVE_FIXED: u32 = 3;
    let root = volume_root(path)?;
    let value = wide(Path::new(&root))?;
    Ok(unsafe { GetDriveTypeW(PCWSTR(value.as_ptr())) } == DRIVE_FIXED)
}

#[cfg(not(windows))]
pub fn recycle_bin_available(_path: &Path) -> FileResult<bool> {
    Ok(false)
}

#[cfg(all(test, windows))]
mod volume_tests {
    use super::*;

    #[test]
    fn volume_root_and_recycle_probe_agree_for_plain_and_canonical_paths() {
        let temp = std::env::temp_dir();
        let canonical = std::fs::canonicalize(&temp).unwrap();
        let root = volume_root(&temp).unwrap();
        assert_eq!(volume_root(&canonical).unwrap(), root);
        assert!(root.ends_with('\\'), "{root}");
        assert!(
            crate::files::display_path(&canonical)
                .to_lowercase()
                .starts_with(&root),
            "{root}"
        );
        let available = recycle_bin_available(&canonical).unwrap();
        println!("Recycle Bin probe for {root}: {available}");
        assert_eq!(recycle_bin_available(&temp).unwrap(), available);
    }
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
    let executable = find_vscode(candidates)?;
    vscode_command(&executable, path).spawn().map_err(|e| {
        format!(
            "Launch VS Code for {}: {e}",
            crate::files::display_path(path)
        )
    })?;
    Ok(())
}

pub fn find_vscode(
    candidates: impl IntoIterator<Item = std::path::PathBuf>,
) -> FileResult<std::path::PathBuf> {
    candidates
        .into_iter()
        .filter(|path| path.is_absolute())
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "VS Code executable not found in standard locations. Install VS Code using its user or system installer, then restart Tomas Commander. No shell fallback was used.".into()
        })
}

fn vscode_command(executable: &Path, folder: &Path) -> Command {
    let mut command = Command::new(executable);
    command.arg("--new-window").arg(folder);
    command
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

#[cfg(test)]
mod launch_tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn logical_drives_are_exact_sorted_roots_including_the_system_drive() {
        let drives = logical_drives().unwrap();
        assert!(!drives.is_empty());
        assert!(drives.windows(2).all(|pair| pair[0] < pair[1]));
        for drive in &drives {
            let text = drive.to_str().unwrap();
            assert_eq!(text.len(), 3);
            assert!(text.as_bytes()[0].is_ascii_uppercase());
            assert_eq!(&text[1..], ":\\");
            assert!(drive.is_absolute());
            assert!(drive.parent().is_none());
        }
        let system = std::env::var("SystemDrive").unwrap();
        assert!(drives.contains(&std::path::PathBuf::from(format!("{system}\\"))));
    }

    #[test]
    fn vscode_folder_is_one_lossless_argument_not_shell_syntax() {
        let folder = Path::new(r"C:\owned fixtures\Příliš žluťoučký & %PATH%");
        let command = vscode_command(Path::new(r"C:\VS Code\Code.exe"), folder);
        assert_eq!(command.get_program(), r"C:\VS Code\Code.exe");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [std::ffi::OsStr::new("--new-window"), folder.as_os_str()]
        );
    }
}

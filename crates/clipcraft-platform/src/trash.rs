//! Moving a file to the system's Recycle Bin (Trash), so deleting a clip can be undone.
//!
//! Windows uses the shell's delete with undo; Linux asks `gio trash`; macOS asks Finder. The one
//! `unsafe` call is documented where it is made.
#![cfg_attr(windows, allow(unsafe_code))]

use std::path::Path;

/// Moves `path` to the Recycle Bin. The file is never deleted for good: when the system cannot put
/// it in the bin this fails and leaves it where it is.
#[cfg(windows)]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };
    use windows::core::PCWSTR;

    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    // The list of paths ends with two zero units.
    let mut wide: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err("the file name is not valid".to_owned());
    }
    wide.extend([0, 0]);
    let mut operation = SHFILEOPSTRUCTW {
        hwnd: HWND::default(),
        wFunc: FO_DELETE,
        pFrom: PCWSTR(wide.as_ptr()),
        pTo: PCWSTR::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT).0 as u16,
        ..Default::default()
    };
    // SAFETY: `operation` is fully initialised, `pFrom` points to a double-zero-terminated list
    // that outlives the call, and FOF_ALLOWUNDO makes this a move to the Recycle Bin.
    let result = unsafe { SHFileOperationW(&mut operation) };
    if result != 0 || operation.fAnyOperationsAborted.as_bool() {
        return Err(format!(
            "Windows could not move the file to the Recycle Bin ({result:#x})"
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    let status = if cfg!(target_os = "macos") {
        let script = format!(
            "tell application \"Finder\" to delete POSIX file \"{}\"",
            absolute
                .to_string_lossy()
                .replace('\u{5c}', "\u{5c}\u{5c}")
                .replace('"', "\u{5c}\"")
        );
        Command::new("osascript")
            .args(["-e", &script])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    } else {
        Command::new("gio")
            .arg("trash")
            .arg("--")
            .arg(&absolute)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    };
    match status {
        Ok(status) if status.success() => Ok(()),
        _ => Err("this computer has no Trash that ClipCraft can use".to_owned()),
    }
}

//! Per-user Windows shortcuts and Apps & Features registration; no elevation (ADR 0031).
#![cfg_attr(windows, allow(unsafe_code))]
use std::path::Path;

#[cfg(windows)]
const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\ClipCraft";

#[cfg(windows)]
fn read_registry(key: &str, name: &str) -> Result<Option<std::ffi::OsString>, String> {
    use std::os::windows::ffi::OsStringExt;
    use windows::{
        Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW},
        core::HSTRING,
    };
    let key = HSTRING::from(key);
    let name = HSTRING::from(name);
    let mut buffer = [0u8; 8192];
    let mut length = buffer.len() as u32;
    // SAFETY: Constant registry names are NUL-free, the output buffer is writable for length bytes,
    // and the API receives a valid count pointer. Only bounded REG_SZ bytes are decoded below.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut length),
        )
    };
    if status.0 == 2 || status.0 == 3 {
        return Ok(None);
    }
    status.ok().map_err(|e| e.to_string())?;
    let length = usize::try_from(length).map_err(|_| "Invalid registry string length.")?;
    if !(2..=buffer.len()).contains(&length) || length % 2 != 0 {
        return Err("Invalid Windows app registration.".into());
    }
    let mut units: Vec<u16> = buffer[..length]
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    if units.pop() != Some(0) || units.contains(&0) {
        return Err("Invalid Windows app registration string.".into());
    }
    Ok(Some(std::ffi::OsString::from_wide(&units)))
}

#[cfg(windows)]
fn migrate_startup(root: &Path, executable: Option<&Path>) -> Result<(), String> {
    let key = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let Some(current) = read_registry(key, "ClipCraft")? else {
        return Ok(());
    };
    let text = current.to_string_lossy();
    let Some((path, _)) = text.strip_prefix('"').and_then(|text| text.split_once('"')) else {
        return Ok(());
    };
    if !Path::new(path).starts_with(root) {
        return Ok(());
    }
    let full_key = format!("HKCU\\{key}");
    let result = if let Some(executable) = executable {
        reg(&[
            "add".as_ref(),
            full_key.as_ref(),
            "/v".as_ref(),
            "ClipCraft".as_ref(),
            "/t".as_ref(),
            "REG_SZ".as_ref(),
            "/d".as_ref(),
            format!("\"{}\" --minimized", executable.display()).as_ref(),
            "/f".as_ref(),
        ])?
    } else {
        reg(&[
            "delete".as_ref(),
            full_key.as_ref(),
            "/v".as_ref(),
            "ClipCraft".as_ref(),
            "/f".as_ref(),
        ])?
    };
    if result.status.success() {
        Ok(())
    } else {
        Err("Could not update ClipCraft's existing login startup entry.".into())
    }
}

#[cfg(windows)]
fn known_folder(id: &windows::core::GUID) -> Result<std::path::PathBuf, String> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };
    // SAFETY: The OS returns an owned NUL-terminated buffer on success. Convert it before freeing
    // exactly once with the documented allocator, including on a UTF-16 conversion error.
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).map_err(|e| e.to_string())?;
        let converted = path.to_string();
        CoTaskMemFree(Some(path.0.cast()));
        converted
            .map(std::path::PathBuf::from)
            .map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
fn reg(arguments: &[&std::ffi::OsStr]) -> Result<std::process::Output, String> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg.exe")
        .args(arguments)
        .creation_flags(0x08000000)
        .output()
        .map_err(|_| "Windows app registration could not be opened.".into())
}

#[cfg(windows)]
fn value(name: &str, content: &std::ffi::OsStr) -> Result<(), String> {
    let result = reg(&[
        "add".as_ref(),
        KEY.as_ref(),
        "/v".as_ref(),
        name.as_ref(),
        "/t".as_ref(),
        "REG_SZ".as_ref(),
        "/d".as_ref(),
        content,
        "/f".as_ref(),
    ])?;
    if result.status.success() {
        Ok(())
    } else {
        Err("Windows could not register this app for your account.".into())
    }
}

#[cfg(windows)]
fn shortcut(target: &Path, path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize, IPersistFile,
            },
            UI::Shell::{IShellLinkW, ShellLink},
        },
        core::{Interface, PCWSTR},
    };
    fn wide(path: &Path) -> Result<Vec<u16>, String> {
        let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) || !path.is_absolute() {
            return Err("The shortcut path is invalid.".into());
        }
        value.push(0);
        Ok(value)
    }
    let target = wide(target)?;
    let path = wide(path)?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: This guard exists only after successful initialization on this same worker thread.
            unsafe { CoUninitialize() };
        }
    }
    // SAFETY: Initializes COM on the installation worker; every success has one matching guard.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|e| e.to_string())?;
    let _apartment = Apartment;
    // SAFETY: ShellLink is a documented in-process COM class; typed interface owns its reference.
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .map_err(|e| e.to_string())?;
    // SAFETY: Both strings are absolute, NUL-terminated buffers that outlive these synchronous calls.
    unsafe {
        link.SetPath(PCWSTR(target.as_ptr()))
            .map_err(|e| e.to_string())?;
        let persist: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        persist
            .Save(PCWSTR(path.as_ptr()), true)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Creates native Start menu/optional desktop links and a per-user uninstall entry.
#[cfg(windows)]
pub fn register_installation(
    root: &Path,
    executable: &Path,
    uninstaller: &Path,
    desktop: bool,
) -> Result<(), String> {
    if !root.is_absolute() || !executable.starts_with(root) || !uninstaller.starts_with(root) {
        return Err("Invalid application installation paths.".into());
    }
    let programs = known_folder(&windows::Win32::UI::Shell::FOLDERID_Programs)?;
    std::fs::create_dir_all(&programs).map_err(|e| e.to_string())?;
    shortcut(executable, &programs.join("ClipCraft.lnk"))?;
    shortcut(uninstaller, &programs.join("Uninstall ClipCraft.lnk"))?;
    if desktop {
        let desktop = known_folder(&windows::Win32::UI::Shell::FOLDERID_Desktop)?;
        shortcut(executable, &desktop.join("ClipCraft.lnk"))?;
    }
    value("DisplayName", "ClipCraft".as_ref())?;
    value("DisplayVersion", env!("CARGO_PKG_VERSION").as_ref())?;
    value("Publisher", "ClipCraft contributors".as_ref())?;
    value("InstallLocation", root.as_os_str())?;
    value("DisplayIcon", executable.as_os_str())?;
    migrate_startup(root, Some(executable))?;
    value(
        "UninstallString",
        format!("\"{}\"", uninstaller.display()).as_ref(),
    )
}

/// Removes registration only when it still belongs to this installation folder.
#[cfg(windows)]
pub fn unregister_installation(root: &Path) -> Result<(), String> {
    let location = read_registry(
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\ClipCraft",
        "InstallLocation",
    )?;
    if location.as_deref().map(Path::new) != Some(root) {
        return Ok(());
    }
    migrate_startup(root, None)?;
    let programs = known_folder(&windows::Win32::UI::Shell::FOLDERID_Programs)?;
    let desktop = known_folder(&windows::Win32::UI::Shell::FOLDERID_Desktop)?;
    for path in [
        programs.join("ClipCraft.lnk"),
        programs.join("Uninstall ClipCraft.lnk"),
        desktop.join("ClipCraft.lnk"),
    ] {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    let deleted = reg(&["delete".as_ref(), KEY.as_ref(), "/f".as_ref()])?;
    if deleted.status.success() {
        Ok(())
    } else {
        Err("Could not remove the Windows uninstall entry. Retry from Windows Settings.".into())
    }
}

#[cfg(not(windows))]
pub fn register_installation(
    _root: &Path,
    _executable: &Path,
    _uninstaller: &Path,
    _desktop: bool,
) -> Result<(), String> {
    Err(
        "The native installer supports Windows only; use the portable package on this platform."
            .into(),
    )
}
#[cfg(not(windows))]
pub fn unregister_installation(_root: &Path) -> Result<(), String> {
    Err("Windows app registration is unavailable on this platform.".into())
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn shell_shortcut_handles_spaces_and_unicode_without_a_script() {
        let folder = std::env::temp_dir().join(format!("cut-link-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("ClipCraft test.lnk");
        super::shortcut(&folder.join("Clip Craft 日本語.exe"), &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() > 76);
        assert_eq!(&bytes[..4], &[0x4c, 0, 0, 0]);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
}

//! Launching ClipCraft when the person signs in.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::EngineError;

const LINUX_FILE: &str = "clipcraft.desktop";
const MAC_LABEL: &str = "com.clipcraft.app";

/// The Linux autostart entry for `executable`.
pub fn desktop_entry(executable: &Path) -> String {
    // The Exec value is quoted, with the characters the Desktop Entry spec reserves escaped.
    let mut quoted = String::from("\"");
    for character in executable.to_string_lossy().chars() {
        if matches!(character, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        if character == '%' {
            quoted.push('%');
        }
        if !character.is_control() {
            quoted.push(character);
        }
    }
    quoted.push('"');
    format!(
        "[Desktop Entry]\nType=Application\nName=ClipCraft\nComment=Local clip recorder\nExec={quoted} --minimized\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

/// The macOS launch agent for `executable`.
pub fn launch_agent(executable: &Path) -> String {
    let path = executable
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n<key>Label</key><string>{MAC_LABEL}</string>\n<key>ProgramArguments</key><array><string>{path}</string><string>--minimized</string></array>\n<key>RunAtLoad</key><true/>\n</dict>\n</plist>\n"
    )
}

fn write_or_remove(path: &Path, contents: Option<&str>) -> Result<(), EngineError> {
    match contents {
        Some(contents) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|_| EngineError::StartupFailed)?;
            }
            fs::write(path, contents).map_err(|_| EngineError::StartupFailed)
        }
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(EngineError::StartupFailed),
        },
    }
}

fn home() -> Result<PathBuf, EngineError> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(EngineError::StartupFailed)
}

/// A program in Windows' own `System32` folder, by its full path, so that a file of the same name
/// next to ClipCraft (a portable copy unpacked into a shared folder, say) or earlier on the search
/// path cannot stand in for it.
fn windows_system_tool(name: &str) -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot")
        .filter(|value| !value.is_empty())
        .map_or_else(
            || std::path::PathBuf::from(r"C:\Windows"),
            std::path::PathBuf::from,
        );
    root.join("System32").join(name)
}

/// Turns launching at sign-in on or off for this person.
pub(crate) fn set_launch_at_startup(enabled: bool, executable: &Path) -> Result<(), EngineError> {
    if enabled && !executable.is_absolute() {
        return Err(EngineError::StartupFailed);
    }
    if cfg!(target_os = "windows") {
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let mut command = Command::new(windows_system_tool("reg.exe"));
        if enabled {
            command.args(["add", key, "/v", "ClipCraft", "/t", "REG_SZ", "/f", "/d"]);
            command.arg(format!("\"{}\" --minimized", executable.display()));
        } else {
            command.args(["delete", key, "/v", "ClipCraft", "/f"]);
        }
        let status = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| EngineError::StartupFailed)?;
        // Deleting a value that is not there is already the wanted state.
        return if status.success() || !enabled {
            Ok(())
        } else {
            Err(EngineError::StartupFailed)
        };
    }
    if cfg!(target_os = "macos") {
        let path = home()?
            .join("Library/LaunchAgents")
            .join(format!("{MAC_LABEL}.plist"));
        let contents = launch_agent(executable);
        return write_or_remove(&path, enabled.then_some(contents.as_str()));
    }
    let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        Some(config) => PathBuf::from(config),
        None => home()?.join(".config"),
    };
    let contents = desktop_entry(executable);
    write_or_remove(
        &config.join("autostart").join(LINUX_FILE),
        enabled.then_some(contents.as_str()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_tools_are_named_by_their_full_system_path() {
        let tool = windows_system_tool("reg.exe");
        assert!(tool.ends_with("System32/reg.exe") || tool.ends_with(r"System32\reg.exe"));
        assert!(tool.components().count() > 2, "{tool:?}");
    }

    #[test]
    fn the_desktop_entry_quotes_the_program_and_starts_minimized() {
        let entry = desktop_entry(Path::new("/opt/Clip Craft/clip$craft"));
        assert!(entry.contains("Exec=\"/opt/Clip Craft/clip\\$craft\" --minimized\n"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
    }

    #[test]
    fn the_launch_agent_escapes_markup() {
        let plist = launch_agent(Path::new("/Apps/A&B/clipcraft"));
        assert!(plist.contains("<string>/Apps/A&amp;B/clipcraft</string>"));
        assert!(plist.contains("<key>RunAtLoad</key><true/>"));
    }

    #[test]
    fn a_relative_program_path_is_refused() {
        assert_eq!(
            set_launch_at_startup(true, Path::new("clipcraft")),
            Err(EngineError::StartupFailed)
        );
    }
}

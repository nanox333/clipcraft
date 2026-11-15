//! Putting a file on the system clipboard, so it can be pasted into Discord, a chat or a file
//! manager as a file (not as text).
//!
//! Windows uses the shell's file-drop clipboard format. macOS asks the system's script runner to
//! set the clipboard to a file. Linux has no clipboard of its own to write to without a toolkit,
//! so it uses `wl-copy` (Wayland) or `xclip` (X11) when one is installed, and says so when none
//! is. The one `unsafe` block is documented where it is.
#![cfg_attr(windows, allow(unsafe_code))]

use std::path::Path;

/// Copies `path` to the clipboard as a file. Fails with a message when the system cannot.
#[cfg(windows)]
pub fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{GlobalFree, HANDLE, POINT};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows::Win32::UI::Shell::DROPFILES;
    use windows::core::BOOL;

    /// The clipboard format of a list of files.
    const CF_HDROP: u32 = 15;

    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    // The list of paths ends with two zero units.
    let mut wide: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err("the file name is not valid".to_owned());
    }
    wide.extend([0, 0]);
    let header = std::mem::size_of::<DROPFILES>();
    let size = header + wide.len() * 2;

    // SAFETY: the block is allocated with room for the header and the list, written only while
    // locked and inside those bounds, and unlocked before it is handed over. After a successful
    // `SetClipboardData` the system owns the block; on any earlier failure it is freed here.
    unsafe {
        let block = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size)
            .map_err(|error| format!("could not allocate clipboard memory: {error}"))?;
        let memory = GlobalLock(block).cast::<u8>();
        if memory.is_null() {
            let _ = GlobalFree(Some(block));
            return Err("could not lock clipboard memory".to_owned());
        }
        let drop = DROPFILES {
            pFiles: header as u32,
            pt: POINT { x: 0, y: 0 },
            fNC: BOOL(0),
            fWide: BOOL(1),
        };
        std::ptr::write_unaligned(memory.cast::<DROPFILES>(), drop);
        std::ptr::copy_nonoverlapping(
            wide.as_ptr().cast::<u8>(),
            memory.add(header),
            wide.len() * 2,
        );
        let _ = GlobalUnlock(block);

        // Another program may hold the clipboard for a moment.
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if !opened {
            let _ = GlobalFree(Some(block));
            return Err("another program is using the clipboard".to_owned());
        }
        let result = EmptyClipboard()
            .and_then(|()| SetClipboardData(CF_HDROP, Some(HANDLE(block.0))).map(|_| ()));
        let _ = CloseClipboard();
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = GlobalFree(Some(block));
                Err(format!("Windows would not take the file: {error}"))
            }
        }
    }
}

/// `file:///home/me/a%20clip.mp4` for an absolute path.
#[cfg(not(windows))]
fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                uri.push(char::from(*byte));
            }
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    uri
}

/// Waits a few seconds for a clipboard tool to finish, and ends it if it does not, so a stuck tool
/// can never freeze the window. True when it finished and succeeded.
#[cfg(not(windows))]
fn finished_in_time(child: &mut std::process::Child) -> bool {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if start.elapsed() < std::time::Duration::from_secs(3) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(not(windows))]
pub fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    if cfg!(target_os = "macos") {
        let script = format!(
            "set the clipboard to (POSIX file \"{}\")",
            absolute
                .to_string_lossy()
                .replace('\u{5c}', "\u{5c}\u{5c}")
                .replace('"', "\u{5c}\"")
        );
        let status = Command::new("osascript")
            .args(["-e", &script])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| error.to_string())?;
        return if status.success() {
            Ok(())
        } else {
            Err("the system would not take the file".to_owned())
        };
    }
    let list = format!("{}\r\n", file_uri(&absolute));
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());
    let tools: [(&str, &[&str]); 2] = [
        ("wl-copy", &["--type", "text/uri-list"]),
        (
            "xclip",
            &["-selection", "clipboard", "-t", "text/uri-list", "-i"],
        ),
    ];
    // The tool that matches the session goes first.
    let order: Vec<_> = if wayland {
        tools.to_vec()
    } else {
        tools.iter().rev().copied().collect()
    };
    for (program, arguments) in order {
        let Ok(mut child) = Command::new(program)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut input) = child.stdin.take() {
            let _ = input.write_all(list.as_bytes());
        }
        if finished_in_time(&mut child) {
            return Ok(());
        }
    }
    Err("copying a file needs wl-copy (Wayland) or xclip (X11) to be installed".to_owned())
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn a_tool_that_never_finishes_is_ended_instead_of_waited_for_forever() {
        let Ok(mut child) = std::process::Command::new("sleep").arg("60").spawn() else {
            return;
        };
        let start = std::time::Instant::now();
        assert!(!finished_in_time(&mut child));
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        let mut quick = std::process::Command::new("true").spawn().unwrap();
        assert!(finished_in_time(&mut quick));
    }

    #[test]
    fn a_path_becomes_a_file_uri_with_the_unsafe_bytes_escaped() {
        assert_eq!(
            file_uri(Path::new("/home/me/ClipCraft/Hades 2026 #1.mp4")),
            "file:///home/me/ClipCraft/Hades%202026%20%231.mp4"
        );
        assert_eq!(file_uri(Path::new("/a/é.mp4")), "file:///a/%C3%A9.mp4");
    }
}

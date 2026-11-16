//! A plain error box for the moments nothing else can speak: the window could not be opened, or
//! the program is stopping because of an internal error. ClipCraft is a program without a console
//! on Windows, so without this it would simply disappear.
//!
//! Only Windows shows a box. Elsewhere the text goes to the error output, which is where a program
//! started from a terminal (the usual way on Linux) is read. The one `unsafe` call is documented
//! where it is made.
#![cfg_attr(windows, allow(unsafe_code))]

/// The most characters shown; a message box that fills the screen helps no one.
const MAX_CHARS: usize = 1_500;

/// `text` as the NUL-terminated UTF-16 a Windows call wants: cut to a sensible length, and with
/// no NUL inside (it would end the text early).
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn wide(text: &str) -> Vec<u16> {
    text.chars()
        .take(MAX_CHARS)
        .map(|c| if c == '\0' { ' ' } else { c })
        .collect::<String>()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

/// Tells the person something went wrong, in a box they must close. Returns when it is closed.
#[cfg(windows)]
pub fn show_error_box(title: &str, message: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW,
    };
    use windows::core::PCWSTR;
    let title = wide(title);
    let message = wide(message);
    // SAFETY: both buffers are NUL-terminated UTF-16 that live until the call returns, and with no
    // owner window the call only shows a box and waits for it to be closed.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}

#[cfg(not(windows))]
pub fn show_error_box(title: &str, message: &str) {
    eprintln!("{title}: {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_cut_terminated_and_has_no_nul_inside() {
        let units = wide("a\0b");
        assert_eq!(
            units,
            vec![u16::from(b'a'), u16::from(b' '), u16::from(b'b'), 0]
        );
        let long = wide(&"é".repeat(10_000));
        assert_eq!(long.len(), MAX_CHARS + 1);
        assert_eq!(long.last(), Some(&0));
        assert_eq!(wide(""), vec![0]);
    }

    #[test]
    fn showing_an_error_where_there_is_no_box_does_not_panic() {
        #[cfg(not(windows))]
        show_error_box("ClipCraft", "a test message");
    }
}

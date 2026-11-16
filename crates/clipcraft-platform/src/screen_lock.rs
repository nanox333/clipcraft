//! Whether the screen is locked, so recording can pause while nobody is looking at it.
//!
//! Neither system has one call for this that works for every desktop, so this asks what each
//! offers and says "not locked" when it cannot tell:
//!
//! - Linux: the session manager's `LockedHint` (GNOME, KDE and others set it when they lock).
//! - Windows: the lock screen is a program, `LogonUI.exe`, that runs only while it is showing.
//! - Elsewhere: never locked.

/// True while the screen is locked, as far as the system says.
pub fn screen_locked() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux_locked()
    }
    #[cfg(windows)]
    {
        crate::running_processes(false)
            .iter()
            .any(|process| is_lock_screen(&process.executable))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        false
    }
}

/// What `loginctl show-session -p LockedHint --value` printed, as a yes or no.
#[cfg(any(target_os = "linux", test))]
fn locked_hint(output: &str) -> bool {
    output.trim().eq_ignore_ascii_case("yes")
}

/// The Windows lock screen program's name.
#[cfg(any(windows, test))]
fn is_lock_screen(executable: &str) -> bool {
    executable.trim().eq_ignore_ascii_case("LogonUI.exe")
}

#[cfg(target_os = "linux")]
fn linux_locked() -> bool {
    use std::process::{Command, Stdio};
    let Ok(session) = std::env::var("XDG_SESSION_ID") else {
        return false;
    };
    // A session id is letters and digits; anything else is not passed on.
    if session.is_empty()
        || session.len() > 32
        || !session.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return false;
    }
    Command::new("loginctl")
        .args(["show-session", &session, "-p", "LockedHint", "--value"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| locked_hint(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_managers_answer_is_read_as_yes_or_no() {
        assert!(locked_hint("yes\n"));
        assert!(locked_hint(" YES "));
        assert!(!locked_hint("no\n"));
        assert!(!locked_hint(""));
        assert!(!locked_hint("maybe"));
    }

    #[test]
    fn only_the_lock_screen_program_counts_on_windows() {
        assert!(is_lock_screen("LogonUI.exe"));
        assert!(is_lock_screen("logonui.EXE"));
        assert!(!is_lock_screen("explorer.exe"));
    }
}

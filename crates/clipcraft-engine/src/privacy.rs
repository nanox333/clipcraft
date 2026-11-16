//! Pausing the desktop recording while a private program is open.
//!
//! A password manager or a banking program on the screen should not end up in a clip someone
//! shares. While one has a window open, the recording is paused (nothing is added to the replay);
//! when it is closed, the recording carries on. Only programs ClipCraft knows by name count; a
//! website in a browser tab is not recognized, and the setting says so.

/// Programs whose window pauses the recording, by executable name without `.exe`, lowercase.
const PRIVATE_PROGRAMS: &[&str] = &[
    "1password",
    "bitwarden",
    "dashlane",
    "enpass",
    "keepass",
    "keepassxc",
    "keeper",
    "lastpass",
    "nordpass",
    "protonpass",
    "proton pass",
    "roboform",
    "authy",
    "yubikey-manager",
];

/// True if an executable of this name is a private program.
pub fn is_private_program(executable: &str) -> bool {
    let lower = executable.trim().to_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    PRIVATE_PROGRAMS.contains(&stem)
}

/// What to do with the recording after looking at the open programs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivacyStep {
    /// Pause the recording: a private program is open.
    Pause,
    /// Resume the recording: the private program is closed.
    Resume,
    /// Leave the recording as it is.
    Keep,
}

/// Remembers whether ClipCraft itself paused the recording, so a pause the person chose is never
/// undone, and a resume the person chose is respected while the program stays open.
#[derive(Clone, Debug, Default)]
pub struct PrivacyPause {
    paused_by_us: bool,
    /// The person resumed while a private program was open; leave it alone until it is closed.
    overridden: bool,
}

impl PrivacyPause {
    /// True while ClipCraft is holding the recording paused.
    pub fn holding(&self) -> bool {
        self.paused_by_us
    }

    /// Decides the next step from what is open and whether the recording is paused right now.
    pub fn step(&mut self, private_open: bool, recording_paused: bool) -> PrivacyStep {
        if self.paused_by_us && !recording_paused {
            // The person pressed resume themselves.
            self.paused_by_us = false;
            self.overridden = private_open;
            return PrivacyStep::Keep;
        }
        if !private_open {
            self.overridden = false;
            if self.paused_by_us {
                self.paused_by_us = false;
                return PrivacyStep::Resume;
            }
            return PrivacyStep::Keep;
        }
        if self.overridden || recording_paused {
            return PrivacyStep::Keep;
        }
        self.paused_by_us = true;
        PrivacyStep::Pause
    }

    /// Forgets everything, for when the recording stops.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_managers_are_recognized_whatever_the_case_or_extension() {
        assert!(is_private_program("1Password.exe"));
        assert!(is_private_program("KeePassXC"));
        assert!(is_private_program(" bitwarden.EXE "));
        assert!(!is_private_program("firefox.exe"));
        assert!(!is_private_program(""));
        assert!(!is_private_program("keepassxc-helper"));
    }

    #[test]
    fn the_recording_pauses_while_a_private_program_is_open_and_resumes_after() {
        let mut privacy = PrivacyPause::default();
        assert_eq!(privacy.step(false, false), PrivacyStep::Keep);
        assert_eq!(privacy.step(true, false), PrivacyStep::Pause);
        assert!(privacy.holding());
        assert_eq!(privacy.step(true, true), PrivacyStep::Keep);
        assert_eq!(privacy.step(false, true), PrivacyStep::Resume);
        assert!(!privacy.holding());
    }

    #[test]
    fn a_pause_the_person_chose_is_never_undone() {
        let mut privacy = PrivacyPause::default();
        // Paused by the person before and during the program being open.
        assert_eq!(privacy.step(true, true), PrivacyStep::Keep);
        assert_eq!(privacy.step(false, true), PrivacyStep::Keep);
        assert!(!privacy.holding());
    }

    #[test]
    fn a_resume_the_person_chose_holds_until_the_program_is_closed() {
        let mut privacy = PrivacyPause::default();
        assert_eq!(privacy.step(true, false), PrivacyStep::Pause);
        // They press resume while the program is still open: it is not paused again.
        assert_eq!(privacy.step(true, false), PrivacyStep::Keep);
        assert_eq!(privacy.step(true, false), PrivacyStep::Keep);
        // Once it is closed and opened again, it pauses again.
        assert_eq!(privacy.step(false, false), PrivacyStep::Keep);
        assert_eq!(privacy.step(true, false), PrivacyStep::Pause);
    }
}

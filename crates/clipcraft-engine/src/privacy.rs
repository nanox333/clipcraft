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

/// Most programs the person can add of their own, and the longest name.
pub const MAX_OWN_PROGRAMS: usize = 32;
const MAX_NAME: usize = 64;

/// The name of a program as it is compared: lowercase, no `.exe`, no spaces around it.
fn stem(executable: &str) -> String {
    let lower = executable.trim().to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
}

/// True if an executable of this name is a private program: one on the built-in list, or one the
/// person added.
pub fn is_private_program(executable: &str, own: &[String]) -> bool {
    let stem = stem(executable);
    !stem.is_empty() && (PRIVATE_PROGRAMS.contains(&stem.as_str()) || own.contains(&stem))
}

/// The person's own list made safe to keep: names only (no paths or control characters), lowercase
/// without `.exe`, no duplicates, at most [`MAX_OWN_PROGRAMS`].
pub fn clean_program_list(names: &[String]) -> Vec<String> {
    let mut clean: Vec<String> = Vec::new();
    for name in names {
        let name: String = name
            .chars()
            .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ','))
            .take(MAX_NAME)
            .collect();
        let name = stem(&name);
        if !name.is_empty() && !clean.contains(&name) {
            clean.push(name);
        }
        if clean.len() == MAX_OWN_PROGRAMS {
            break;
        }
    }
    clean
}

/// A list typed as text (commas or new lines between names), as a clean list.
pub fn parse_program_list(text: &str) -> Vec<String> {
    let names: Vec<String> = text.split([',', '\n', ';']).map(str::to_owned).collect();
    clean_program_list(&names)
}

/// A list as it is shown in the box, names separated by commas.
pub fn program_list_text(names: &[String]) -> String {
    names.join(", ")
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
        assert!(is_private_program("1Password.exe", &[]));
        assert!(is_private_program("KeePassXC", &[]));
        assert!(is_private_program(" bitwarden.EXE ", &[]));
        assert!(!is_private_program("firefox.exe", &[]));
        assert!(!is_private_program("", &[]));
        assert!(!is_private_program("keepassxc-helper", &[]));
    }

    #[test]
    fn the_persons_own_programs_count_too_and_the_list_is_made_safe() {
        let own = parse_program_list("MyBank.exe, my bank app; ../evil\\path.exe\n mybank ,,");
        assert_eq!(own, vec!["mybank", "my bank app", "..evilpath"]);
        assert!(is_private_program("MYBANK.EXE", &own));
        assert!(!is_private_program("mybank2.exe", &own));
        // Never more than the limit, however long the text.
        let many: String = (0..100).map(|i| format!("p{i},")).collect();
        assert_eq!(parse_program_list(&many).len(), MAX_OWN_PROGRAMS);
        assert_eq!(program_list_text(&own), "mybank, my bank app, ..evilpath");
        let long = "x".repeat(500);
        assert_eq!(parse_program_list(&long)[0].len(), 64);
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

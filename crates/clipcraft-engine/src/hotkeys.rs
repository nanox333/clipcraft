//! Keyboard shortcuts for recording, as validated text such as `Ctrl+Shift+F8`.

use serde::{Deserialize, Serialize};

/// Something a shortcut can do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotkeyAction {
    /// Save the last stretch of play as a clip.
    SaveClip,
    /// Start or stop a long recording.
    ToggleRecording,
    /// Mute or unmute the microphone.
    ToggleMicrophone,
    /// Hold to talk when push to talk is on.
    PushToTalk,
    /// Save a still picture of the game.
    Screenshot,
}

impl HotkeyAction {
    /// Every action, in the order the settings list them.
    pub const ALL: [HotkeyAction; 5] = [
        Self::SaveClip,
        Self::ToggleRecording,
        Self::ToggleMicrophone,
        Self::PushToTalk,
        Self::Screenshot,
    ];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::SaveClip => "Save clip",
            Self::ToggleRecording => "Start or stop long recording",
            Self::ToggleMicrophone => "Mute or unmute microphone",
            Self::PushToTalk => "Push to talk",
            Self::Screenshot => "Take screenshot",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::SaveClip => "Keeps the last moments of play as a clip.",
            Self::ToggleRecording => "Records until you press it again.",
            Self::ToggleMicrophone => "Switches your microphone on or off in clips.",
            Self::PushToTalk => "Your microphone is only recorded while you hold this key.",
            Self::Screenshot => "Saves the current frame as a picture.",
        }
    }

    /// The shortcut used until the person picks another. Empty means none.
    pub fn default_key(self) -> &'static str {
        match self {
            Self::SaveClip => "F8",
            Self::ToggleRecording => "F9",
            Self::ToggleMicrophone => "F10",
            Self::PushToTalk => "",
            Self::Screenshot => "F11",
        }
    }
}

/// The shortcut for every action. An empty string means the action has none.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Hotkeys {
    /// Save clip.
    pub save_clip: String,
    /// Start or stop long recording.
    pub toggle_recording: String,
    /// Mute or unmute the microphone.
    pub toggle_microphone: String,
    /// Push to talk.
    pub push_to_talk: String,
    /// Take screenshot.
    pub screenshot: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        let mut keys = Self {
            save_clip: String::new(),
            toggle_recording: String::new(),
            toggle_microphone: String::new(),
            push_to_talk: String::new(),
            screenshot: String::new(),
        };
        for action in HotkeyAction::ALL {
            *keys.slot(action) = action.default_key().to_owned();
        }
        keys
    }
}

impl Hotkeys {
    fn slot(&mut self, action: HotkeyAction) -> &mut String {
        match action {
            HotkeyAction::SaveClip => &mut self.save_clip,
            HotkeyAction::ToggleRecording => &mut self.toggle_recording,
            HotkeyAction::ToggleMicrophone => &mut self.toggle_microphone,
            HotkeyAction::PushToTalk => &mut self.push_to_talk,
            HotkeyAction::Screenshot => &mut self.screenshot,
        }
    }

    /// The shortcut for `action`; empty when it has none.
    pub fn get(&self, action: HotkeyAction) -> &str {
        match action {
            HotkeyAction::SaveClip => &self.save_clip,
            HotkeyAction::ToggleRecording => &self.toggle_recording,
            HotkeyAction::ToggleMicrophone => &self.toggle_microphone,
            HotkeyAction::PushToTalk => &self.push_to_talk,
            HotkeyAction::Screenshot => &self.screenshot,
        }
    }

    /// Gives `action` a shortcut (or none, for an empty string). An unusable shortcut is ignored.
    pub fn set(&mut self, action: HotkeyAction, shortcut: &str) {
        if shortcut.is_empty() {
            self.slot(action).clear();
        } else if let Some(shortcut) = normalize_hotkey(shortcut) {
            *self.slot(action) = shortcut;
        }
    }

    /// Another action that already uses `action`'s shortcut, if any.
    pub fn conflict(&self, action: HotkeyAction) -> Option<HotkeyAction> {
        let own = self.get(action);
        if own.is_empty() {
            return None;
        }
        HotkeyAction::ALL
            .into_iter()
            .find(|other| *other != action && self.get(*other) == own)
    }

    /// Replaces unusable shortcuts with the action's default and clears a later duplicate of an
    /// earlier shortcut, so no two actions share a key.
    pub fn sanitized(mut self) -> Self {
        for action in HotkeyAction::ALL {
            let current = self.get(action).to_owned();
            if !current.is_empty() {
                *self.slot(action) =
                    normalize_hotkey(&current).unwrap_or_else(|| action.default_key().to_owned());
            }
        }
        for (index, action) in HotkeyAction::ALL.into_iter().enumerate() {
            let own = self.get(action).to_owned();
            let taken = HotkeyAction::ALL[..index]
                .iter()
                .any(|earlier| !own.is_empty() && self.get(*earlier) == own);
            if taken {
                self.slot(action).clear();
            }
        }
        self
    }
}

const NAMED_KEYS: [&str; 20] = [
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "Backspace",
    "Delete",
    "End",
    "Enter",
    "Home",
    "Insert",
    "PageDown",
    "PageUp",
    "Space",
    "Tab",
    "Minus",
    "Equals",
    "Comma",
    "Period",
    "Slash",
    "Backtick",
];

fn is_key_name(name: &str) -> bool {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(only), None) => only.is_ascii_uppercase() || only.is_ascii_digit(),
        (Some('F'), Some(_)) => name[1..]
            .parse::<u32>()
            .is_ok_and(|number| (1..=24).contains(&number) && !name[1..].starts_with('0')),
        _ => NAMED_KEYS.contains(&name),
    }
}

/// Checks a shortcut such as `ctrl+shift+f8` and returns it in its canonical form
/// (`Ctrl+Alt+Shift+Key`), or `None` when it is not a usable shortcut.
pub fn normalize_hotkey(text: &str) -> Option<String> {
    if text.len() > 40 {
        return None;
    }
    let (mut ctrl, mut alt, mut shift, mut key) = (false, false, false, None);
    for part in text.split('+') {
        let part = part.trim();
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" | "option" => alt = true,
            "shift" => shift = true,
            _ => {
                if key.is_some() {
                    return None;
                }
                let canonical = canonical_key(part)?;
                key = Some(canonical);
            }
        }
    }
    let key = key?;
    let mut out = String::new();
    for (on, name) in [(ctrl, "Ctrl"), (alt, "Alt"), (shift, "Shift")] {
        if on {
            out.push_str(name);
            out.push('+');
        }
    }
    out.push_str(&key);
    Some(out)
}

fn canonical_key(part: &str) -> Option<String> {
    if let Some(named) = NAMED_KEYS
        .iter()
        .find(|named| named.eq_ignore_ascii_case(part))
    {
        return Some((*named).to_owned());
    }
    let upper = part.to_ascii_uppercase();
    is_key_name(&upper).then_some(upper)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_normalized() {
        assert_eq!(normalize_hotkey("f8").as_deref(), Some("F8"));
        assert_eq!(
            normalize_hotkey("shift+ctrl+f12").as_deref(),
            Some("Ctrl+Shift+F12")
        );
        assert_eq!(normalize_hotkey("alt + a").as_deref(), Some("Alt+A"));
        assert_eq!(normalize_hotkey("space").as_deref(), Some("Space"));
        assert_eq!(normalize_hotkey("pagedown").as_deref(), Some("PageDown"));
    }

    #[test]
    fn unusable_shortcuts_are_rejected() {
        for bad in [
            "",
            "Ctrl",
            "F0",
            "F25",
            "F08",
            "A+B",
            "Ctrl+Nope",
            "é",
            "Ctrl+",
            &"A".repeat(60),
        ] {
            assert_eq!(normalize_hotkey(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn defaults_have_no_conflicts() {
        let keys = Hotkeys::default();
        assert_eq!(keys.get(HotkeyAction::SaveClip), "F8");
        for action in HotkeyAction::ALL {
            assert_eq!(keys.conflict(action), None);
        }
        assert_eq!(keys.clone().sanitized(), keys);
    }

    #[test]
    fn a_shared_shortcut_is_reported_and_the_later_action_loses_it() {
        let mut keys = Hotkeys::default();
        keys.set(HotkeyAction::Screenshot, "F8");
        assert_eq!(
            keys.conflict(HotkeyAction::Screenshot),
            Some(HotkeyAction::SaveClip)
        );
        let keys = keys.sanitized();
        assert_eq!(keys.get(HotkeyAction::SaveClip), "F8");
        assert_eq!(keys.get(HotkeyAction::Screenshot), "");
    }

    #[test]
    fn setting_nothing_unbinds_and_garbage_is_ignored() {
        let mut keys = Hotkeys::default();
        keys.set(HotkeyAction::SaveClip, "");
        assert_eq!(keys.get(HotkeyAction::SaveClip), "");
        keys.set(HotkeyAction::ToggleRecording, "not a key");
        assert_eq!(keys.get(HotkeyAction::ToggleRecording), "F9");
        let damaged = Hotkeys {
            save_clip: "???".to_owned(),
            ..Hotkeys::default()
        }
        .sanitized();
        assert_eq!(damaged.save_clip, "F8");
    }
}

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
            Self::ToggleRecording => "Start or stop clipping",
            Self::ToggleMicrophone => "Mute or unmute microphone",
            Self::PushToTalk => "Push to talk",
            Self::Screenshot => "Take screenshot",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::SaveClip => "Keeps the last moments of play as a clip.",
            Self::ToggleRecording => {
                "Starts or stops clipping: the replay buffer that clips are saved from."
            }
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
    /// Extra bindings in action order; the original string fields remain compatible with older settings.
    pub alternatives: [Vec<String>; 5],
}

impl Default for Hotkeys {
    fn default() -> Self {
        let mut keys = Self {
            save_clip: String::new(),
            toggle_recording: String::new(),
            toggle_microphone: String::new(),
            push_to_talk: String::new(),
            screenshot: String::new(),
            alternatives: Default::default(),
        };
        for action in HotkeyAction::ALL {
            *keys.slot(action) = action.default_key().to_owned();
        }
        keys
    }
}

impl Hotkeys {
    /// Maximum bindings per action, including its original shortcut.
    pub const MAX_BINDINGS: usize = 5;

    fn index(action: HotkeyAction) -> usize {
        HotkeyAction::ALL
            .iter()
            .position(|item| *item == action)
            .unwrap_or(0)
    }

    /// All assigned shortcuts, with a stable, unique native registration ID.
    pub fn registrations(&self) -> impl Iterator<Item = (u32, HotkeyAction, &str)> {
        HotkeyAction::ALL
            .into_iter()
            .enumerate()
            .flat_map(move |(action_index, action)| {
                self.bindings(action)
                    .enumerate()
                    .map(move |(binding, key)| {
                        (
                            (action_index * Self::MAX_BINDINGS + binding + 1) as u32,
                            action,
                            key,
                        )
                    })
            })
    }

    /// Resolves an ID returned by a native shortcut listener.
    pub fn action_for_id(id: u32) -> Option<HotkeyAction> {
        let index = usize::try_from(id.checked_sub(1)?).ok()? / Self::MAX_BINDINGS;
        HotkeyAction::ALL.get(index).copied()
    }

    /// Assigned shortcuts for an action (at most five after validation).
    pub fn bindings(&self, action: HotkeyAction) -> impl Iterator<Item = &str> {
        std::iter::once(self.get(action))
            .chain(
                self.alternatives[Self::index(action)]
                    .iter()
                    .take(1024)
                    .map(String::as_str),
            )
            .filter(|key| !key.is_empty())
            .take(Self::MAX_BINDINGS)
    }

    /// Replaces a binding, or appends at the current count. Invalid and duplicate keys leave settings intact.
    pub fn set_binding(
        &mut self,
        action: HotkeyAction,
        index: usize,
        key: &str,
    ) -> Result<(), String> {
        let key = normalize_hotkey(key).ok_or("Choose a supported keyboard shortcut.")?;
        let mut bindings: Vec<String> = self.bindings(action).map(str::to_owned).collect();
        if index > bindings.len() || index >= Self::MAX_BINDINGS {
            return Err("Each action can have up to five shortcuts.".into());
        }
        for other in HotkeyAction::ALL {
            if self
                .bindings(other)
                .enumerate()
                .any(|(i, existing)| existing == key && (other != action || i != index))
            {
                return Err(format!(
                    "That shortcut is already assigned to {}.",
                    other.label()
                ));
            }
        }
        if index == bindings.len() {
            bindings.push(key);
        } else {
            bindings[index] = key;
        }
        self.replace_bindings(action, bindings);
        Ok(())
    }

    /// Removes a binding, promoting the next one to the original slot.
    pub fn remove_binding(&mut self, action: HotkeyAction, index: usize) {
        let mut bindings: Vec<String> = self.bindings(action).map(str::to_owned).collect();
        if index < bindings.len() {
            bindings.remove(index);
            self.replace_bindings(action, bindings);
        }
    }

    fn replace_bindings(&mut self, action: HotkeyAction, bindings: Vec<String>) {
        *self.slot(action) = bindings.first().cloned().unwrap_or_default();
        self.alternatives[Self::index(action)] = bindings.into_iter().skip(1).take(4).collect();
    }

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
        HotkeyAction::ALL.into_iter().find(|other| {
            *other != action
                && self
                    .bindings(action)
                    .any(|own| self.bindings(*other).any(|key| own == key))
        })
    }

    /// Replaces unusable shortcuts with the action's default and clears a later duplicate of an
    /// earlier shortcut, so no two actions share a key.
    pub fn sanitized(mut self) -> Self {
        let mut seen = Vec::new();
        for action in HotkeyAction::ALL {
            let primary = self.get(action);
            let primary = if primary.is_empty() {
                String::new()
            } else {
                normalize_hotkey(primary).unwrap_or_else(|| action.default_key().to_owned())
            };
            let candidates = std::iter::once(primary).chain(
                self.alternatives[Self::index(action)]
                    .iter()
                    .filter_map(|s| normalize_hotkey(s)),
            );
            let mut bindings = Vec::new();
            for key in candidates.take(1024) {
                if !key.is_empty() && !seen.contains(&key) && bindings.len() < Self::MAX_BINDINGS {
                    seen.push(key.clone());
                    bindings.push(key);
                }
            }
            self.replace_bindings(action, bindings);
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
    fn alternatives_round_trip_limit_collisions_and_registration_ids() {
        let old: Hotkeys = serde_json::from_str(r#"{"save_clip":"Ctrl+F8"}"#).unwrap();
        assert_eq!(
            old.bindings(HotkeyAction::SaveClip).collect::<Vec<_>>(),
            ["Ctrl+F8"]
        );
        let mut keys = old;
        for (index, key) in ["F1", "F2", "F3", "F4"].into_iter().enumerate() {
            keys.set_binding(HotkeyAction::SaveClip, index + 1, key)
                .unwrap();
        }
        let before = keys.clone();
        assert!(keys.set_binding(HotkeyAction::SaveClip, 5, "F5").is_err());
        assert!(
            keys.set_binding(HotkeyAction::Screenshot, 1, "ctrl+f8")
                .is_err()
        );
        assert!(keys.set_binding(HotkeyAction::SaveClip, 1, "Ctrl").is_err());
        assert_eq!(keys, before);
        let decoded: Hotkeys = serde_json::from_slice(&serde_json::to_vec(&keys).unwrap()).unwrap();
        assert_eq!(decoded, keys);
        let registrations: Vec<_> = keys.registrations().collect();
        for (id, action, _) in &registrations {
            assert_eq!(Hotkeys::action_for_id(*id), Some(*action));
        }
        assert_eq!(Hotkeys::action_for_id(0), None);
        assert_eq!(Hotkeys::action_for_id(26), None);
        keys.remove_binding(HotkeyAction::SaveClip, 0);
        assert_eq!(keys.save_clip, "F1");
        assert_eq!(keys.bindings(HotkeyAction::SaveClip).count(), 4);
    }

    #[test]
    fn hostile_alternatives_are_bounded_normalized_and_deduplicated() {
        let mut keys = Hotkeys::default();
        keys.alternatives[0] = vec![
            "f9".into(),
            "F8".into(),
            "???".into(),
            "F1".into(),
            "F2".into(),
            "F3".into(),
            "F4".into(),
            "F5".into(),
        ];
        keys.alternatives[1] = vec!["F1".into(); 10_000];
        let keys = keys.sanitized();
        assert_eq!(
            keys.bindings(HotkeyAction::SaveClip).collect::<Vec<_>>(),
            ["F8", "F9", "F1", "F2", "F3"]
        );
        assert_eq!(keys.bindings(HotkeyAction::ToggleRecording).count(), 0);
        assert_eq!(keys.registrations().count(), 7);
    }

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

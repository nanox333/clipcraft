//! Explicit disclosure policy for Discord. Never forward raw game data or library metadata.
use crate::game_state::{GameIntegration, GameSnapshot};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct PresenceSettings {
    pub enabled: bool,
    pub application_id: String,
    pub show_game: bool,
    pub show_match: bool,
}
impl PresenceSettings {
    pub fn sanitized(mut self) -> Self {
        self.application_id = self.application_id.trim().to_owned();
        if !valid_application_id(&self.application_id) {
            self.application_id.clear();
        }
        self
    }
}
pub fn valid_application_id(id: &str) -> bool {
    (17..=20).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_digit())
        && !id.starts_with('0')
        && id.parse::<u64>().is_ok()
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceMode {
    #[default]
    Browsing,
    Clipping,
    Editing,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct PresenceContext {
    pub mode: PresenceMode,
    /// Current captured game's label, never a saved clip's title/game label.
    pub game_name: Option<String>,
    /// Only a fresh Live integration snapshot belongs here; failures/staleness supply None.
    pub snapshot: Option<Box<GameSnapshot>>,
}

/// The entire permitted publication. This type cannot contain media, account data or secrets.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PresenceActivity {
    pub details: String,
    pub game: Option<String>,
    pub match_context: Option<String>,
}
impl PresenceActivity {
    pub fn wire(&self) -> serde_json::Value {
        let mut activity = serde_json::json!({"details":self.details});
        if let Some(game) = &self.game {
            let text = self
                .match_context
                .as_ref()
                .map_or_else(|| game.clone(), |m| format!("{game} · {m}"));
            activity["state"] = serde_json::Value::String(bound(&text, 128));
        }
        activity
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PresenceTarget {
    pub application_id: String,
    pub activity: PresenceActivity,
    /// Used locally to clear old disclosures before a more restrictive policy is applied.
    pub disclosure: u8,
}
fn bound(text: &str, bytes: usize) -> String {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
fn clean(text: &str, bytes: usize) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_control) || text.contains(['/', '\\']) {
        return None;
    }
    Some(bound(text, bytes))
}
pub fn integration_for_name(name: &str) -> Option<GameIntegration> {
    match name.trim().to_ascii_lowercase().as_str() {
        "counter-strike 2" | "cs2" => Some(GameIntegration::Cs2),
        "dota 2" | "dota2" => Some(GameIntegration::Dota2),
        "league of legends" => Some(GameIntegration::League),
        _ => None,
    }
}

pub fn build_presence(
    settings: &PresenceSettings,
    context: &PresenceContext,
) -> Option<PresenceTarget> {
    if !settings.enabled || !valid_application_id(&settings.application_id) {
        return None;
    }
    let mut activity = PresenceActivity {
        details: match context.mode {
            PresenceMode::Browsing => "Browsing clips",
            PresenceMode::Clipping => "Clipping",
            PresenceMode::Editing => "Editing clips",
        }
        .into(),
        game: None,
        match_context: None,
    };
    if settings.show_game && context.mode == PresenceMode::Clipping {
        activity.game = context.game_name.as_deref().and_then(|s| clean(s, 64));
        if settings.show_match
            && let (Some(name), Some(snapshot)) = (&activity.game, &context.snapshot)
            && integration_for_name(name) == Some(snapshot.game)
        {
            let mut fields = Vec::new();
            for field in [&snapshot.map, &snapshot.mode, &snapshot.character]
                .into_iter()
                .flatten()
            {
                if let Some(text) = clean(field, 48) {
                    fields.push(text);
                }
            }
            if let (Some(a), Some(b)) = (snapshot.team_score, snapshot.opponent_score)
                && a <= 100_000
                && b <= 100_000
            {
                fields.push(format!("{a}–{b}"));
            }
            if !fields.is_empty() {
                activity.match_context = Some(bound(&fields.join(" · "), 128));
            }
        }
    }
    Some(PresenceTarget {
        application_id: settings.application_id.clone(),
        activity,
        disclosure: u8::from(settings.show_game)
            | (u8::from(settings.show_game && settings.show_match) << 1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> PresenceSettings {
        PresenceSettings {
            enabled: true,
            application_id: "123456789012345678".into(),
            ..Default::default()
        }
    }
    fn context() -> PresenceContext {
        PresenceContext { mode:PresenceMode::Clipping,game_name:Some("Counter-Strike 2".into()),snapshot:Some(Box::new(crate::game_state::parse_game_state(GameIntegration::Cs2,br#"{"provider":{"appid":730,"steamid":"private"},"map":{"name":"de_mirage","mode":"competitive","team_ct":{"score":8},"team_t":{"score":6}},"player":{"steamid":"private","team":"CT","state":{"health":73,"money":555},"match_stats":{"kills":99}}}"#).unwrap())) }
    }
    #[test]
    fn disclosure_matrix_is_off_then_activity_then_game_then_selected_match_fields() {
        assert!(build_presence(&PresenceSettings::default(), &context()).is_none());
        for game in [false, true] {
            for details in [false, true] {
                let mut s = settings();
                s.show_game = game;
                s.show_match = details;
                let target = build_presence(&s, &context()).unwrap();
                let wire = target.activity.wire();
                assert_eq!(wire["details"], "Clipping");
                assert_eq!(wire.get("state").is_some(), game);
                let text = wire.to_string();
                assert_eq!(text.contains("de_mirage"), game && details);
                for forbidden in [
                    "private", "steamid", "73", "555", "99", "kills", "health", "money", "assets",
                    "secrets",
                ] {
                    assert!(!text.contains(forbidden));
                }
            }
        }
    }
    #[test]
    fn editing_stale_closed_and_mismatched_games_remove_context() {
        let mut s = settings();
        s.show_game = true;
        s.show_match = true;
        let mut c = context();
        c.snapshot = None;
        assert!(
            build_presence(&s, &c)
                .unwrap()
                .activity
                .match_context
                .is_none()
        );
        c = context();
        c.game_name = Some("Dota 2".into());
        assert!(
            build_presence(&s, &c)
                .unwrap()
                .activity
                .match_context
                .is_none()
        );
        c.game_name = None;
        assert!(build_presence(&s, &c).unwrap().activity.game.is_none());
        for mode in [PresenceMode::Browsing, PresenceMode::Editing] {
            c = context();
            c.mode = mode;
            let a = build_presence(&s, &c).unwrap().activity;
            assert!(a.game.is_none() && a.match_context.is_none());
        }
    }
    #[test]
    fn identifiers_and_hostile_fields_are_validated_and_utf8_output_is_bounded() {
        for id in [
            "",
            "123",
            "000000000000000000",
            "12345678901234567x",
            "18446744073709551616",
            "1\n23456789012345678",
        ] {
            assert!(!valid_application_id(id));
            let mut s = settings();
            s.application_id = id.into();
            assert!(build_presence(&s, &context()).is_none());
        }
        let mut s = settings();
        s.show_game = true;
        s.show_match = true;
        for name in ["C:\\private\\clip.mp4", "/private/clip.mp4", "bad\nname"] {
            let mut c = context();
            c.game_name = Some(name.into());
            assert!(build_presence(&s, &c).unwrap().activity.game.is_none());
        }
        let mut c = context();
        c.game_name = Some("é".repeat(200));
        let a = build_presence(&s, &c).unwrap().activity;
        assert!(a.game.as_ref().unwrap().len() <= 64);
        assert!(a.wire()["state"].as_str().unwrap().len() <= 128);
        let restored: crate::RecordingSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(restored.discord_presence, PresenceSettings::default());
    }
}

//! Game name matching for process and window metadata.
//!
//! The longest-needle and stable tie-breaking rule is a Rust adaptation of
//! Vice's `_best_game_match` in `vice/main.py` at commit
//! `7e4b2a3bccc6e43285d095afe95d0d7fe3d0c84c`, used with permission reported
//! by the ClipCraft maintainer. Bundled recognition metadata lives in game_catalog;
//! callers of this matcher provide custom rules and process metadata.

use serde::{Deserialize, Serialize};

/// A configured game process that currently has a capturable game window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameProcessMatch {
    /// User-facing game label from the matching rule.
    pub game_name: String,
    /// Operating-system process that owns the selected game window.
    pub process_id: u32,
}

/// Decision produced by the game-only replay arming state machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GameArmingDecision {
    /// Start capture for this game's process.
    Start(GameProcessMatch),
    /// Continue on a desktop only after an actual, successfully captured game exited.
    StartDesktop,
    /// Stop the current game-managed capture because its process is no longer present.
    Stop,
}

/// Tracks the process that currently owns the replay session and a user's manual pause.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GameArmingState {
    observed: Option<GameProcessMatch>,
    recording: Option<GameProcessMatch>,
    paused: bool,
    desktop: bool,
}

impl GameArmingState {
    /// Updates the detected processes and returns the next capture transition, if any.
    pub fn observe(&mut self, matches: &[GameProcessMatch]) -> Option<GameArmingDecision> {
        self.observe_with_desktop(matches, false)
    }

    /// `allow_desktop` must be false for errors, settings changes and still-running processes.
    pub fn observe_with_desktop(
        &mut self,
        matches: &[GameProcessMatch],
        allow_desktop: bool,
    ) -> Option<GameArmingDecision> {
        self.observed = matches.first().cloned();

        if self
            .recording
            .as_ref()
            .is_some_and(|recording| !matches.iter().any(|detected| detected == recording))
        {
            self.recording = None;
            if self.observed.is_none() {
                self.paused = false;
            }
            return Some(
                if allow_desktop && self.observed.is_none() && !self.paused {
                    GameArmingDecision::StartDesktop
                } else {
                    GameArmingDecision::Stop
                },
            );
        }

        if self.desktop {
            if !allow_desktop {
                return Some(GameArmingDecision::Stop);
            }
            return self.observed.clone().map(GameArmingDecision::Start);
        }

        let Some(current) = self.observed.as_ref() else {
            self.paused = false;
            return None;
        };
        if self.recording.is_none() && !self.paused {
            return Some(GameArmingDecision::Start(current.clone()));
        }
        None
    }

    /// Records that capture successfully started for a detected or manually resumed game.
    pub fn capture_started(&mut self, game: GameProcessMatch) {
        self.recording = Some(game);
        self.desktop = false;
        self.paused = false;
    }

    pub fn desktop_started(&mut self) {
        self.recording = None;
        self.desktop = true;
        self.paused = false;
    }

    pub fn is_desktop(&self) -> bool {
        self.desktop
    }

    /// Records that the user stopped capture, suppressing automatic restart until the game exits.
    pub fn capture_stopped_by_user(&mut self) {
        self.recording = None;
        self.desktop = false;
        self.paused = self.observed.is_some();
    }

    /// Clears a failed start so the next process scan can retry.
    pub fn capture_start_failed(&mut self) {
        self.recording = None;
        self.desktop = false;
    }

    /// Clears a game-managed session after its process exits or game-only mode is disabled.
    pub fn capture_stopped_automatically(&mut self) {
        self.recording = None;
        self.desktop = false;
    }

    /// The game process currently detected by the latest scan.
    pub fn observed(&self) -> Option<&GameProcessMatch> {
        self.observed.as_ref()
    }

    /// The game that owns the current replay buffer.
    pub fn recording(&self) -> Option<&GameProcessMatch> {
        self.recording.as_ref()
    }

    /// Whether the user paused capture for the still-running game.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Clears remembered detection and pause state when game-only mode is turned off.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// A game name and the process, window-class, or store-ID strings that identify it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct GameMatchRule {
    /// Name shown in ClipCraft.
    pub name: String,
    /// Case-insensitive strings, matched as substrings unless `exact_executable` is set.
    pub needles: Vec<String>,
    /// Exact Steam application IDs. Unlike process-name needles, IDs are not substring matched.
    pub steam_app_ids: Vec<String>,
    /// Match whole executable basenames instead of substrings. False for existing custom rules.
    pub exact_executable: bool,
}

impl GameMatchRule {
    /// Makes a game rule from a display name and matching strings.
    pub fn new(
        name: impl Into<String>,
        needles: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            needles: needles.into_iter().map(Into::into).collect(),
            steam_app_ids: Vec::new(),
            exact_executable: false,
        }
    }

    /// Adds exact Steam application IDs to this rule.
    pub fn with_steam_app_ids(
        mut self,
        steam_app_ids: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.steam_app_ids = steam_app_ids.into_iter().map(Into::into).collect();
        self
    }

    /// Trims and bounds user-provided rule text before it is stored.
    pub fn sanitized(mut self) -> Self {
        fn truncate(text: &mut String, max_bytes: usize) {
            let mut end = max_bytes.min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }

        self.name = self.name.trim().to_owned();
        truncate(&mut self.name, 100);
        self.needles = self
            .needles
            .into_iter()
            .take(8)
            .map(|needle| {
                let mut needle = needle.trim().to_owned();
                truncate(&mut needle, 200);
                needle
            })
            .filter(|needle| !needle.is_empty())
            .collect();
        self.steam_app_ids = self
            .steam_app_ids
            .into_iter()
            .take(8)
            .map(|id| id.trim().to_owned())
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= 20
                    && id != "0"
                    && id.bytes().all(|byte| byte.is_ascii_digit())
            })
            .collect();
        self
    }
}

impl Default for GameMatchRule {
    fn default() -> Self {
        Self::new("", std::iter::empty::<String>())
    }
}

/// Finds the rule whose longest non-empty needle matches a candidate under that rule's mode.
///
/// Candidate strings commonly include an executable name, window class, and a
/// store-specific application ID. Equal-length matches keep the earlier rule,
/// so list order is a stable tie-breaker. Empty needles never match. Exact-mode rules require
/// whole-string equality; the process observer supplies executable basenames.
pub fn best_game_match<'a>(rules: &'a [GameMatchRule], candidates: &[&str]) -> Option<&'a str> {
    let candidates: Vec<String> = candidates
        .iter()
        .map(|candidate| candidate.to_lowercase())
        .collect();
    let mut best: Option<(&str, usize)> = None;

    for rule in rules {
        if rule.name.trim().is_empty() {
            continue;
        }
        for needle in &rule.needles {
            let needle = needle.to_lowercase();
            let length = needle.chars().count();
            if length == 0 || best.is_some_and(|(_, best_length)| length <= best_length) {
                continue;
            }
            if candidates.iter().any(|candidate| {
                if rule.exact_executable {
                    candidate == &needle
                } else {
                    candidate.contains(&needle)
                }
            }) {
                best = Some((&rule.name, length));
            }
        }
    }

    best.map(|(name, _)| name)
}

/// Checks user-defined rules first, then a supplied default list.
///
/// A user's explicit rule wins even if a default rule has a longer match.
pub fn match_game_with_user_priority<'a>(
    user_rules: &'a [GameMatchRule],
    default_rules: &'a [GameMatchRule],
    candidates: &[&str],
) -> Option<&'a str> {
    best_game_match(user_rules, candidates).or_else(|| best_game_match(default_rules, candidates))
}

/// Matches one running process using exact/substring executable rules and exact Steam IDs.
/// An exact Steam ID wins over a generic executable match, which handles games that share a
/// launcher binary.
pub fn match_game_process<'a>(
    rules: &'a [GameMatchRule],
    executable: &str,
    steam_app_id: Option<&str>,
) -> Option<&'a str> {
    if let Some(app_id) = steam_app_id
        && let Some(rule) = rules.iter().find(|rule| {
            !rule.name.trim().is_empty() && rule.steam_app_ids.iter().any(|id| id == app_id)
        })
    {
        return Some(&rule.name);
    }
    best_game_match(rules, &[executable])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_match_wins_over_a_generic_executable_name() {
        let rules = [
            GameMatchRule::new("Unreal Game", ["client-win64-shipping.exe"]),
            GameMatchRule::new("Specific Game", ["specific-game-client-win64-shipping.exe"]),
        ];

        assert_eq!(
            best_game_match(&rules, &["specific-game-client-win64-shipping.exe"]),
            Some("Specific Game")
        );
    }

    #[test]
    fn a_longer_store_id_does_not_match_a_shorter_ids_game() {
        let rules = [
            GameMatchRule::new("Game 400", ["steam_app_400"]),
            GameMatchRule::new("Game 4000", ["steam_app_4000"]),
        ];

        assert_eq!(
            best_game_match(&rules, &["steam_app_4000"]),
            Some("Game 4000")
        );
    }

    #[test]
    fn matches_are_case_insensitive_and_can_use_multiple_metadata_fields() {
        let rules = [GameMatchRule::new(
            "Counter-Strike 2",
            ["cs2.exe", "CounterStrike"],
        )];

        assert_eq!(
            best_game_match(&rules, &["steam", "CS2.EXE"]),
            Some("Counter-Strike 2")
        );
        assert_eq!(
            best_game_match(&rules, &["steam", "counterstrike"]),
            Some("Counter-Strike 2")
        );
    }

    #[test]
    fn equal_length_matches_keep_rule_order() {
        let rules = [
            GameMatchRule::new("First", ["game.exe"]),
            GameMatchRule::new("Second", ["game.exe"]),
        ];

        assert_eq!(best_game_match(&rules, &["game.exe"]), Some("First"));
    }

    #[test]
    fn user_rules_take_priority_over_defaults() {
        let user = [GameMatchRule::new("My custom name", ["game.exe"])];
        let defaults = [GameMatchRule::new("Bundled name", ["specific-game.exe"])];

        assert_eq!(
            match_game_with_user_priority(&user, &defaults, &["specific-game.exe", "game.exe"]),
            Some("My custom name")
        );
        assert_eq!(
            match_game_with_user_priority(&[], &defaults, &["specific-game.exe"]),
            Some("Bundled name")
        );
    }

    #[test]
    fn empty_needles_do_not_match_everything() {
        let rules = [GameMatchRule::new("Invalid", [""])];

        assert_eq!(best_game_match(&rules, &["a running process"]), None);
    }

    #[test]
    fn a_rule_with_no_display_name_is_not_armed() {
        let rules = [GameMatchRule::new("", ["game.exe"])];
        assert_eq!(match_game_process(&rules, "game.exe", None), None);
    }

    #[test]
    fn steam_application_ids_match_exactly_and_override_shared_binaries() {
        let rules = [
            GameMatchRule::new("Generic source game", ["source.exe"]),
            GameMatchRule::new("Half-Life 2", ["source.exe"]).with_steam_app_ids(["220"]),
            GameMatchRule::new("Half-Life 2 Deathmatch", ["source.exe"])
                .with_steam_app_ids(["320"]),
        ];
        assert_eq!(
            match_game_process(&rules, "source.exe", Some("320")),
            Some("Half-Life 2 Deathmatch")
        );
        assert_eq!(
            match_game_process(&rules, "source.exe", Some("3200")),
            Some("Generic source game")
        );
    }

    #[test]
    fn game_rules_deserialize_without_new_fields_and_sanitize_limits() {
        let old: GameMatchRule =
            serde_json::from_str(r#"{"name":"Game","needles":["game.exe"]}"#).unwrap();
        assert!(old.steam_app_ids.is_empty());
        let rule = GameMatchRule::new(
            " Game ",
            vec![" game.exe ".to_owned(), String::new(), "x".repeat(250)],
        )
        .with_steam_app_ids(["570", "0", "not-an-id"])
        .sanitized();
        assert_eq!(rule.name, "Game");
        assert_eq!(rule.needles.len(), 2);
        assert_eq!(rule.needles[1].len(), 200);
        assert_eq!(rule.steam_app_ids, ["570"]);
    }

    #[test]
    fn arming_starts_once_and_stops_after_the_matched_process_exits() {
        let game = GameProcessMatch {
            game_name: "Counter-Strike 2".to_owned(),
            process_id: 42,
        };
        let mut arming = GameArmingState::default();

        assert_eq!(
            arming.observe(std::slice::from_ref(&game)),
            Some(GameArmingDecision::Start(game.clone()))
        );
        arming.capture_started(game.clone());
        assert_eq!(arming.observe(std::slice::from_ref(&game)), None);
        assert_eq!(arming.observe(&[]), Some(GameArmingDecision::Stop));
        assert_eq!(arming.observe(&[]), None);
    }

    #[test]
    fn manual_stop_pauses_until_the_game_process_exits() {
        let game = GameProcessMatch {
            game_name: "Counter-Strike 2".to_owned(),
            process_id: 42,
        };
        let mut arming = GameArmingState::default();
        arming.observe(std::slice::from_ref(&game));
        arming.capture_started(game.clone());
        arming.capture_stopped_by_user();
        assert_eq!(arming.observe(std::slice::from_ref(&game)), None);
        assert_eq!(arming.observe(&[]), None);
        assert_eq!(
            arming.observe(std::slice::from_ref(&game)),
            Some(GameArmingDecision::Start(game))
        );
    }

    #[test]
    fn manual_pause_waits_for_all_selected_games_to_exit() {
        let first = GameProcessMatch {
            game_name: "First".to_owned(),
            process_id: 42,
        };
        let second = GameProcessMatch {
            game_name: "Second".to_owned(),
            process_id: 84,
        };
        let mut arming = GameArmingState::default();
        arming.observe(std::slice::from_ref(&first));
        arming.capture_started(first.clone());
        arming.capture_stopped_by_user();
        assert_eq!(arming.observe(&[first, second]), None);
        assert!(arming.is_paused());
        assert_eq!(arming.observe(&[]), None);
        assert!(!arming.is_paused());
    }

    #[test]
    fn a_reused_game_name_with_a_new_process_restarts_the_session() {
        let original = GameProcessMatch {
            game_name: "Game".to_owned(),
            process_id: 42,
        };
        let replacement = GameProcessMatch {
            process_id: 84,
            ..original.clone()
        };
        let mut arming = GameArmingState::default();
        arming.observe(std::slice::from_ref(&original));
        arming.capture_started(original);
        assert_eq!(
            arming.observe(std::slice::from_ref(&replacement)),
            Some(GameArmingDecision::Stop)
        );
        assert_eq!(
            arming.observe(std::slice::from_ref(&replacement)),
            Some(GameArmingDecision::Start(replacement))
        );
    }
}

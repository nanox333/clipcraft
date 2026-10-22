//! Offline game recognition; bundled Vice metadata with ClipCraft identification supplements.

use serde::Serialize;

use crate::{EngineError, GameMatchRule, RecordingSettings};

/// One automatically recognized game; inclusion does not establish platform/capture compatibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct BuiltInGame {
    /// Stable CLI/catalog identifier.
    pub id: &'static str,
    /// Display and recorded-game label.
    pub name: &'static str,
    /// Exact process basenames and Vice aliases; generic runtimes/launchers are excluded.
    pub executables: &'static [&'static str],
    /// Exact numeric Steam IDs, when applicable. Read from processes only on Linux.
    pub steam_app_ids: &'static [&'static str],
}

impl BuiltInGame {
    /// Returns a rule representation of the built-in identification metadata.
    pub fn rule(self) -> GameMatchRule {
        let mut rule = GameMatchRule::new(self.name, self.executables.iter().copied())
            .with_steam_app_ids(self.steam_app_ids.iter().copied());
        rule.exact_executable = true;
        rule
    }

    /// Whether automatic recognition permits this built-in game.
    pub fn enabled(self, settings: &RecordingSettings) -> bool {
        settings.detect_built_in_games
            && !settings
                .disabled_builtin_games
                .iter()
                .any(|id| id == self.id)
    }
}

const fn game(
    id: &'static str,
    name: &'static str,
    executables: &'static [&'static str],
    steam_app_ids: &'static [&'static str],
) -> BuiltInGame {
    BuiltInGame {
        id,
        name,
        executables,
        steam_app_ids,
    }
}

#[path = "game_catalog_data.rs"]
mod data;
use data::CATALOG;

/// Maximum exclusions or IDs per request, independent of the 64 custom-rule limit.
pub const MAX_BUILT_IN_GAMES: usize = 1024;

/// Built-in identification metadata, recognized automatically unless disabled in settings.
pub const fn built_in_games() -> &'static [BuiltInGame] {
    CATALOG
}

impl RecordingSettings {
    /// Custom rules take priority over all bundled identifiers, as in Vice's matching flow.
    pub fn match_game<'a>(
        &'a self,
        executable: &str,
        steam_app_id: Option<&str>,
    ) -> Option<&'a str> {
        if let Some(name) = crate::match_game_process(&self.game_rules, executable, steam_app_id) {
            return Some(name);
        }
        if let Some(id) = steam_app_id
            && let Some(game) = built_in_games()
                .iter()
                .find(|game| game.enabled(self) && game.steam_app_ids.contains(&id))
        {
            return Some(game.name);
        }
        built_in_games()
            .iter()
            .find(|game| {
                game.enabled(self)
                    && game
                        .executables
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(executable))
            })
            .map(|game| game.name)
    }

    /// Avoid observing processes when neither built-in nor custom recognition is enabled.
    pub fn has_game_rules(&self) -> bool {
        !self.game_rules.is_empty() || built_in_games().iter().any(|game| game.enabled(self))
    }

    /// Steam IDs are read only while a permitted rule can use them.
    pub fn needs_steam_game_ids(&self) -> bool {
        self.game_rules
            .iter()
            .any(|rule| !rule.steam_app_ids.is_empty())
            || built_in_games()
                .iter()
                .any(|game| game.enabled(self) && !game.steam_app_ids.is_empty())
    }
}

/// Changes built-in exclusions on a settings copy; custom rules and the global switch are kept.
pub(crate) fn set_games(
    mut settings: RecordingSettings,
    ids: &[String],
    enabled: bool,
) -> Result<RecordingSettings, EngineError> {
    if ids.is_empty()
        || ids.len() > MAX_BUILT_IN_GAMES
        || settings.disabled_builtin_games.len() > MAX_BUILT_IN_GAMES
    {
        return Err(EngineError::GameRuleLimit);
    }
    for id in ids {
        if !built_in_games().iter().any(|game| game.id == id) {
            return Err(EngineError::GamePresetInvalid);
        }
        if enabled {
            settings
                .disabled_builtin_games
                .retain(|disabled| disabled != id);
        } else if !settings.disabled_builtin_games.contains(id) {
            settings.disabled_builtin_games.push(id.clone());
        }
    }
    settings.disabled_builtin_games.sort();
    settings.disabled_builtin_games.dedup();
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, CommandOutput, execute, match_game_process};

    #[test]
    fn every_catalog_executable_and_steam_id_matches_without_launcher_lookalikes() {
        let catalog = built_in_games();
        assert_eq!(catalog.len(), 348);
        assert!(catalog.len() <= MAX_BUILT_IN_GAMES);
        let mut ids = std::collections::HashSet::new();
        let mut executables = std::collections::HashSet::new();
        let mut steam_ids = std::collections::HashSet::new();
        let rules: Vec<_> = catalog.iter().map(|game| game.rule()).collect();
        for game in catalog {
            assert!(ids.insert(game.id));
            assert_eq!(game.rule(), game.rule().sanitized());
            assert!(crate::clean_name(game.name).is_some());
            for name in game.executables {
                assert!(executables.insert(name.to_lowercase()));
                assert_eq!(
                    RecordingSettings::default().match_game(&name.to_uppercase(), None),
                    Some(game.name)
                );
                assert_eq!(
                    match_game_process(&rules, &name.to_uppercase(), None),
                    Some(game.name)
                );
                for lookalike in [
                    format!("launcher-{name}"),
                    format!("{name}.launcher"),
                    format!("{name}-helper"),
                ] {
                    assert_eq!(match_game_process(&rules, &lookalike, None), None);
                }
            }
            for id in game.steam_app_ids {
                assert!(steam_ids.insert(*id));
                assert!(id.parse::<u64>().is_ok());
                assert_eq!(
                    RecordingSettings::default().match_game("wine64-preloader", Some(id)),
                    Some(game.name)
                );
                assert_eq!(
                    match_game_process(&rules, "wine64-preloader", Some(id)),
                    Some(game.name)
                );
                assert_eq!(
                    match_game_process(&[game.rule()], "unknown", Some(&format!("{id}0"))),
                    None
                );
            }
        }
        for unrelated in [
            "steam.exe",
            "EpicGamesLauncher.exe",
            "LeagueClient.exe",
            "java.exe",
            "javaw.exe",
            "wine64",
            "RiotClientServices.exe",
            "WowVoiceProxy.exe",
            "Rust.exe",
            "Minecraft-launcher",
            "PrismLauncher",
            "tlauncher",
            "FortniteLauncher",
            "PlayGTAV.exe",
            "Client-Win64-Shipping.exe",
        ] {
            assert_eq!(match_game_process(&rules, unrelated, None), None);
            assert_eq!(
                RecordingSettings::default().match_game(unrelated, None),
                None
            );
        }
    }

    #[test]
    fn built_in_games_work_with_an_empty_custom_list_and_custom_names_win() {
        let mut settings = RecordingSettings::default();
        assert!(settings.game_rules.is_empty());
        assert!(settings.detect_built_in_games && settings.has_game_rules());
        assert!(settings.needs_steam_game_ids());
        assert_eq!(
            settings.match_game("CS2.EXE", None),
            Some("Counter-Strike 2")
        );
        assert_eq!(
            settings.match_game("wine64-preloader", Some("570")),
            Some("Dota 2")
        );
        settings
            .game_rules
            .push(GameMatchRule::new("My CS label", ["cs2"]));
        assert_eq!(settings.match_game("cs2.exe", None), Some("My CS label"));
        // The custom process match wins even over a bundled Steam identifier.
        assert_eq!(
            settings.match_game("cs2.exe", Some("570")),
            Some("My CS label")
        );
        assert_eq!(settings.match_game("unrelated.exe", None), None);
    }

    #[test]
    fn default_recognition_drives_game_arming_and_exclusions_are_bounded() {
        let settings = RecordingSettings::default();
        let detected = crate::GameProcessMatch {
            game_name: settings.match_game("cs2.exe", None).unwrap().to_owned(),
            process_id: 123,
        };
        let mut arming = crate::GameArmingState::default();
        assert_eq!(
            arming.observe(std::slice::from_ref(&detected)),
            Some(crate::GameArmingDecision::Start(detected.clone()))
        );
        arming.capture_started(detected.clone());
        assert_eq!(arming.observe(&[detected]), None);
        assert_eq!(arming.observe(&[]), Some(crate::GameArmingDecision::Stop));
        let settings = RecordingSettings {
            disabled_builtin_games: vec!["cs2".into(), "cs2".into(), "bad-id".into(), "wow".into()],
            ..Default::default()
        }
        .sanitized();
        assert_eq!(settings.disabled_builtin_games, ["cs2", "wow"]);
        assert_eq!(settings.match_game("cs2.exe", None), None);
    }

    #[test]
    fn exclusions_global_off_and_custom_fallback_are_independent() {
        let settings = RecordingSettings::default();
        let disabled = set_games(settings, &["cs2".into()], false).unwrap();
        assert_eq!(disabled.match_game("cs2.exe", Some("730")), None);
        assert_eq!(
            disabled.match_game("Wow.exe", None),
            Some("World of Warcraft")
        );
        assert_eq!(
            set_games(disabled.clone(), &["cs2".into()], false).unwrap(),
            disabled
        );
        let enabled = set_games(disabled, &["cs2".into()], true).unwrap();
        assert_eq!(
            enabled.match_game("cs2.exe", None),
            Some("Counter-Strike 2")
        );
        let ids: Vec<_> = built_in_games()
            .iter()
            .map(|game| game.id.to_owned())
            .collect();
        let mut all_off = set_games(enabled, &ids, false).unwrap();
        assert!(!all_off.has_game_rules());
        assert!(!all_off.needs_steam_game_ids());
        all_off.detect_built_in_games = false;
        all_off
            .game_rules
            .push(GameMatchRule::new("My game", ["mygame.exe"]));
        assert!(all_off.has_game_rules());
        assert_eq!(all_off.match_game("mygame.exe", None), Some("My game"));
        assert_eq!(all_off.match_game("cs2.exe", None), None);
    }

    #[test]
    fn unknown_and_oversized_changes_are_refused_atomically() {
        let settings = RecordingSettings::default();
        let command = Command::SetBuiltInGames {
            settings: Box::new(settings.clone()),
            ids: vec!["cs2".into(), "unknown".into()],
            enabled: false,
        };
        assert_eq!(command.id(), "recording.set_builtin_games");
        assert_eq!(execute(command), Err(EngineError::GamePresetInvalid));
        assert!(settings.disabled_builtin_games.is_empty());
        assert_eq!(
            set_games(settings.clone(), &[], false),
            Err(EngineError::GameRuleLimit)
        );
        assert_eq!(
            set_games(settings, &vec!["cs2".into(); MAX_BUILT_IN_GAMES + 1], false),
            Err(EngineError::GameRuleLimit)
        );
    }

    #[test]
    fn bulk_exclusions_survive_normalization_and_settings_file_reload() {
        let ids: Vec<_> = built_in_games()
            .iter()
            .map(|game| game.id.to_owned())
            .collect();
        let mut original = RecordingSettings::default();
        original
            .game_rules
            .push(GameMatchRule::new("My game", ["personal-game"]));
        let disabled = set_games(original.clone(), &ids, false).unwrap();
        assert_eq!(disabled.disabled_builtin_games.len(), 348);
        assert_eq!(disabled.clone().sanitized(), disabled);
        let path =
            std::env::temp_dir().join(format!("clipcraft-all-games-{}.json", std::process::id()));
        execute(Command::SaveRecordingSettings {
            path: path.clone(),
            settings: Box::new(disabled.clone()),
        })
        .unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() < crate::recording::MAX_SETTINGS_BYTES);
        assert_eq!(
            execute(Command::LoadRecordingSettings { path: path.clone() }).unwrap(),
            CommandOutput::RecordingSettings(Box::new(disabled.clone()))
        );
        assert_eq!(
            disabled.match_game("personal-game.exe", None),
            Some("My game")
        );
        for game in built_in_games() {
            for executable in game.executables {
                assert_eq!(disabled.match_game(executable, None), None);
            }
            for id in game.steam_app_ids {
                assert_eq!(disabled.match_game("wine64", Some(id)), None);
            }
        }
        assert_eq!(set_games(disabled, &ids, true).unwrap(), original);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn similarly_named_games_and_steam_ids_remain_distinct() {
        let settings = RecordingSettings::default();
        assert_eq!(settings.match_game("Hades.exe", None), Some("Hades"));
        assert_eq!(settings.match_game("Hades2.exe", None), Some("Hades II"));
        assert_eq!(
            settings.match_game("wine64", Some("1145350")),
            Some("Hades II")
        );
        assert_eq!(settings.match_game("wine64", Some("11453500")), None);
        assert_eq!(settings.match_game("launcher-Hades2.exe", None), None);
    }

    #[test]
    fn exclusions_and_global_switch_survive_reload_and_old_custom_rules_keep_their_meaning() {
        let path = std::env::temp_dir().join(format!(
            "clipcraft-game-detection-{}.json",
            std::process::id()
        ));
        let mut settings = set_games(RecordingSettings::default(), &["wow".into()], false).unwrap();
        settings.detect_built_in_games = false;
        settings
            .game_rules
            .push(GameMatchRule::new("Custom", ["game"]));
        execute(Command::SaveRecordingSettings {
            path: path.clone(),
            settings: Box::new(settings.clone()),
        })
        .unwrap();
        assert_eq!(
            execute(Command::LoadRecordingSettings { path: path.clone() }).unwrap(),
            CommandOutput::RecordingSettings(Box::new(settings))
        );
        let legacy: GameMatchRule =
            serde_json::from_str(r#"{"name":"Custom","needles":["game"],"steam_app_ids":[]}"#)
                .unwrap();
        assert!(!legacy.exact_executable);
        assert_eq!(
            match_game_process(&[legacy], "my-game-client.exe", None),
            Some("Custom")
        );
        let legacy_settings: RecordingSettings =
            serde_json::from_str(r#"{"game_rules":[],"game_only_capture":false}"#).unwrap();
        assert!(legacy_settings.detect_built_in_games);
        assert!(!legacy_settings.game_only_capture);
        let _ = std::fs::remove_file(path);
    }
}

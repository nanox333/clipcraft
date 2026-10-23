//! Minimal, local-only game data. Account identifiers and other players are never retained.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MAX_GAME_STATE_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GameIntegration {
    Cs2,
    Dota2,
    League,
}

impl GameIntegration {
    pub const ALL: [Self; 3] = [Self::Cs2, Self::Dota2, Self::League];
    pub fn label(self) -> &'static str {
        match self {
            Self::Cs2 => "Counter-Strike 2",
            Self::Dota2 => "Dota 2",
            Self::League => "League of Legends",
        }
    }
    pub fn port(self) -> u16 {
        match self {
            Self::Cs2 => 29871,
            Self::Dota2 => 29872,
            Self::League => 2999,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct GameIntegrationSettings {
    pub cs2: bool,
    pub dota2: bool,
    pub league: bool,
}

impl GameIntegrationSettings {
    pub fn enabled(&self, game: GameIntegration) -> bool {
        match game {
            GameIntegration::Cs2 => self.cs2,
            GameIntegration::Dota2 => self.dota2,
            GameIntegration::League => self.league,
        }
    }
}

/// Transient allowlist of useful match information. No names, IDs, credentials or raw JSON.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GameSnapshot {
    pub game: GameIntegration,
    pub map: Option<String>,
    pub mode: Option<String>,
    pub phase: Option<String>,
    pub round: Option<u32>,
    pub team_score: Option<u32>,
    pub opponent_score: Option<u32>,
    pub character: Option<String>,
    pub health: Option<u32>,
    pub armor: Option<u32>,
    pub resource: Option<u32>,
    pub money: Option<u32>,
    pub kills: Option<u32>,
    pub deaths: Option<u32>,
    pub assists: Option<u32>,
    pub weapon: Option<String>,
    pub ammo: Option<u32>,
    pub game_seconds: Option<u32>,
}

impl GameSnapshot {
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        for text in [&self.map, &self.mode, &self.phase, &self.character]
            .into_iter()
            .flatten()
        {
            let value = text
                .strip_prefix("DOTA_GAMERULES_STATE_")
                .or_else(|| text.strip_prefix("npc_dota_hero_"))
                .or_else(|| text.strip_prefix("de_"))
                .or_else(|| text.strip_prefix("cs_"))
                .unwrap_or(text);
            parts.push(
                value
                    .split('_')
                    .map(|word| {
                        let mut chars = word.chars();
                        match chars.next() {
                            Some(first) => {
                                format!("{}{}", first.to_uppercase(), chars.as_str().to_lowercase())
                            }
                            None => String::new(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        if let (Some(a), Some(b)) = (self.team_score, self.opponent_score) {
            parts.push(format!("{a}–{b}"));
        }
        if let (Some(k), Some(d), Some(a)) = (self.kills, self.deaths, self.assists) {
            parts.push(format!("{k}/{d}/{a} K/D/A"));
        }
        if let Some(hp) = self.health {
            parts.push(format!("{hp} HP"));
        }
        if parts.is_empty() {
            "Connected · waiting for match data".into()
        } else {
            parts.join(" · ")
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameStateError {
    Invalid,
    Unavailable,
    ConfigOccupied,
    KeyUnavailable,
    Unsupported,
}
impl std::fmt::Display for GameStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "The game sent incomplete, oversized or invalid match data.",
            Self::Unavailable => "Game data is unavailable. Open a match and check the integration setup.",
            Self::ConfigOccupied => "An integration config already exists and differs. Keep it or rename it before setting up again.",
            Self::KeyUnavailable => "The local game-integration key could not be read or created.",
            Self::Unsupported => "This game integration is not supported on this operating system.",
        })
    }
}
impl std::error::Error for GameStateError {}

fn text(v: &Value) -> Option<String> {
    v.as_str()
        .filter(|s| !s.is_empty() && s.len() <= 96 && !s.chars().any(char::is_control))
        .map(str::to_owned)
}
fn number(v: &Value) -> Option<u32> {
    let n = v.as_f64()?;
    (n.is_finite() && (0.0..=1_000_000_000.0).contains(&n)).then_some(n as u32)
}

/// Parse a full snapshot, never merge omitted fields with an earlier match.
pub fn parse_game_state(
    game: GameIntegration,
    bytes: &[u8],
) -> Result<GameSnapshot, GameStateError> {
    if bytes.len() > MAX_GAME_STATE_BYTES {
        return Err(GameStateError::Invalid);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|_| GameStateError::Invalid)?;
    if !v.is_object() {
        return Err(GameStateError::Invalid);
    }
    let mut s = GameSnapshot {
        game,
        map: None,
        mode: None,
        phase: None,
        round: None,
        team_score: None,
        opponent_score: None,
        character: None,
        health: None,
        armor: None,
        resource: None,
        money: None,
        kills: None,
        deaths: None,
        assists: None,
        weapon: None,
        ammo: None,
        game_seconds: None,
    };
    if game == GameIntegration::League {
        if !v["gameData"].is_object() || !v["activePlayer"].is_object() {
            return Err(GameStateError::Invalid);
        }
        s.map = text(&v["gameData"]["mapName"]);
        s.mode = text(&v["gameData"]["gameMode"]);
        s.game_seconds = number(&v["gameData"]["gameTime"]);
        let active = &v["activePlayer"];
        s.health = number(&active["championStats"]["currentHealth"]);
        s.resource = number(&active["championStats"]["resourceValue"]);
        s.money = number(&active["currentGold"]);
        // Identity is used only for this lookup and discarded. Never choose another player.
        let players = v["allPlayers"]
            .as_array()
            .filter(|p| p.len() <= 64)
            .ok_or(GameStateError::Invalid)?;
        let player = players.iter().find(|p| {
            let riot = active["riotId"].as_str().filter(|s| !s.is_empty());
            let old = active["summonerName"].as_str().filter(|s| !s.is_empty());
            riot.is_some_and(|id| p["riotId"].as_str() == Some(id))
                || (riot.is_none() && old.is_some_and(|id| p["summonerName"].as_str() == Some(id)))
        });
        if let Some(p) = player {
            s.character = text(&p["championName"]);
            s.kills = number(&p["scores"]["kills"]);
            s.deaths = number(&p["scores"]["deaths"]);
            s.assists = number(&p["scores"]["assists"]);
        }
        return Ok(s);
    }
    let app_id = if game == GameIntegration::Cs2 {
        730
    } else {
        570
    };
    if v["provider"]["appid"].as_u64() != Some(app_id) {
        return Err(GameStateError::Invalid);
    }
    let map = &v["map"];
    let player = &v["player"];
    s.map = text(&map["name"]);
    s.mode = text(&map["mode"]);
    s.phase = text(if game == GameIntegration::Cs2 {
        &v["round"]["phase"]
    } else {
        &map["game_state"]
    });
    if s.phase.is_none() {
        s.phase = text(&map["phase"]);
    }
    s.round = number(&map["round"]);
    s.game_seconds = number(&map["clock_time"]);
    let own_player = match (
        v["provider"]["steamid"].as_str(),
        player["steamid"].as_str(),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => game == GameIntegration::Dota2 && player["activity"].as_str() != Some("spectating"),
    };
    // CS2 spectators can receive a different player's data. Do not retain it.
    if own_player {
        let stats = if game == GameIntegration::Cs2 {
            &player["match_stats"]
        } else {
            player
        };
        let state = if game == GameIntegration::Cs2 {
            &player["state"]
        } else {
            &v["hero"]
        };
        s.health = number(&state["health"]);
        s.armor = number(&state["armor"]);
        s.resource = number(&state["mana"]);
        s.money = number(if game == GameIntegration::Cs2 {
            &state["money"]
        } else {
            &player["gold"]
        });
        s.kills = number(&stats["kills"]);
        s.deaths = number(&stats["deaths"]);
        s.assists = number(&stats["assists"]);
        if game == GameIntegration::Dota2 {
            s.character = text(&v["hero"]["name"]);
        }
        if let Some(weapons) = player["weapons"].as_object().filter(|w| w.len() <= 64)
            && let Some(w) = weapons.values().find(|w| w["state"] == "active")
        {
            s.weapon = text(&w["name"]);
            s.ammo = number(&w["ammo_clip"]);
        }
        let (own, other) = match player["team"]
            .as_str()
            .or_else(|| player["team_name"].as_str())
        {
            Some("CT") => (&map["team_ct"]["score"], &map["team_t"]["score"]),
            Some("T") => (&map["team_t"]["score"], &map["team_ct"]["score"]),
            _ => (&Value::Null, &Value::Null),
        };
        s.team_score = number(own);
        s.opponent_score = number(other);
    }
    Ok(s)
}

/// Stored separately from settings and never returned through CLI/diagnostics.
pub fn game_state_key(path: &Path) -> Result<String, GameStateError> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() || meta.len() != 64 {
            return Err(GameStateError::KeyUnavailable);
        }
        let mut bytes = Vec::new();
        fs::File::open(path)
            .and_then(|f| f.take(65).read_to_end(&mut bytes))
            .map_err(|_| GameStateError::KeyUnavailable)?;
        let key = String::from_utf8(bytes).map_err(|_| GameStateError::KeyUnavailable)?;
        return if key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(key)
        } else {
            Err(GameStateError::KeyUnavailable)
        };
    }
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| GameStateError::KeyUnavailable)?;
    let key: String = random.iter().map(|b| format!("{b:02x}")).collect();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| GameStateError::KeyUnavailable)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| GameStateError::KeyUnavailable)?;
    file.write_all(key.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| GameStateError::KeyUnavailable)?;
    Ok(key)
}

pub fn install_game_config(
    game: GameIntegration,
    directory: &Path,
    key_path: &Path,
) -> Result<PathBuf, GameStateError> {
    if game == GameIntegration::League {
        return Err(GameStateError::Unsupported);
    }
    if !directory.is_dir() {
        return Err(GameStateError::Unavailable);
    }
    let key = game_state_key(key_path)?;
    let data = match game {
        GameIntegration::Cs2 => {
            "\"provider\" \"1\"\n\"map\" \"1\"\n\"round\" \"1\"\n\"player_id\" \"1\"\n\"player_state\" \"1\"\n\"player_weapons\" \"1\"\n\"player_match_stats\" \"1\""
        }
        GameIntegration::Dota2 => {
            "\"auth\" \"1\"\n\"provider\" \"1\"\n\"map\" \"1\"\n\"player\" \"1\"\n\"hero\" \"1\""
        }
        GameIntegration::League => return Err(GameStateError::Unsupported),
    };
    let contents = format!(
        "\"ClipCraft local game data\"\n{{\n\"uri\" \"http://127.0.0.1:{}/\"\n\"timeout\" \"1.0\"\n\"buffer\" \"0.1\"\n\"throttle\" \"0.5\"\n\"heartbeat\" \"5.0\"\n\"auth\" {{ \"token\" \"{}\" }}\n\"data\" {{\n{}\n}}\n}}\n",
        game.port(),
        key,
        data
    );
    let path = directory.join("gamestate_integration_clipcraft.cfg");
    if fs::symlink_metadata(&path).is_ok() {
        let mut old = Vec::new();
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file())
            && fs::File::open(&path)
                .and_then(|f| f.take(4097).read_to_end(&mut old))
                .is_ok()
            && old == contents.as_bytes()
        {
            return Ok(path);
        }
        return Err(GameStateError::ConfigOccupied);
    }
    // A complete exclusive temporary is linked into place without replacing existing config.
    let temporary = directory.join("gamestate_integration_clipcraft.cfg.partial");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| GameStateError::ConfigOccupied)?;
    let result = (|| {
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::hard_link(&temporary, &path)
    })();
    let _ = fs::remove_file(&temporary);
    result.map_err(|_| GameStateError::Unavailable)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn parse(game: GameIntegration, v: Value) -> GameSnapshot {
        parse_game_state(game, &serde_json::to_vec(&v).unwrap()).unwrap()
    }

    #[test]
    fn cs2_keeps_only_local_match_fields_and_replaces_omitted_fields() {
        let full = parse(
            GameIntegration::Cs2,
            json!({
                "provider":{"appid":730,"steamid":"private-local"},
                "auth":{"token":"private-key"},
                "map":{"name":"de_mirage","mode":"competitive","round":14,"team_ct":{"score":8},"team_t":{"score":6}},
                "round":{"phase":"live"},
                "player":{"steamid":"private-local","name":"private-name","team":"CT","state":{"health":73,"armor":92,"money":2400},"match_stats":{"kills":18,"deaths":9,"assists":4},"weapons":{"weapon_0":{"name":"weapon_ak47","state":"active","ammo_clip":21}}},
                "allplayers":{"private-opponent":{"position":"private-position"}}
            }),
        );
        assert_eq!(full.map.as_deref(), Some("de_mirage"));
        assert_eq!(
            (full.team_score, full.opponent_score, full.round),
            (Some(8), Some(6), Some(14))
        );
        assert_eq!(
            (full.health, full.kills, full.ammo),
            (Some(73), Some(18), Some(21))
        );
        let stored = serde_json::to_string(&full).unwrap();
        assert!(!stored.contains("private"));
        let empty = parse(GameIntegration::Cs2, json!({"provider":{"appid":730}}));
        assert_eq!(empty.map, None);
        assert_eq!(empty.health, None);
        assert_eq!(empty.kills, None);
        let spectator = parse(
            GameIntegration::Cs2,
            json!({"provider":{"appid":730,"steamid":"local"},"player":{"steamid":"other","state":{"health":40},"match_stats":{"kills":10}}}),
        );
        assert_eq!(spectator.health, None);
        assert_eq!(spectator.kills, None);
    }

    #[test]
    fn dota_local_hero_and_player_do_not_include_nested_spectator_teams() {
        let s = parse(
            GameIntegration::Dota2,
            json!({"provider":{"appid":570},"map":{"game_state":"DOTA_GAMERULES_STATE_GAME_IN_PROGRESS","clock_time":800},"player":{"steamid":"discard","team_name":"radiant","gold":1900,"kills":8,"deaths":2,"assists":11},"hero":{"name":"npc_dota_hero_axe","health":1500,"mana":280}}),
        );
        assert_eq!(s.character.as_deref(), Some("npc_dota_hero_axe"));
        assert!(s.summary().contains("Game In Progress · Axe"));
        assert_eq!(
            (s.health, s.resource, s.money, s.kills, s.game_seconds),
            (Some(1500), Some(280), Some(1900), Some(8), Some(800))
        );
        let spectator = parse(
            GameIntegration::Dota2,
            json!({"provider":{"appid":570},"player":{"team2":{"player0":{"kills":100}}},"hero":{"team2":{"player0":{"health":10}}}}),
        );
        assert_eq!(spectator.kills, None);
        assert_eq!(spectator.health, None);
    }

    #[test]
    fn league_selects_active_player_by_exact_identity_and_discards_accounts_events() {
        for modern in [false, true] {
            let identity = if modern { "riotId" } else { "summonerName" };
            let mut v = json!({"gameData":{"mapName":"Map11","gameMode":"CLASSIC","gameTime":123.75},"activePlayer":{"championStats":{"currentHealth":642.5,"resourceValue":300},"currentGold":500},"allPlayers":[{"championName":"Other","scores":{"kills":99}},{"championName":"Annie","scores":{"kills":4,"deaths":2,"assists":7}}],"events":{"Events":[{"KillerName":"private"}]}});
            v["activePlayer"][identity] = json!("local-private");
            v["allPlayers"][0][identity] = json!("other-private");
            v["allPlayers"][1][identity] = json!("local-private");
            let s = parse(GameIntegration::League, v.clone());
            assert_eq!(s.character.as_deref(), Some("Annie"));
            assert_eq!(s.kills, Some(4));
            assert_eq!(s.health, Some(642));
            assert!(!serde_json::to_string(&s).unwrap().contains("private"));
            v["activePlayer"][identity] = json!("unknown");
            let missing = parse(GameIntegration::League, v);
            assert_eq!(missing.character, None);
            assert_eq!(missing.kills, None);
        }
    }

    #[test]
    fn malformed_hostile_unknown_and_oversized_data_is_bounded() {
        for bytes in [
            b"null".as_slice(),
            b"[]",
            b"{",
            b"{}",
            b"{\"provider\":{\"appid\":570}}",
        ] {
            assert_eq!(
                parse_game_state(GameIntegration::Cs2, bytes),
                Err(GameStateError::Invalid)
            );
        }
        assert!(parse_game_state(GameIntegration::League, b"{}").is_err());
        assert!(
            parse_game_state(GameIntegration::Cs2, &vec![b' '; MAX_GAME_STATE_BYTES + 1]).is_err()
        );
        let v = json!({"provider":{"appid":730,"steamid":"a"},"map":{"name":"bad\nmap","round":-1},"player":{"steamid":"a","state":{"health":1e100,"money":-50},"match_stats":{"kills":"99"}}});
        let s = parse(GameIntegration::Cs2, v);
        assert_eq!(s.map, None);
        assert_eq!(s.health, None);
        assert_eq!(s.money, None);
        assert_eq!(s.kills, None);
        assert_eq!(s.round, None);
        let nested = format!("{}0{}", "[".repeat(150), "]".repeat(150));
        assert!(parse_game_state(GameIntegration::Cs2, nested.as_bytes()).is_err());
    }

    #[test]
    fn config_setup_is_repeatable_exclusive_and_secret_is_separate() {
        let root = std::env::temp_dir().join(format!(
            "clipcraft-gsi-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let cfg = root.join("cfg");
        fs::create_dir(&cfg).unwrap();
        let key_path = root.join("state/key");
        let path = install_game_config(GameIntegration::Cs2, &cfg, &key_path).unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        let key = game_state_key(&key_path).unwrap();
        assert_eq!(key.len(), 64);
        assert!(contents.contains(&key));
        assert!(contents.contains("http://127.0.0.1:29871/"));
        assert_eq!(
            install_game_config(GameIntegration::Cs2, &cfg, &key_path),
            Ok(path.clone())
        );
        fs::write(&path, b"user config").unwrap();
        assert_eq!(
            install_game_config(GameIntegration::Cs2, &cfg, &key_path),
            Err(GameStateError::ConfigOccupied)
        );
        assert_eq!(fs::read(&path).unwrap(), b"user config");
        fs::remove_file(&path).unwrap();
        let partial = cfg.join("gamestate_integration_clipcraft.cfg.partial");
        fs::write(&partial, b"another writer").unwrap();
        assert_eq!(
            install_game_config(GameIntegration::Dota2, &cfg, &key_path),
            Err(GameStateError::ConfigOccupied)
        );
        assert_eq!(fs::read(&partial).unwrap(), b"another writer");
        fs::remove_file(&partial).unwrap();
        install_game_config(GameIntegration::Dota2, &cfg, &key_path).unwrap();
        assert!(fs::read_to_string(path).unwrap().contains("\"auth\" \"1\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&key_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(
            install_game_config(GameIntegration::League, &cfg, &key_path),
            Err(GameStateError::Unsupported)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn integration_defaults_keep_existing_settings_off_and_commands_are_registered() {
        let settings: crate::RecordingSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(
            settings.game_integrations,
            GameIntegrationSettings::default()
        );
        assert!(
            GameIntegration::ALL
                .into_iter()
                .all(|g| !settings.game_integrations.enabled(g))
        );
        for id in ["game_state.parse", "game_state.install"] {
            assert!(crate::command_ids().contains(&id));
        }
        let command = crate::Command::ParseGameState {
            game: GameIntegration::Dota2,
            bytes: br#"{"provider":{"appid":570}}"#.to_vec(),
        };
        assert_eq!(command.id(), "game_state.parse");
        assert!(matches!(
            crate::execute(command),
            Ok(crate::CommandOutput::GameState(_))
        ));
    }
}

//! Bounded local Steam KeyValues metadata. No store API, downloaded catalog, or game-file reads.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::{EngineError, RecordingSettings, built_in_games};

pub const MAX_STEAM_GAMES: usize = 1024;
const MAX_LIBRARIES: usize = 64;
const MAX_MANIFESTS: usize = 4096;
const MAX_VDF_BYTES: u64 = 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SteamGame {
    pub app_id: String,
    pub name: String,
    pub install_directory: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SteamGames {
    pub games: Vec<SteamGame>,
    /// Unreadable/malformed individual manifests or unavailable additional libraries.
    pub skipped: usize,
}

impl RecordingSettings {
    /// Custom/bundled recognition first; installed metadata only fills unknown exact Steam IDs.
    pub fn match_game_with_steam<'a>(
        &'a self,
        executable: &str,
        app_id: Option<&str>,
        installed: &'a [SteamGame],
    ) -> Option<&'a str> {
        // Explicit custom rules are allowed to override exclusions.
        if let Some(name) = crate::match_game_process(&self.game_rules, executable, app_id) {
            return Some(name);
        }
        if app_id.is_some_and(|id| {
            self.disabled_steam_app_ids
                .iter()
                .any(|disabled| disabled == id)
        }) {
            return None;
        }
        if let Some(name) = self.match_game(executable, app_id) {
            return Some(name);
        }
        if !self.detect_built_in_games || !self.detect_installed_steam_games {
            return None;
        }
        // Installed manifests cannot bypass a deliberately excluded built-in entry.
        if built_in_games().iter().any(|game| {
            self.disabled_builtin_games.iter().any(|id| id == game.id)
                && (app_id.is_some_and(|id| game.steam_app_ids.contains(&id))
                    || game
                        .executables
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(executable)))
        }) {
            return None;
        }
        let id = app_id?;
        installed
            .iter()
            .find(|game| game.app_id == id)
            .map(|game| game.name.as_str())
    }
}

#[derive(Debug)]
enum Value {
    Text(String),
    Object(Vec<(String, Value)>),
}
impl Value {
    fn get(&self, key: &str) -> Option<&Value> {
        let Self::Object(fields) = self else {
            return None;
        };
        let mut matches = fields
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case(key));
        let (_, value) = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(value)
    }
    fn text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            _ => None,
        }
    }
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
    tokens: usize,
}
impl Parser<'_> {
    fn whitespace(&mut self) {
        loop {
            while self
                .input
                .get(self.position)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.position += 1;
            }
            if self
                .input
                .get(self.position..self.position.saturating_add(2))
                == Some(b"//")
            {
                while self
                    .input
                    .get(self.position)
                    .is_some_and(|byte| *byte != b'\n')
                {
                    self.position += 1;
                }
            } else {
                break;
            }
        }
    }
    fn string(&mut self) -> Result<String, EngineError> {
        self.tokens += 1;
        if self.tokens > 100_000 {
            return Err(EngineError::SteamMetadataInvalid);
        }
        self.whitespace();
        let start = self.position;
        if self.input.get(start) != Some(&b'"') {
            return Err(EngineError::SteamMetadataInvalid);
        }
        self.position += 1;
        let mut bytes = Vec::new();
        while let Some(&byte) = self.input.get(self.position) {
            self.position += 1;
            match byte {
                b'"' => {
                    return String::from_utf8(bytes).map_err(|_| EngineError::SteamMetadataInvalid);
                }
                b'\\' => {
                    let escaped = *self
                        .input
                        .get(self.position)
                        .ok_or(EngineError::SteamMetadataInvalid)?;
                    self.position += 1;
                    bytes.push(match escaped {
                        b'\\' | b'"' => escaped,
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'r' => b'\r',
                        _ => return Err(EngineError::SteamMetadataInvalid),
                    });
                }
                0 => return Err(EngineError::SteamMetadataInvalid),
                _ => bytes.push(byte),
            }
        }
        Err(EngineError::SteamMetadataInvalid)
    }
    fn object(&mut self, depth: usize, closing: bool) -> Result<Value, EngineError> {
        if depth > 12 {
            return Err(EngineError::SteamMetadataInvalid);
        }
        let mut fields = Vec::new();
        loop {
            self.whitespace();
            match self.input.get(self.position) {
                None if !closing => return Ok(Value::Object(fields)),
                Some(b'}') if closing => {
                    self.position += 1;
                    return Ok(Value::Object(fields));
                }
                None | Some(b'}') => return Err(EngineError::SteamMetadataInvalid),
                _ => {}
            }
            let key = self.string()?;
            self.whitespace();
            let value = if self.input.get(self.position) == Some(&b'{') {
                self.position += 1;
                self.object(depth + 1, true)?
            } else {
                Value::Text(self.string()?)
            };
            fields.push((key, value));
        }
    }
}

fn read_vdf(path: &Path, limit: u64) -> Result<Value, EngineError> {
    let file = fs::File::open(path).map_err(|_| EngineError::SteamMetadataInvalid)?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| EngineError::SteamMetadataInvalid)?;
    if bytes.len() as u64 > limit {
        return Err(EngineError::SteamMetadataInvalid);
    }
    Parser {
        input: &bytes,
        position: 0,
        tokens: 0,
    }
    .object(0, false)
}

pub(crate) fn valid_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id != "0" && id.bytes().all(|byte| byte.is_ascii_digit())
}

/// Scan selected Steam installation roots, including libraries listed in libraryfolders.vdf.
/// Invalid individual manifests are counted and skipped; invalid library indexes or exceeded
/// whole-scan budgets return an error so the caller can retain its previous catalog.
pub(crate) fn scan(roots: &[PathBuf], cancel: &AtomicBool) -> Result<SteamGames, EngineError> {
    if roots.len() > MAX_LIBRARIES {
        return Err(EngineError::SteamMetadataInvalid);
    }
    let mut result = SteamGames::default();
    let mut libraries = Vec::new();
    for root in roots {
        if !root.is_absolute() || root.as_os_str().len() > 4096 {
            return Err(EngineError::SteamMetadataInvalid);
        }
        if !root.exists() {
            continue;
        }
        let root = fs::canonicalize(root).map_err(|_| EngineError::SteamMetadataInvalid)?;
        if !libraries.contains(&root) {
            libraries.push(root.clone());
        }
        let path = root.join("steamapps/libraryfolders.vdf");
        if path.exists() {
            let vdf = read_vdf(&path, MAX_VDF_BYTES)?;
            let Value::Object(entries) = vdf
                .get("libraryfolders")
                .ok_or(EngineError::SteamMetadataInvalid)?
            else {
                return Err(EngineError::SteamMetadataInvalid);
            };
            for (index, value) in entries {
                if !index.bytes().all(|byte| byte.is_ascii_digit()) || index.is_empty() {
                    continue;
                }
                let path = value
                    .text()
                    .or_else(|| value.get("path").and_then(Value::text))
                    .ok_or(EngineError::SteamMetadataInvalid)?;
                if path.len() > 4096
                    || path.chars().any(char::is_control)
                    || !Path::new(path).is_absolute()
                {
                    return Err(EngineError::SteamMetadataInvalid);
                }
                match fs::canonicalize(path) {
                    Ok(path) if !libraries.contains(&path) => libraries.push(path),
                    Ok(_) => {}
                    Err(_) => result.skipped += 1,
                }
                if libraries.len() > MAX_LIBRARIES {
                    return Err(EngineError::SteamMetadataInvalid);
                }
            }
        }
    }
    let mut games = BTreeMap::new();
    let mut visited = 0;
    for root in libraries {
        if cancel.load(Ordering::Relaxed) {
            return Err(EngineError::ScanCancelled);
        }
        let steamapps = root.join("steamapps");
        let entries = match fs::read_dir(&steamapps) {
            Ok(entries) => entries,
            Err(_) => {
                result.skipped += 1;
                continue;
            }
        };
        for entry in entries {
            visited += 1;
            if visited > MAX_MANIFESTS {
                return Err(EngineError::SteamMetadataInvalid);
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(EngineError::ScanCancelled);
            }
            let entry = entry.map_err(|_| EngineError::SteamMetadataInvalid)?;
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            let Some(id) = file_name
                .strip_prefix("appmanifest_")
                .and_then(|name| name.strip_suffix(".acf"))
            else {
                continue;
            };
            if !valid_app_id(id) {
                result.skipped += 1;
                continue;
            }
            let parsed = (|| {
                if entry.file_type().ok()?.is_symlink() {
                    return None;
                }
                let vdf = read_vdf(&entry.path(), MAX_MANIFEST_BYTES).ok()?;
                let app = vdf.get("AppState")?;
                if app.get("appid")?.text()? != id {
                    return None;
                }
                let name = app.get("name")?.text()?;
                if name.trim().is_empty() || name.len() > 100 || name.chars().any(char::is_control)
                {
                    return None;
                }
                let directory = app.get("installdir")?.text()?;
                if directory.is_empty()
                    || directory.len() > 512
                    || directory.contains(['/', '\\'])
                    || directory.chars().any(char::is_control)
                    || !Path::new(directory)
                        .components()
                        .all(|part| matches!(part, Component::Normal(_)))
                {
                    return None;
                }
                let common = fs::canonicalize(steamapps.join("common")).ok()?;
                let install_directory = fs::canonicalize(common.join(directory)).ok()?;
                if install_directory == common
                    || !install_directory.starts_with(&common)
                    || !install_directory.is_dir()
                {
                    return None;
                }
                Some(SteamGame {
                    app_id: id.to_owned(),
                    name: name.trim().to_owned(),
                    install_directory,
                })
            })();
            match parsed {
                Some(game) => {
                    games.entry(game.app_id.clone()).or_insert(game);
                }
                None => result.skipped += 1,
            }
            if games.len() > MAX_STEAM_GAMES {
                return Err(EngineError::SteamMetadataInvalid);
            }
        }
    }
    result.games = games.into_values().collect();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, CommandOutput, DesktopCaptureMode, GameMatchRule, execute};
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn temp() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "clipcraft-steam-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("steamapps/common")).unwrap();
        root
    }
    fn manifest(root: &Path, id: &str, name: &str) {
        fs::create_dir_all(root.join("steamapps/common").join(name)).unwrap();
        fs::write(
            root.join(format!("steamapps/appmanifest_{id}.acf")),
            format!(r#""AppState" {{ "appid" "{id}" "name" "{name}" "installdir" "{name}" }}"#),
        )
        .unwrap();
    }
    #[test]
    fn discovers_modern_and_legacy_alternate_libraries_and_ignores_duplicate_roots() {
        for modern in [true, false] {
            let root = temp();
            let other = temp();
            manifest(&root, "999111", "Synthetic one");
            manifest(&other, "999222", "Synthetic two");
            let path = other.to_str().unwrap().replace('\\', "\\\\");
            let entry = if modern {
                format!(r#""1" {{ "path" "{path}" "apps" {{ "999222" "100" }} }}"#)
            } else {
                format!(r#""1" "{path}""#)
            };
            fs::write(
                root.join("steamapps/libraryfolders.vdf"),
                format!(
                    r#"// comment
"libraryfolders" {{ {entry} }}"#
                ),
            )
            .unwrap();
            let CommandOutput::SteamGames(found) = execute(Command::ScanSteamGames {
                roots: vec![root.clone(), root.clone()],
            })
            .unwrap() else {
                panic!("wrong output")
            };
            assert_eq!(
                found
                    .games
                    .iter()
                    .map(|game| game.app_id.as_str())
                    .collect::<Vec<_>>(),
                ["999111", "999222"]
            );
            assert_eq!(found.skipped, 0);
            fs::remove_dir_all(root).unwrap();
            fs::remove_dir_all(other).unwrap();
        }
    }
    #[test]
    fn corrupted_oversized_traversal_mismatched_and_uninstalled_entries_never_become_games() {
        let root = temp();
        manifest(&root, "999111", "Valid");
        for (id, body) in [
            (
                "1",
                r#""AppState" {"appid" "1" "name" "Bad" "installdir" "../escape"}"#.to_owned(),
            ),
            (
                "2",
                r#""AppState" {"appid" "3" "name" "Bad" "installdir" "Valid"}"#.to_owned(),
            ),
            (
                "3",
                r#""AppState" {"appid" "3" "name" "Bad" "name" "Duplicate" "installdir" "Valid"}"#
                    .to_owned(),
            ),
            ("4", "x".repeat(MAX_MANIFEST_BYTES as usize + 1)),
            (
                "5",
                r#""AppState" {"appid" "5" "name" "Not installed" "installdir" "Absent"}"#
                    .to_owned(),
            ),
        ] {
            fs::write(root.join(format!("steamapps/appmanifest_{id}.acf")), body).unwrap();
        }
        let found = scan(std::slice::from_ref(&root), &AtomicBool::new(false)).unwrap();
        assert_eq!(found.games.len(), 1);
        assert_eq!(found.skipped, 5);
        fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            vec![b'x'; MAX_VDF_BYTES as usize + 1],
        )
        .unwrap();
        assert_eq!(
            scan(std::slice::from_ref(&root), &AtomicBool::new(false)),
            Err(EngineError::SteamMetadataInvalid)
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn parser_rejects_truncation_bad_utf8_escapes_depth_and_root_limits_and_cancels() {
        for bytes in [
            &b"\"x\" {"[..],
            &b"\"x\" \"\xff\""[..],
            &b"\"x\" \"a\\q\""[..],
            &b"}"[..],
        ] {
            assert!(
                Parser {
                    input: bytes,
                    position: 0,
                    tokens: 0
                }
                .object(0, false)
                .is_err()
            );
        }
        let deep = format!("{}{}", "\"x\" {".repeat(14), "}".repeat(14));
        assert!(
            Parser {
                input: deep.as_bytes(),
                position: 0,
                tokens: 0
            }
            .object(0, false)
            .is_err()
        );
        let root = temp();
        assert_eq!(
            scan(&vec![root.clone(); 65], &AtomicBool::new(false)),
            Err(EngineError::SteamMetadataInvalid)
        );
        assert_eq!(
            scan(std::slice::from_ref(&root), &AtomicBool::new(true)),
            Err(EngineError::ScanCancelled)
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn exact_installed_ids_respect_custom_priority_built_in_exclusions_global_off_and_steam_exclusions()
     {
        let games = vec![
            SteamGame {
                app_id: "999111".into(),
                name: "New installed game".into(),
                install_directory: "/games/new".into(),
            },
            SteamGame {
                app_id: "730".into(),
                name: "CS".into(),
                install_directory: "/games/cs".into(),
            },
        ];
        let mut settings = RecordingSettings::default();
        assert_eq!(
            settings.match_game_with_steam("generic", Some("999111"), &games),
            Some("New installed game")
        );
        assert_eq!(
            settings.match_game_with_steam("generic", Some("9991110"), &games),
            None
        );
        settings.disabled_builtin_games.push("cs2".into());
        assert_eq!(
            settings.match_game_with_steam("generic", Some("730"), &games),
            None
        );
        settings.disabled_steam_app_ids.push("999111".into());
        assert_eq!(
            settings.match_game_with_steam("generic", Some("999111"), &games),
            None
        );
        settings
            .game_rules
            .push(GameMatchRule::new("My label", ["generic"]));
        assert_eq!(
            settings.match_game_with_steam("generic", Some("999111"), &games),
            Some("My label")
        );
        settings.game_rules.clear();
        settings.disabled_steam_app_ids.clear();
        settings.detect_built_in_games = false;
        assert_eq!(
            settings.match_game_with_steam("generic", Some("999111"), &games),
            None
        );
        let old: RecordingSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.desktop_capture_mode, DesktopCaptureMode::Never);
        assert_eq!(old.desktop_after_game_seconds, 60);
        let bounded = RecordingSettings {
            desktop_after_game_seconds: u32::MAX,
            disabled_steam_app_ids: vec!["0".into(), "bad".into(), "730".into(), "730".into()],
            ..Default::default()
        }
        .sanitized();
        assert_eq!(bounded.desktop_after_game_seconds, 3600);
        assert_eq!(bounded.disabled_steam_app_ids, ["730"]);
    }
}

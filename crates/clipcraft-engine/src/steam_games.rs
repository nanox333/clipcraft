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
                    if !matches!(escaped, b'\\' | b'"') {
                        return Err(EngineError::SteamMetadataInvalid);
                    }
                    bytes.push(escaped);
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
/// A partial/limited discovery is reported as an error rather than silently replacing a catalog.
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

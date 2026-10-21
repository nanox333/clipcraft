//! Local process metadata used for game-aware replay arming.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// A running program identified without retaining its full command line or executable path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunningProcess {
    /// Process identifier for matching the game to a capturable window.
    pub process_id: u32,
    /// Executable file name, or the operating system's short process name when unavailable.
    pub executable: String,
    /// Steam's exact numeric application ID when available on Linux and explicitly requested.
    pub steam_app_id: Option<String>,
}

/// Reuses the process table between scans so repeated game checks do not rebuild system state.
pub struct ProcessObserver {
    system: System,
    steam_app_ids: HashMap<u32, (u64, String, Option<String>)>,
}

impl Default for ProcessObserver {
    fn default() -> Self {
        Self {
            system: System::new(),
            steam_app_ids: HashMap::new(),
        }
    }
}

impl ProcessObserver {
    /// Refreshes process names and executable basenames, with optional exact Steam IDs on Linux.
    pub fn refresh(&mut self, include_steam_app_ids: bool) -> Vec<RunningProcess> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_exe(UpdateKind::OnlyIfNotSet),
        );

        let mut processes = Vec::new();
        let mut live_process_ids = HashSet::new();
        for process in self.system.processes().values() {
            let executable = process
                .exe()
                .and_then(Path::file_name)
                .unwrap_or_else(|| process.name())
                .to_string_lossy()
                .trim()
                .to_owned();
            if executable.is_empty() {
                continue;
            }
            let process_id = process.pid().as_u32();
            live_process_ids.insert(process_id);
            #[cfg(target_os = "linux")]
            let steam_app_id = if include_steam_app_ids {
                let start_time = process.start_time();
                match self.steam_app_ids.get(&process_id) {
                    Some((cached_start_time, cached_executable, id))
                        if *cached_start_time == start_time && cached_executable == &executable =>
                    {
                        id.clone()
                    }
                    _ => {
                        let id = steam_app_id(process_id);
                        self.steam_app_ids
                            .insert(process_id, (start_time, executable.clone(), id.clone()));
                        id
                    }
                }
            } else {
                None
            };
            #[cfg(not(target_os = "linux"))]
            let steam_app_id = {
                let _ = include_steam_app_ids;
                None
            };
            processes.push(RunningProcess {
                process_id,
                executable,
                steam_app_id,
            });
        }
        self.steam_app_ids
            .retain(|process_id, _| live_process_ids.contains(process_id));
        processes.sort_by(|left, right| {
            left.executable
                .to_lowercase()
                .cmp(&right.executable.to_lowercase())
                .then_with(|| left.process_id.cmp(&right.process_id))
        });
        processes.truncate(16_384);
        processes
    }
}

/// Lists running programs. Steam environment variables are read only when at least one configured
/// rule needs them, and only their numeric Steam ID is retained; the rest of each environment is
/// discarded while parsing.
pub fn running_processes(include_steam_app_ids: bool) -> Vec<RunningProcess> {
    ProcessObserver::default().refresh(include_steam_app_ids)
}

#[cfg(target_os = "linux")]
fn steam_app_id(process_id: u32) -> Option<String> {
    use std::io::Read;

    const MAX_ENVIRONMENT_BYTES: u64 = 1024 * 1024;
    let path = format!("/proc/{process_id}/environ");
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_ENVIRONMENT_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    steam_app_id_from_environment(&bytes)
}

#[cfg(target_os = "linux")]
fn steam_app_id_from_environment(environment: &[u8]) -> Option<String> {
    let mut fallback = None;
    for entry in environment.split(|byte| *byte == 0) {
        let Some(index) = entry.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let (name, rest) = entry.split_at(index);
        let value = &rest[1..];
        if name != b"SteamAppId" && name != b"SteamGameId" {
            continue;
        }
        if value.is_empty() || value.len() > 20 || !value.iter().all(u8::is_ascii_digit) {
            continue;
        }
        let id = std::str::from_utf8(value).ok()?;
        if id == "0" {
            continue;
        }
        if name == b"SteamAppId" {
            return Some(id.to_owned());
        }
        fallback.get_or_insert_with(|| id.to_owned());
    }
    fallback
}

#[cfg(target_os = "linux")]
#[cfg(test)]
mod tests {
    use super::steam_app_id_from_environment;

    #[test]
    fn extracts_only_steam_ids_and_prefers_steam_app_id() {
        assert_eq!(
            steam_app_id_from_environment(b"TOKEN=private\0SteamGameId=440\0SteamAppId=570\0"),
            Some("570".to_owned())
        );
        assert_eq!(
            steam_app_id_from_environment(b"SteamGameId=440\0"),
            Some("440".to_owned())
        );
    }

    #[test]
    fn rejects_invalid_and_zero_steam_ids() {
        for bytes in [
            b"SteamAppId=0\0".as_slice(),
            b"SteamAppId=12x\0".as_slice(),
            b"SteamAppId=123456789012345678901\0".as_slice(),
            b"STEAMAPPID=570\0".as_slice(),
        ] {
            assert_eq!(steam_app_id_from_environment(bytes), None);
        }
    }
}

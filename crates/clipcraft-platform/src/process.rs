//! Local process metadata used for game-aware replay arming.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

/// A running program identified without retaining its full command line or executable path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunningProcess {
    /// Process identifier for matching the game to a capturable window.
    pub process_id: u32,
    /// Executable file name, or the operating system's short process name when unavailable.
    pub executable: String,
    /// Exact Steam ID from Linux process metadata or a verified local installation path.
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
        self.refresh_with_installations(include_steam_app_ids, &[])
    }

    /// Installation paths are only compared locally; full process paths are never returned.
    pub fn refresh_with_installations(
        &mut self,
        include_steam_app_ids: bool,
        installations: &[(String, PathBuf)],
    ) -> Vec<RunningProcess> {
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
            // An exited child may remain in the OS table until its parent reaps it.
            // It no longer has a capturable window and must not keep game capture armed.
            if process.status() == ProcessStatus::Zombie {
                continue;
            }
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
            let lower = executable.to_lowercase();
            let launcher = lower.contains("launcher")
                || lower.contains("crashreport")
                || lower.contains("crashhandler")
                || lower.starts_with("unins")
                || lower.starts_with("steamwebhelper")
                || lower == "playgtav.exe"
                || lower == "leagueclient.exe";
            let steam_app_id = if include_steam_app_ids && !launcher {
                steam_app_id.or_else(|| {
                    process.exe().and_then(|path| {
                        steam_id_for_path(path, installations, cfg!(windows)).map(str::to_owned)
                    })
                })
            } else {
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

/// Default installation roots only; libraryfolders.vdf supplies additional libraries.
/// Called on the discovery worker, not on the interface thread.
pub fn steam_installation_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "linux")]
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.extend([
            home.join(".steam/steam"),
            home.join(".local/share/Steam"),
            home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        ]);
    }
    #[cfg(windows)]
    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(path) = std::env::var_os(variable) {
            roots.push(PathBuf::from(path).join("Steam"));
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Library/Application Support/Steam"));
    }
    roots
}

fn steam_id_for_path<'a>(
    path: &Path,
    installations: &'a [(String, PathBuf)],
    windows: bool,
) -> Option<&'a str> {
    fn normalize(path: &Path, windows: bool) -> Option<String> {
        let text = path.to_str()?;
        if windows {
            Some(text.replace('\\', "/").to_lowercase())
        } else {
            Some(text.to_owned())
        }
    }
    let path = normalize(path, windows)?;
    installations
        .iter()
        .filter_map(|(id, directory)| {
            let directory = normalize(directory, windows)?;
            let directory = directory.trim_end_matches('/');
            path.strip_prefix(directory)
                .filter(|rest| rest.starts_with('/'))
                .map(|_| (directory.len(), id.as_str()))
        })
        .max_by_key(|(length, _)| *length)
        .map(|(_, id)| id)
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
    fn exited_unreaped_process_is_not_a_running_game() {
        use std::time::{Duration, Instant};
        const CHILD_FLAG: &str = "CLIPCRAFT_TEST_UNREAPED_CHILD";
        if std::env::var_os(CHILD_FLAG).is_some() {
            std::thread::sleep(Duration::from_secs(30));
            return;
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::tests::exited_unreaped_process_is_not_a_running_game",
            ])
            .env(CHILD_FLAG, "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut observer = super::ProcessObserver::default();
        assert!(observer.refresh(false).iter().any(|p| p.process_id == pid));
        child.kill().unwrap();
        // Deliberately leave the child unreaped while observing its exit.
        let deadline = Instant::now() + Duration::from_secs(3);
        let absent = loop {
            if !observer.refresh(false).iter().any(|p| p.process_id == pid) {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        child.wait().unwrap();
        assert!(absent, "an exited unreaped child kept game capture armed");
    }

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

#[cfg(test)]
mod installation_tests {
    use super::*;
    #[test]
    fn installation_matching_respects_path_boundaries_and_windows_case() {
        let installs = vec![
            ("100".into(), PathBuf::from("/games/Game")),
            ("200".into(), PathBuf::from("/games/Game/Second")),
        ];
        assert_eq!(
            steam_id_for_path(Path::new("/games/Game/bin/game"), &installs, false),
            Some("100")
        );
        assert_eq!(
            steam_id_for_path(Path::new("/games/Game/Second/game"), &installs, false),
            Some("200")
        );
        assert_eq!(
            steam_id_for_path(Path::new("/games/Game2/game"), &installs, false),
            None
        );
        assert_eq!(
            steam_id_for_path(Path::new("/games/game/game"), &installs, false),
            None
        );
        let installs = vec![(
            "300".into(),
            PathBuf::from(r"C:\SteamLibrary\steamapps\common\MyGame"),
        )];
        assert_eq!(
            steam_id_for_path(
                Path::new(r"c:\steamlibrary\SteamApps\common\mygame\game.exe"),
                &installs,
                true
            ),
            Some("300")
        );
        assert_eq!(
            steam_id_for_path(
                Path::new(r"c:\steamlibrary\SteamApps\common\mygame2\game.exe"),
                &installs,
                true
            ),
            None
        );
    }
}

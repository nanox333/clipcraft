//! One discovery thread, one replaceable pending request and one replaceable result.

use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};

use clipcraft_engine::{
    Command, CommandOutput, GameProcessMatch, RecordingSettings, SteamGames, execute,
};
use clipcraft_platform::{ProcessObserver, RunningProcess};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DetectionConfig {
    pub automatic: bool,
    pub installed: bool,
    pub disabled: Vec<String>,
    pub disabled_steam: Vec<String>,
    pub custom: Vec<clipcraft_engine::GameMatchRule>,
}
impl From<&RecordingSettings> for DetectionConfig {
    fn from(settings: &RecordingSettings) -> Self {
        Self {
            automatic: settings.detect_built_in_games,
            installed: settings.detect_installed_steam_games,
            disabled: settings.disabled_builtin_games.clone(),
            disabled_steam: settings.disabled_steam_app_ids.clone(),
            custom: settings.game_rules.clone(),
        }
    }
}
impl DetectionConfig {
    fn settings(&self) -> RecordingSettings {
        RecordingSettings {
            detect_built_in_games: self.automatic,
            detect_installed_steam_games: self.installed,
            disabled_builtin_games: self.disabled.clone(),
            disabled_steam_app_ids: self.disabled_steam.clone(),
            game_rules: self.custom.clone(),
            ..Default::default()
        }
    }
}

pub(crate) struct ResultSet {
    pub processes: Vec<RunningProcess>,
    pub matches: Vec<GameProcessMatch>,
    pub steam: SteamGames,
    pub steam_warning: Option<String>,
}

pub(crate) struct Job {
    pub config: DetectionConfig,
    pub receiver: mpsc::Receiver<Result<ResultSet, String>>,
}
struct Request {
    config: DetectionConfig,
    force_steam: bool,
    context: egui::Context,
    sender: mpsc::SyncSender<Result<ResultSet, String>>,
}
#[derive(Default)]
struct Mailbox {
    pending: Option<Request>,
    stopped: bool,
}

pub(crate) struct Worker {
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    thread: std::thread::JoinHandle<()>,
}
use eframe::egui;

impl Worker {
    pub fn start() -> Result<Self, String> {
        let mut observer = ProcessObserver::default();
        let mut steam = SteamGames::default();
        let mut steam_checked = None::<Instant>;
        let mut steam_warning = None;
        Self::with_scanner(move |config, force| {
            let settings = config.settings();
            let include_steam =
                settings.detect_built_in_games && settings.detect_installed_steam_games;
            if include_steam
                && (force || steam_checked.is_none_or(|at| at.elapsed() >= Duration::from_secs(30)))
            {
                steam_checked = Some(Instant::now());
                match execute(Command::ScanSteamGames {
                    roots: clipcraft_platform::steam_installation_roots(),
                }) {
                    Ok(CommandOutput::SteamGames(found)) => {
                        steam = found;
                        steam_warning = None;
                    }
                    Ok(_) => {
                        steam_warning =
                            Some("Steam discovery returned an unexpected result.".to_owned())
                    }
                    Err(error) => steam_warning = Some(error.to_string()),
                }
            }
            let installations: Vec<_> = if include_steam {
                steam
                    .games
                    .iter()
                    .map(|game| (game.app_id.clone(), game.install_directory.clone()))
                    .collect()
            } else {
                Vec::new()
            };
            let include_ids = include_steam || settings.needs_steam_game_ids();
            let processes = observer.refresh_with_installations(include_ids, &installations);
            if processes.len() >= 16_384 {
                return Err(
                    "Too many processes to determine whether the game exited safely.".to_owned(),
                );
            }
            let matches = processes
                .iter()
                .filter(|process| process.process_id != std::process::id())
                .filter_map(|process| {
                    settings
                        .match_game_with_steam(
                            &process.executable,
                            process.steam_app_id.as_deref(),
                            &steam.games,
                        )
                        .map(|name| GameProcessMatch {
                            game_name: name.to_owned(),
                            process_id: process.process_id,
                        })
                })
                .collect();
            Ok(ResultSet {
                processes,
                matches,
                steam: if include_steam {
                    steam.clone()
                } else {
                    SteamGames::default()
                },
                steam_warning: steam_warning.clone(),
            })
        })
    }

    fn with_scanner(
        mut scan: impl FnMut(&DetectionConfig, bool) -> Result<ResultSet, String> + Send + 'static,
    ) -> Result<Self, String> {
        let mailbox = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let worker_mailbox = Arc::clone(&mailbox);
        let thread = std::thread::Builder::new()
            .name("game-discovery".into())
            .spawn(move || {
                loop {
                    let (lock, ready) = &*worker_mailbox;
                    let Ok(mut state) = lock.lock() else { return };
                    while state.pending.is_none() && !state.stopped {
                        state = match ready.wait(state) {
                            Ok(state) => state,
                            Err(_) => return,
                        };
                    }
                    if state.stopped {
                        return;
                    }
                    let request = state.pending.take();
                    drop(state);
                    if let Some(request) = request {
                        let result = scan(&request.config, request.force_steam);
                        let _ = request.sender.send(result);
                        request.context.request_repaint();
                    }
                }
            })
            .map_err(|_| "Game discovery could not start. Try again.".to_owned())?;
        Ok(Self { mailbox, thread })
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    #[cfg(test)]
    pub(crate) fn fake(
        scanner: impl FnMut(&DetectionConfig, bool) -> Result<ResultSet, String> + Send + 'static,
    ) -> Self {
        Self::with_scanner(scanner).unwrap()
    }

    pub fn submit(
        &self,
        config: DetectionConfig,
        force_steam: bool,
        context: &egui::Context,
    ) -> Result<Job, String> {
        if self.thread.is_finished() {
            return Err("Game discovery stopped unexpectedly. Try again.".to_owned());
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let (lock, ready) = &*self.mailbox;
        let mut state = lock
            .lock()
            .map_err(|_| "Game discovery stopped unexpectedly.".to_owned())?;
        state.pending = Some(Request {
            config: config.clone(),
            force_steam,
            context: context.clone(),
            sender,
        });
        ready.notify_one();
        Ok(Job { config, receiver })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let (lock, ready) = &*self.mailbox;
        if let Ok(mut state) = lock.lock() {
            state.stopped = true;
            state.pending = None;
            ready.notify_one();
        }
        // A blocked OS call cannot be joined on the UI thread. No new worker is created while
        // this one runs; a finished/panicked worker is detected by submit/receiver disconnection.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn empty() -> ResultSet {
        ResultSet {
            processes: Vec::new(),
            matches: Vec::new(),
            steam: SteamGames::default(),
            steam_warning: None,
        }
    }
    #[test]
    fn a_blocked_scan_keeps_one_worker_and_only_the_latest_pending_request() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut first = true;
        let worker = Worker::fake(move |_, _| {
            if first {
                first = false;
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
            Ok(empty())
        });
        let context = egui::Context::default();
        let config = DetectionConfig::from(&RecordingSettings::default());
        let initial = worker.submit(config.clone(), false, &context).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let mut latest = None;
        for _ in 0..50 {
            latest = Some(worker.submit(config.clone(), false, &context).unwrap());
        }
        assert_eq!(
            worker
                .mailbox
                .0
                .lock()
                .unwrap()
                .pending
                .as_ref()
                .unwrap()
                .config,
            config
        );
        release_tx.send(()).unwrap();
        initial
            .receiver
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        latest
            .unwrap()
            .receiver
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
    }
    #[test]
    fn errors_and_panics_are_reported_and_do_not_become_empty_successful_observations() {
        let context = egui::Context::default();
        let config = DetectionConfig::from(&RecordingSettings::default());
        let worker = Worker::fake(|_, _| Err("scan failed".into()));
        assert!(
            worker
                .submit(config.clone(), false, &context)
                .unwrap()
                .receiver
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .is_err()
        );
        let worker = Worker::fake(|_, _| panic!("synthetic worker failure"));
        assert!(
            worker
                .submit(config, false, &context)
                .unwrap()
                .receiver
                .recv_timeout(Duration::from_secs(3))
                .is_err()
        );
    }
}

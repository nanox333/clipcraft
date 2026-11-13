//! Recording what one application is playing, apart from everything else.
//!
//! Windows 10 (version 2004) and newer can give a program the sound of one process and the
//! processes it started ("process loopback"), which is what this uses. The source is patient: if
//! the application is not running yet (Discord, a music player) it reports itself as waiting and
//! picks the application up when it appears, and again if it restarts. Other systems cannot do
//! this yet and say so.
//!
//! The Windows capture runs on a thread of its own that fills a bounded queue, the same shape as
//! the other sound sources; all the COM objects live and die on that thread. The `unsafe` calls
//! are documented where they are made.
#![cfg_attr(windows, allow(unsafe_code))]

use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use clipcraft_record::AudioSource;

/// Which process a mixer row records, as the provider is asked for it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppTarget {
    /// The game being recorded (its process is told to the source as it changes).
    Game,
    /// A program, by file name.
    Executable(String),
}

impl AppTarget {
    /// Reads `game` or `exe:<name>`.
    pub fn parse(spec: &str) -> Option<Self> {
        if spec == "game" {
            return Some(Self::Game);
        }
        let name = spec.strip_prefix("exe:")?.trim();
        (!name.is_empty() && name.len() <= 128 && !name.contains(['/', '\\', '\0']))
            .then(|| Self::Executable(name.to_owned()))
    }

    /// A name for messages.
    pub fn label(&self) -> String {
        match self {
            Self::Game => "The game".to_owned(),
            Self::Executable(name) => name.trim_end_matches(".exe").to_owned(),
        }
    }
}

/// The process to record for `target`: the game's own, or the root of the named program's
/// processes (the one whose parent is not the same program), so everything it started is
/// included. `None` while it is not running.
pub fn resolve_process(
    target: &AppTarget,
    game_process: u32,
    processes: &[crate::RunningProcess],
) -> Option<u32> {
    match target {
        AppTarget::Game => (game_process != 0).then_some(game_process),
        AppTarget::Executable(wanted) => {
            let wanted = wanted.to_ascii_lowercase();
            let same = |name: &str| {
                let name = name.to_ascii_lowercase();
                name == wanted || name.trim_end_matches(".exe") == wanted.trim_end_matches(".exe")
            };
            let matching: Vec<&crate::RunningProcess> = processes
                .iter()
                .filter(|process| same(&process.executable))
                .collect();
            matching
                .iter()
                .filter(|process| {
                    process.parent_id.is_none_or(|parent| {
                        !matching.iter().any(|other| other.process_id == parent)
                    })
                })
                .map(|process| process.process_id)
                .min()
                .or_else(|| matching.iter().map(|process| process.process_id).min())
        }
    }
}

/// Opens a source for the application named by `spec` (`game` or `exe:<name>`). `game_process` is
/// the process of the game being recorded, kept up to date by the caller (0 when there is none).
pub fn open_app_audio(
    spec: &str,
    game_process: Arc<AtomicU32>,
) -> Result<Box<dyn AudioSource>, String> {
    let target =
        AppTarget::parse(spec).ok_or_else(|| "that application is not valid".to_owned())?;
    platform::open(target, game_process)
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub fn open(_: AppTarget, _: Arc<AtomicU32>) -> Result<Box<dyn AudioSource>, String> {
        Err(
            "recording one application's sound needs Windows 10 (version 2004) or newer; it is not available on this system yet"
                .to_owned(),
        )
    }
}

#[cfg(windows)]
mod platform {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Condvar, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use clipcraft_record::{AudioError, AudioFormat, SampleQueue};
    use windows::Win32::Media::Audio::{
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
        AUDCLNT_STREAMFLAGS_NOPERSIST, AUDIOCLIENT_ACTIVATION_PARAMS,
        AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
        ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
        IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
        IAudioCaptureClient, IAudioClient, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
        VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
    };
    use windows::Win32::System::Com::StructuredStorage::{
        PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
    };
    use windows::Win32::System::Com::{BLOB, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::Win32::System::Variant::VT_BLOB;
    use windows::core::{IUnknown, Interface, Ref, implement};

    use super::*;

    /// The format asked of the virtual device: 16-bit stereo at 44.1 kHz, which Windows converts
    /// to for any process.
    const RATE: u32 = 44_100;
    const CHANNELS: u16 = 2;
    const QUEUE_SECONDS: usize = 2;
    /// How long activation may take.
    const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(5);
    /// `WAVE_FORMAT_PCM`.
    const FORMAT_PCM: u16 = 1;
    /// `AUDCLNT_BUFFERFLAGS_SILENT`.
    const BUFFER_SILENT: u32 = 2;

    /// Signals that activation has finished.
    #[implement(IActivateAudioInterfaceCompletionHandler)]
    struct Completion(Arc<(Mutex<bool>, Condvar)>);

    impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
        fn ActivateCompleted(
            &self,
            _operation: Ref<'_, IActivateAudioInterfaceAsyncOperation>,
        ) -> windows::core::Result<()> {
            let (done, signal) = &*self.0;
            if let Ok(mut done) = done.lock() {
                *done = true;
            }
            signal.notify_all();
            Ok(())
        }
    }

    /// An audio client for the sound of `process` and everything it started.
    fn activate(process: u32) -> Result<IAudioClient, String> {
        let mut parameters = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: windows::Win32::Media::Audio::AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: process,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        };
        let blob = BLOB {
            cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            pBlobData: (&raw mut parameters).cast(),
        };
        let variant = PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_BLOB,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 { blob },
                }),
            },
        };
        let finished = Arc::new((Mutex::new(false), Condvar::new()));
        let handler: IActivateAudioInterfaceCompletionHandler = Completion(finished.clone()).into();
        // SAFETY: `variant` and the parameters it points to stay alive and unmoved until after the
        // completion handler has run (this function waits for it below); the interface id and
        // device path are static; `handler` is a live COM object.
        let operation = unsafe {
            ActivateAudioInterfaceAsync(
                VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
                &IAudioClient::IID,
                Some(&variant),
                &handler,
            )
        }
        .map_err(|error| format!("Windows would not start the application's sound: {error}"))?;
        let (done, signal) = &*finished;
        let guard = done
            .lock()
            .map_err(|_| "the sound activation failed".to_owned())?;
        let (guard, timeout) = signal
            .wait_timeout_while(guard, ACTIVATION_TIMEOUT, |done| !*done)
            .map_err(|_| "the sound activation failed".to_owned())?;
        drop(guard);
        if timeout.timed_out() {
            return Err("Windows did not answer in time".to_owned());
        }
        let mut result = windows::core::HRESULT(0);
        let mut unknown: Option<IUnknown> = None;
        // SAFETY: both places are valid for the call, and the operation finished (see above).
        unsafe { operation.GetActivateResult(&mut result, &mut unknown) }
            .map_err(|error| format!("Windows would not start the application's sound: {error}"))?;
        result
            .ok()
            .map_err(|error| format!("Windows would not record that application: {error}"))?;
        unknown
            .ok_or_else(|| "Windows gave no audio client".to_owned())?
            .cast::<IAudioClient>()
            .map_err(|error| error.to_string())
    }

    /// Runs on the capture thread: sets up, reports, then copies sound into the queue until told
    /// to stop or the application's sound goes away.
    fn capture_thread(
        process: u32,
        queue: &SampleQueue,
        stop: &AtomicBool,
        ready: &mpsc::Sender<Result<(), String>>,
    ) {
        // SAFETY: the thread owns its COM apartment, entered here and left on every exit below.
        let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let setup = (|| -> Result<(IAudioClient, IAudioCaptureClient, windows::Win32::Foundation::HANDLE), String> {
            let client = activate(process)?;
            let format = WAVEFORMATEX {
                wFormatTag: FORMAT_PCM,
                nChannels: CHANNELS,
                nSamplesPerSec: RATE,
                nAvgBytesPerSec: RATE * u32::from(CHANNELS) * 2,
                nBlockAlign: CHANNELS * 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            // SAFETY: `format` is a valid, fully initialised description that outlives the call.
            unsafe {
                client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                        | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                        | AUDCLNT_STREAMFLAGS_NOPERSIST,
                    0,
                    0,
                    &format,
                    None,
                )
            }
            .map_err(|error| format!("Windows would not open the application's sound: {error}"))?;
            // SAFETY: plain event creation; the handle is closed on every exit of the thread.
            let event = unsafe { CreateEventW(None, false, false, None) }
                .map_err(|error| error.to_string())?;
            // SAFETY: `event` is a live handle, kept until the capture ends.
            unsafe { client.SetEventHandle(event) }.map_err(|error| error.to_string())?;
            // SAFETY: the client was initialised above.
            let capture: IAudioCaptureClient =
                unsafe { client.GetService() }.map_err(|error| error.to_string())?;
            // SAFETY: the client was initialised above.
            unsafe { client.Start() }.map_err(|error| error.to_string())?;
            Ok((client, capture, event))
        })();
        let (client, capture, event) = match setup {
            Ok(parts) => {
                let _ = ready.send(Ok(()));
                parts
            }
            Err(why) => {
                let _ = ready.send(Err(why));
                if initialized {
                    // SAFETY: balances the successful `CoInitializeEx` above.
                    unsafe { CoUninitialize() };
                }
                return;
            }
        };
        while !stop.load(Ordering::SeqCst) {
            // SAFETY: `event` is a live handle.
            unsafe { WaitForSingleObject(event, 20) };
            loop {
                // SAFETY: the capture client belongs to this thread and was started above.
                let packet = match unsafe { capture.GetNextPacketSize() } {
                    Ok(frames) => frames,
                    Err(_) => {
                        queue.fail();
                        return finish(&client, event, initialized);
                    }
                };
                if packet == 0 {
                    break;
                }
                let (mut data, mut frames, mut flags) = (std::ptr::null_mut::<u8>(), 0_u32, 0_u32);
                // SAFETY: the places are valid for the call; the buffer is read only until the
                // matching `ReleaseBuffer` below, and only `frames` frames of it.
                if unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }
                    .is_err()
                {
                    queue.fail();
                    return finish(&client, event, initialized);
                }
                let samples = frames as usize * usize::from(CHANNELS);
                if flags & BUFFER_SILENT != 0 || data.is_null() {
                    queue.push(std::iter::repeat_n(0.0_f32, samples));
                } else {
                    // SAFETY: `data` points at `frames` frames of 16-bit stereo, valid until the
                    // buffer is released; unaligned reads of each value are used.
                    let raw = unsafe { std::slice::from_raw_parts(data, samples * 2) };
                    queue.push(
                        raw.chunks_exact(2)
                            .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
                            .collect::<Vec<f32>>()
                            .into_iter(),
                    );
                }
                // SAFETY: releases exactly the frames just taken.
                let _ = unsafe { capture.ReleaseBuffer(frames) };
            }
        }
        finish(&client, event, initialized);
    }

    fn finish(client: &IAudioClient, event: windows::Win32::Foundation::HANDLE, initialized: bool) {
        // SAFETY: stopping a started client, closing the handle this thread created, and leaving
        // the apartment this thread entered: each once, on the way out.
        unsafe {
            let _ = client.Stop();
            let _ = windows::Win32::Foundation::CloseHandle(event);
            if initialized {
                CoUninitialize();
            }
        }
    }

    /// One application's sound as a source. See the module note.
    pub struct AppAudio {
        target: AppTarget,
        game_process: Arc<AtomicU32>,
        queue: Arc<SampleQueue>,
        stop: Arc<AtomicBool>,
        attached_to: u32,
        waiting: String,
        last_try: Instant,
    }

    pub fn open(
        target: AppTarget,
        game_process: Arc<AtomicU32>,
    ) -> Result<Box<dyn AudioSource>, String> {
        let mut source = AppAudio {
            waiting: format!("{} is not running", target.label()),
            target,
            game_process,
            queue: Arc::new(SampleQueue::new(
                RATE as usize * usize::from(CHANNELS) * QUEUE_SECONDS,
            )),
            stop: Arc::new(AtomicBool::new(true)),
            attached_to: 0,
            last_try: Instant::now(),
        };
        // Attaching now when the application is already running; otherwise the source waits and
        // `recover` keeps looking.
        let _ = source.attach();
        Ok(Box::new(source))
    }

    impl AppAudio {
        /// Looks for the application and starts capturing it. True when it is being captured.
        fn attach(&mut self) -> bool {
            self.last_try = Instant::now();
            let processes = crate::ProcessObserver::default().refresh(false);
            let Some(process) = resolve_process(
                &self.target,
                self.game_process.load(Ordering::SeqCst),
                &processes,
            ) else {
                self.waiting = format!("{} is not running", self.target.label());
                return false;
            };
            self.stop.store(true, Ordering::SeqCst);
            let queue = Arc::new(SampleQueue::new(
                RATE as usize * usize::from(CHANNELS) * QUEUE_SECONDS,
            ));
            let stop = Arc::new(AtomicBool::new(false));
            let (ready, wait) = mpsc::channel();
            let (thread_queue, thread_stop) = (queue.clone(), stop.clone());
            if std::thread::Builder::new()
                .name("clipcraft-app-audio".to_owned())
                .spawn(move || capture_thread(process, &thread_queue, &thread_stop, &ready))
                .is_err()
            {
                self.waiting = "the sound thread could not start".to_owned();
                return false;
            }
            match wait.recv_timeout(ACTIVATION_TIMEOUT + Duration::from_secs(1)) {
                Ok(Ok(())) => {
                    self.queue = queue;
                    self.stop = stop;
                    self.attached_to = process;
                    true
                }
                Ok(Err(why)) => {
                    self.waiting = why;
                    false
                }
                Err(_) => {
                    stop.store(true, Ordering::SeqCst);
                    self.waiting = "Windows did not answer in time".to_owned();
                    false
                }
            }
        }
    }

    impl AudioSource for AppAudio {
        fn name(&self) -> String {
            self.target.label()
        }

        fn format(&self) -> AudioFormat {
            AudioFormat {
                sample_rate: RATE,
                channels: CHANNELS,
            }
        }

        fn read(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
            if self.attached_to == 0 {
                return Err(AudioError::DeviceLost(self.waiting.clone()));
            }
            self.queue.drain_into(out);
            if self.queue.failed() {
                self.attached_to = 0;
                self.waiting = format!("{} stopped playing sound", self.target.label());
                return Err(AudioError::DeviceLost(self.waiting.clone()));
            }
            Ok(())
        }

        fn recover(&mut self) -> bool {
            // The game may have changed, or the application may have started or restarted.
            if self.last_try.elapsed() < Duration::from_secs(2) {
                return false;
            }
            self.attach()
        }
    }

    impl Drop for AppAudio {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RunningProcess;

    fn process(id: u32, name: &str, parent: Option<u32>) -> RunningProcess {
        RunningProcess {
            process_id: id,
            executable: name.to_owned(),
            steam_app_id: None,
            parent_id: parent,
        }
    }

    #[test]
    fn a_target_is_read_from_its_text_and_hostile_text_is_refused() {
        assert_eq!(AppTarget::parse("game"), Some(AppTarget::Game));
        assert_eq!(
            AppTarget::parse("exe:Discord.exe"),
            Some(AppTarget::Executable("Discord.exe".into()))
        );
        for bad in [
            "", "exe:", "exe:  ", "exe:a/b", "exe:a\\b", "exe:a\0b", "Discord", "games",
        ] {
            assert_eq!(AppTarget::parse(bad), None, "{bad:?}");
        }
        assert_eq!(AppTarget::parse(&format!("exe:{}", "x".repeat(129))), None);
        assert_eq!(
            AppTarget::Executable("Discord.exe".into()).label(),
            "Discord"
        );
    }

    #[test]
    fn the_process_to_record_is_the_root_of_the_programs_processes() {
        let processes = [
            process(10, "explorer.exe", None),
            // Discord: a main process and helpers that it started.
            process(100, "Discord.exe", Some(10)),
            process(101, "Discord.exe", Some(100)),
            process(102, "discord.exe", Some(100)),
            process(200, "spotify.exe", Some(10)),
        ];
        let find = |target: &AppTarget, game| resolve_process(target, game, &processes);
        assert_eq!(
            find(&AppTarget::Executable("discord.exe".into()), 0),
            Some(100)
        );
        assert_eq!(find(&AppTarget::Executable("Discord".into()), 0), Some(100));
        assert_eq!(
            find(&AppTarget::Executable("SPOTIFY.EXE".into()), 0),
            Some(200)
        );
        assert_eq!(find(&AppTarget::Executable("chrome.exe".into()), 0), None);
        // The game is whatever process the app says it is, or nothing.
        assert_eq!(find(&AppTarget::Game, 4242), Some(4242));
        assert_eq!(find(&AppTarget::Game, 0), None);
        // Several unrelated copies: the lowest root.
        let two = [
            process(30, "game.exe", None),
            process(20, "game.exe", None),
            process(21, "game.exe", Some(20)),
        ];
        assert_eq!(
            resolve_process(&AppTarget::Executable("game.exe".into()), 0, &two),
            Some(20)
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn where_it_is_not_available_it_says_so_instead_of_pretending() {
        let error = open_app_audio("exe:discord.exe", Arc::new(AtomicU32::new(0)))
            .err()
            .expect("not available here");
        assert!(error.contains("Windows 10"), "{error}");
        assert!(open_app_audio("nonsense", Arc::new(AtomicU32::new(0))).is_err());
    }
}

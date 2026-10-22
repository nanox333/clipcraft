use clipcraft_engine::{Command, CommandOutput, command_ids, execute};
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "clipcraft-cli: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: Vec<OsString>) -> Result<(), CliError> {
    let Some(first) = arguments.first().and_then(|argument| argument.to_str()) else {
        return print_help();
    };

    match first {
        "help" | "--help" | "-h" => print_help(),
        "version" | "--version" | "-V" => {
            write_stdout(format!("clipcraft-cli {}\n", env!("CARGO_PKG_VERSION")).as_bytes())
        }
        "commands" => {
            if arguments.len() != 1 {
                return Err(CliError::Usage);
            }
            write_json(&command_ids())
        }
        "library" => run_library(&arguments[1..]),
        "recording" => run_recording(&arguments[1..]),
        "capture" => run_capture(&arguments[1..]),
        "clip" => run_clip(&arguments[1..]),
        "editor" => run_editor(&arguments[1..]),
        "diagnose" => run_diagnose(&arguments[1..]),
        _ => Err(CliError::Usage),
    }
}

/// Local JSON command transport; a transient edit payload is not a project-file format.
fn run_editor(arguments: &[OsString]) -> Result<(), CliError> {
    use std::io::Read;
    if arguments.len() != 1 {
        return Err(CliError::Usage);
    }
    const LIMIT: u64 = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    io::stdin()
        .lock()
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CliError::Usage)?;
    if bytes.len() as u64 > LIMIT {
        return Err(CliError::Usage);
    }
    let command = match arguments[0].to_str() {
        Some("import") => Command::ImportEditorMedia {
            paths: serde_json::from_slice(&bytes).map_err(|_| CliError::Usage)?,
        },
        Some("apply") => {
            #[derive(serde::Deserialize)]
            struct Edit {
                #[serde(default)]
                session: clipcraft_engine::EditorSession,
                action: clipcraft_engine::EditorAction,
            }
            let request: Edit = serde_json::from_slice(&bytes).map_err(|_| CliError::Usage)?;
            Command::EditSequence {
                session: Box::new(request.session),
                action: request.action,
            }
        }
        Some("export") => Command::ExportSequence {
            request: {
                clipcraft_engine::set_export_hardware(clipcraft_platform::hardware_provider());
                clipcraft_platform::register_hardware_decoding();
                let gpu = clipcraft_engine::default_settings_path()
                    .and_then(
                        |path| match execute(Command::LoadRecordingSettings { path }) {
                            Ok(CommandOutput::RecordingSettings(settings)) => Some(*settings),
                            _ => None,
                        },
                    )
                    .is_none_or(|settings| {
                        settings.video_encoder == clipcraft_engine::VideoEncoder::Gpu
                    });
                clipcraft_engine::enable_export_hardware(gpu);
                Box::new(serde_json::from_slice(&bytes).map_err(|_| CliError::Usage)?)
            },
        },
        _ => return Err(CliError::Usage),
    };
    match execute(command).map_err(CliError::Engine)? {
        CommandOutput::EditorMedia(entries) => write_json(&entries),
        CommandOutput::EditorSession(session) => write_json(&session),
        CommandOutput::SequenceExported(report) => write_json(&report),
        _ => Err(CliError::UnexpectedOutput),
    }
}

fn run_library(arguments: &[OsString]) -> Result<(), CliError> {
    let Some(operation) = arguments.first().and_then(|argument| argument.to_str()) else {
        return Err(CliError::Usage);
    };
    if arguments.len() != 2 {
        return Err(CliError::Usage);
    }

    let path = PathBuf::from(&arguments[1]);
    match operation {
        "scan" => {
            let output =
                execute(Command::ScanLibrary { directory: path }).map_err(CliError::Engine)?;
            match output {
                CommandOutput::LibraryScan(result) => write_json(&result),
                _ => Err(CliError::UnexpectedOutput),
            }
        }
        "reveal" => {
            let output =
                execute(Command::RevealInFileManager { path }).map_err(CliError::Engine)?;
            match output {
                CommandOutput::Revealed { path } => write_json(&path),
                _ => Err(CliError::UnexpectedOutput),
            }
        }
        "thumbnail" => {
            let cache_dir = clipcraft_engine::default_thumbnail_dir().ok_or(CliError::Usage)?;
            let output =
                execute(Command::EnsureThumbnail { path, cache_dir }).map_err(CliError::Engine)?;
            match output {
                CommandOutput::Thumbnail(file) => write_json(&file),
                _ => Err(CliError::UnexpectedOutput),
            }
        }
        "probe" => {
            let output = execute(Command::ProbeClip { path }).map_err(CliError::Engine)?;
            match output {
                CommandOutput::ClipProbe(probe) => write_json(&probe),
                _ => Err(CliError::UnexpectedOutput),
            }
        }
        "path" => {
            let output = execute(Command::LocalMediaPath { path }).map_err(CliError::Engine)?;
            match output {
                CommandOutput::LocalMediaPath { path } => write_json(&path),
                _ => Err(CliError::UnexpectedOutput),
            }
        }
        _ => Err(CliError::Usage),
    }
}

/// `clip trim INPUT OUTPUT START END` cuts INPUT from START to END (seconds) into a new MP4
/// without re-encoding, and prints what was written.
fn run_clip(arguments: &[OsString]) -> Result<(), CliError> {
    // `clip normalize INPUT OUTPUT START END [LUFS]` evens out the loudness of a range.
    if arguments.first().and_then(|arg| arg.to_str()) == Some("normalize") {
        if !(5..=6).contains(&arguments.len()) {
            return Err(CliError::Usage);
        }
        let millis = |index: usize| -> Result<u64, CliError> {
            let value: f64 = arguments[index]
                .to_str()
                .and_then(|text| text.parse().ok())
                .ok_or(CliError::Usage)?;
            if !(0.0..=1.0e9).contains(&value) {
                return Err(CliError::Usage);
            }
            Ok((value * 1000.0).round() as u64)
        };
        let target_lufs: i8 = match arguments.get(5) {
            None => -16,
            Some(text) => text
                .to_str()
                .and_then(|text| text.parse().ok())
                .filter(|value| clipcraft_engine::LOUDNESS_TARGETS.contains(&f64::from(*value)))
                .ok_or(CliError::Usage)?,
        };
        let command = Command::NormalizeClip {
            input: PathBuf::from(&arguments[1]),
            output: PathBuf::from(&arguments[2]),
            start_ms: millis(3)?,
            end_ms: millis(4)?,
            target_lufs,
        };
        return match execute(command).map_err(CliError::Engine)? {
            CommandOutput::Normalized(clip) => write_json(&clip),
            _ => Err(CliError::UnexpectedOutput),
        };
    }
    // `clip gif INPUT OUTPUT.gif START END [WIDTH [FPS]]` saves a range of up to 30 seconds.
    if arguments.first().and_then(|arg| arg.to_str()) == Some("gif") {
        if !(5..=7).contains(&arguments.len()) {
            return Err(CliError::Usage);
        }
        let number = |index: usize| -> Option<f64> { arguments.get(index)?.to_str()?.parse().ok() };
        let millis = |index: usize| -> Result<u64, CliError> {
            let value = number(index).ok_or(CliError::Usage)?;
            if !(0.0..=1.0e9).contains(&value) {
                return Err(CliError::Usage);
            }
            Ok((value * 1000.0).round() as u64)
        };
        let defaults = clipcraft_engine::GifOptions::default();
        let whole = |index: usize, default: u32| -> Result<u32, CliError> {
            match arguments.get(index) {
                None => Ok(default),
                Some(text) => text
                    .to_str()
                    .and_then(|text| text.parse().ok())
                    .ok_or(CliError::Usage),
            }
        };
        let command = Command::ExportGif {
            input: PathBuf::from(&arguments[1]),
            output: PathBuf::from(&arguments[2]),
            start_ms: millis(3)?,
            end_ms: millis(4)?,
            max_width: whole(5, defaults.max_width)?,
            fps: whole(6, defaults.fps)?,
        };
        return match execute(command).map_err(CliError::Engine)? {
            CommandOutput::Trimmed(clip) => write_json(&clip),
            _ => Err(CliError::UnexpectedOutput),
        };
    }
    let exporting = arguments.first().and_then(|arg| arg.to_str()) == Some("export");
    if exporting {
        // Small-file export uses the GPU encoder when the recording settings say so.
        clipcraft_engine::set_export_hardware(clipcraft_platform::hardware_provider());
        clipcraft_platform::register_hardware_decoding();
        let gpu = clipcraft_engine::default_settings_path()
            .and_then(
                |path| match execute(Command::LoadRecordingSettings { path }) {
                    Ok(CommandOutput::RecordingSettings(settings)) => Some(*settings),
                    _ => None,
                },
            )
            .is_none_or(|settings| settings.video_encoder == clipcraft_engine::VideoEncoder::Gpu);
        clipcraft_engine::enable_export_hardware(gpu);
    }
    if !((exporting && arguments.len() == 6)
        || (arguments.len() == 5 && arguments[0].to_str() == Some("trim")))
    {
        return Err(CliError::Usage);
    }
    let seconds = |index: usize| -> Result<u64, CliError> {
        let value: f64 = arguments[index]
            .to_str()
            .and_then(|text| text.parse().ok())
            .ok_or(CliError::Usage)?;
        if !(0.0..=1.0e9).contains(&value) {
            return Err(CliError::Usage);
        }
        Ok((value * 1000.0).round() as u64)
    };
    let input = PathBuf::from(&arguments[1]);
    let output = PathBuf::from(&arguments[2]);
    let (start_ms, end_ms) = (seconds(3)?, seconds(4)?);
    let command = if exporting {
        let megabytes: u16 = arguments[5]
            .to_str()
            .and_then(|text| text.parse().ok())
            .filter(|size| (1..=500).contains(size))
            .ok_or(CliError::Usage)?;
        Command::ExportClip {
            input,
            output,
            start_ms,
            end_ms,
            preset: clipcraft_engine::ExportPreset::Custom { megabytes },
        }
    } else {
        Command::TrimClip {
            input,
            output,
            start_ms,
            end_ms,
        }
    };
    match execute(command).map_err(CliError::Engine)? {
        CommandOutput::Trimmed(clip) => write_json(&clip),
        CommandOutput::Exported(clip) => write_json(&clip),
        _ => Err(CliError::UnexpectedOutput),
    }
}

/// `diagnose [--json]` checks the folders, screens, sound, and encoders and says what works. It
/// exits with failure when something that recording needs is wrong.
fn run_diagnose(arguments: &[OsString]) -> Result<(), CliError> {
    let json = match arguments {
        [] => false,
        [flag] if flag.to_str() == Some("--json") => true,
        _ => return Err(CliError::Usage),
    };
    let settings = clipcraft_engine::default_settings_path()
        .and_then(
            |path| match execute(Command::LoadRecordingSettings { path }) {
                Ok(CommandOutput::RecordingSettings(settings)) => Some(*settings),
                _ => None,
            },
        )
        .unwrap_or_default();
    let mut report = clipcraft_engine::Diagnostics::collect(&settings);
    let check = |(name, ok, detail): clipcraft_platform::Finding| clipcraft_engine::Check {
        name,
        ok,
        detail,
    };
    report.capture = clipcraft_platform::diagnose_capture()
        .into_iter()
        .map(check)
        .collect();
    report.video = clipcraft_platform::diagnose_video()
        .into_iter()
        .map(check)
        .collect();
    let devices = clipcraft_audio_out::list_devices();
    // A missing device only counts as a problem when the settings ask for it.
    let named = |kind: &str, names: &[String], needed: bool| {
        if names.is_empty() && needed {
            clipcraft_engine::Check::fail(kind, "none found")
        } else if names.is_empty() {
            clipcraft_engine::Check::pass(kind, "none found (not needed with the current settings)")
        } else {
            clipcraft_engine::Check::pass(kind, names.join(", "))
        }
    };
    report.audio.push(named(
        "Microphones",
        &devices.inputs,
        settings.capture_microphone,
    ));
    report.audio.push(named(
        "Speakers",
        &devices.outputs,
        settings.capture_game_audio,
    ));
    let mut system = check(clipcraft_platform::diagnose_system_sound());
    if !system.ok && !settings.capture_game_audio {
        system.ok = true;
        system.detail = format!("{} (not needed with the current settings)", system.detail);
    }
    report.audio.push(system);
    if json {
        write_json(&report)?;
    } else {
        write_stdout(report.text().as_bytes())?;
    }
    if report.healthy() {
        Ok(())
    } else {
        Err(CliError::Capture(
            "something recording needs is not working; see the lines marked !!".to_owned(),
        ))
    }
}

fn run_recording(arguments: &[OsString]) -> Result<(), CliError> {
    if arguments.first().and_then(|value| value.to_str()) == Some("games") {
        let operation = arguments.get(1).and_then(|value| value.to_str());
        if operation == Some("list") && arguments.len() == 2 {
            return write_json(&clipcraft_engine::built_in_games());
        }
        if operation == Some("installed") && arguments.len() <= 66 {
            let roots = if arguments.len() == 2 {
                clipcraft_platform::steam_installation_roots()
            } else {
                arguments[2..].iter().map(PathBuf::from).collect()
            };
            return match execute(Command::ScanSteamGames { roots }).map_err(CliError::Engine)? {
                CommandOutput::SteamGames(found) => write_json(&found),
                _ => Err(CliError::UnexpectedOutput),
            };
        }
        let path = clipcraft_engine::default_settings_path().ok_or(CliError::Usage)?;
        let CommandOutput::RecordingSettings(mut settings) =
            execute(Command::LoadRecordingSettings { path: path.clone() })
                .map_err(CliError::Engine)?
        else {
            return Err(CliError::UnexpectedOutput);
        };
        match operation {
            Some("automatic") if arguments.len() == 3 => {
                settings.detect_built_in_games = match arguments[2].to_str() {
                    Some("on") => true,
                    Some("off") => false,
                    _ => return Err(CliError::Usage),
                };
            }
            Some(operation @ ("enable" | "disable"))
                if (3..=clipcraft_engine::MAX_BUILT_IN_GAMES + 2).contains(&arguments.len()) =>
            {
                let ids = if arguments.len() == 3 && arguments[2].to_str() == Some("all") {
                    clipcraft_engine::built_in_games()
                        .iter()
                        .map(|game| game.id.to_owned())
                        .collect()
                } else {
                    arguments[2..]
                        .iter()
                        .map(|id| id.to_str().map(str::to_owned).ok_or(CliError::Usage))
                        .collect::<Result<Vec<_>, _>>()?
                };
                let CommandOutput::RecordingSettings(updated) = execute(Command::SetBuiltInGames {
                    settings,
                    ids,
                    enabled: operation == "enable",
                })
                .map_err(CliError::Engine)?
                else {
                    return Err(CliError::UnexpectedOutput);
                };
                settings = updated;
            }
            _ => return Err(CliError::Usage),
        }
        let CommandOutput::RecordingSettings(settings) =
            execute(Command::SaveRecordingSettings { path, settings }).map_err(CliError::Engine)?
        else {
            return Err(CliError::UnexpectedOutput);
        };
        return write_json(&settings);
    }
    if arguments.len() != 1 || arguments[0].to_str() != Some("settings") {
        return Err(CliError::Usage);
    }
    let path = clipcraft_engine::default_settings_path().ok_or(CliError::Usage)?;
    match execute(Command::LoadRecordingSettings { path }).map_err(CliError::Engine)? {
        CommandOutput::RecordingSettings(settings) => write_json(&settings),
        _ => Err(CliError::UnexpectedOutput),
    }
}

/// `capture targets` lists what can be captured. `capture record --seconds N [--target ID]
/// [--out DIR]` records for N seconds with the saved settings, then saves the replay buffer as a
/// clip and prints where it went.
fn run_capture(arguments: &[OsString]) -> Result<(), CliError> {
    match arguments.first().and_then(|argument| argument.to_str()) {
        Some("targets") if arguments.len() == 1 => {
            let targets = clipcraft_platform::list_targets()
                .map_err(|error| CliError::Capture(error.to_string()))?;
            let rows: Vec<serde_json::Value> = targets
                .iter()
                .map(|target| {
                    serde_json::json!({
                        "id": target.id,
                        "name": target.name,
                        "kind": match target.kind {
                            clipcraft_capture::TargetKind::Display => "display",
                            clipcraft_capture::TargetKind::Window => "window",
                        },
                        "width": target.width,
                        "height": target.height,
                    })
                })
                .collect();
            write_json(&rows)
        }
        Some("record") => run_record(&arguments[1..]),
        _ => Err(CliError::Usage),
    }
}

fn run_record(arguments: &[OsString]) -> Result<(), CliError> {
    let (mut seconds, mut target, mut out) = (None::<u64>, None::<String>, None::<PathBuf>);
    let mut rest = arguments.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or(CliError::Usage)?;
        match flag.to_str() {
            Some("--seconds") => {
                seconds = Some(
                    value
                        .to_str()
                        .and_then(|v| v.parse().ok())
                        .ok_or(CliError::Usage)?,
                );
            }
            Some("--target") => target = Some(value.to_str().ok_or(CliError::Usage)?.to_owned()),
            Some("--out") => out = Some(PathBuf::from(value)),
            _ => return Err(CliError::Usage),
        }
    }
    let seconds = seconds
        .filter(|seconds| (1..=3_600).contains(seconds))
        .ok_or(CliError::Usage)?;
    let settings_path = clipcraft_engine::default_settings_path().ok_or(CliError::Usage)?;
    let settings = match execute(Command::LoadRecordingSettings {
        path: settings_path,
    })
    .map_err(CliError::Engine)?
    {
        CommandOutput::RecordingSettings(settings) => *settings,
        _ => return Err(CliError::UnexpectedOutput),
    };
    let target_id = match target {
        Some(id) => id,
        None => clipcraft_platform::list_targets()
            .map_err(|error| CliError::Capture(error.to_string()))?
            .into_iter()
            .find(|target| target.kind == clipcraft_capture::TargetKind::Display)
            .map(|target| target.id)
            .ok_or_else(|| CliError::Capture("no screen was found".to_owned()))?,
    };
    let source = clipcraft_platform::open(
        &target_id,
        &clipcraft_platform::CaptureOptions {
            show_cursor: settings.capture_cursor,
            hdr_to_sdr: settings.hdr_compatibility,
        },
    )
    .map_err(|error| CliError::Capture(error.to_string()))?;
    let folder = out
        .or_else(|| {
            (!settings.capture_folder.is_empty()).then(|| PathBuf::from(&settings.capture_folder))
        })
        .or_else(clipcraft_engine::default_capture_folder)
        .ok_or(CliError::Usage)?;
    let mut session = clipcraft_engine::CaptureSession::new();
    let hardware = clipcraft_platform::hardware_provider();
    session
        .start(&settings, source, hardware)
        .map_err(CliError::Engine)?;
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    let report = session
        .save_clip(&settings, &folder)
        .map_err(CliError::Engine)?;
    let status = session.stop();
    write_json(&serde_json::json!({
        "path": report.clip.path,
        "frames": report.clip.frames,
        "duration_frames": report.clip.duration_frames,
        "bytes": report.clip.bytes,
        "deleted": report.deleted,
        "frames_captured": status.as_ref().map(|s| s.frames_captured),
        "frames_encoded": status.as_ref().map(|s| s.frames_encoded),
        "frames_dropped": status.as_ref().map(|s| s.frames_dropped),
        "average_encode_ms": status.as_ref().map(|s| s.average_encode_ms),
        "encoder": status.as_ref().map(|s| s.encoder.clone()),
        "output": status.as_ref().and_then(|s| s.output),
    }))
}

fn print_help() -> Result<(), CliError> {
    let help = concat!(
        "ClipCraft local tools\n",
        "\n",
        "Usage:\n",
        "  clipcraft-cli commands\n",
        "  clipcraft-cli editor import | editor apply | editor export  (JSON on stdin)\n",
        "  clipcraft-cli recording games list | recording games installed [STEAM_ROOT ...]\n",
        "  clipcraft-cli recording games automatic on|off\n",
        "  clipcraft-cli recording games enable|disable <game-id...|all>\n",
        "  clipcraft-cli library scan <folder>\n",
        "  clipcraft-cli library path <file>\n",
        "  clipcraft-cli library reveal <file>\n",
        "  clipcraft-cli library probe <file>\n",
        "  clipcraft-cli library thumbnail <file>\n",
        "  clipcraft-cli clip trim <input> <output.mp4> <start-seconds> <end-seconds>\n",
        "  clipcraft-cli clip export <input> <output.mp4> <start-seconds> <end-seconds> <target-MB:1..500>\n",
        "  clipcraft-cli clip gif <input> <output.gif> <start-seconds> <end-seconds> [width:160..960 [fps:5..20]]\n",
        "  clipcraft-cli clip normalize <input> <output.mp4> <start-seconds> <end-seconds> [target-LUFS:-30..-8, default -16]\n",
        "  clipcraft-cli diagnose [--json]\n",
        "  clipcraft-cli recording settings\n",
        "  clipcraft-cli capture targets\n",
        "  clipcraft-cli capture record --seconds N [--target ID] [--out DIR]\n",
        "  clipcraft-cli version\n",
        "\n",
        "All commands run locally. A library scan includes subfolders (8 levels), skips symlinks, and is bounded.\n",
    );
    write_stdout(help.as_bytes())
}

fn write_json(value: &impl serde::Serialize) -> Result<(), CliError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(CliError::Json)?;
    bytes.push(b'\n');
    write_stdout(&bytes)
}

fn write_stdout(bytes: &[u8]) -> Result<(), CliError> {
    io::stdout().lock().write_all(bytes).map_err(CliError::Io)
}

#[derive(Debug)]
enum CliError {
    Usage,
    Engine(clipcraft_engine::EngineError),
    Capture(String),
    UnexpectedOutput,
    Json(serde_json::Error),
    Io(io::Error),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage => formatter
                .write_str("invalid arguments. Run `clipcraft-cli help` for local command usage."),
            Self::Engine(error) => error.fmt(formatter),
            Self::Capture(message) => formatter.write_str(message),
            Self::UnexpectedOutput => {
                formatter.write_str("the command returned an unexpected result.")
            }
            Self::Json(_) => formatter.write_str("the result could not be written as JSON."),
            Self::Io(_) => formatter.write_str("standard output could not be written."),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Usage | Self::UnexpectedOutput | Self::Capture(_) => None,
            Self::Engine(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CliError, run};
    use std::ffi::OsString;

    #[test]
    fn export_rejects_missing_target_bad_sizes_and_non_finite_times() {
        for args in [
            vec!["clip", "export", "a.mp4", "b.mp4", "0", "2"],
            vec!["clip", "export", "a.mp4", "b.mp4", "0", "2", "0"],
            vec!["clip", "export", "a.mp4", "b.mp4", "0", "2", "501"],
            vec!["clip", "export", "a.mp4", "b.mp4", "NaN", "2", "10"],
        ] {
            assert!(matches!(
                run(args.into_iter().map(OsString::from).collect()),
                Err(CliError::Usage)
            ));
        }
        assert!(matches!(
            run(["clip", "export", "", "out.mp4", "0", "2", "10"]
                .into_iter()
                .map(OsString::from)
                .collect()),
            Err(CliError::Engine(_))
        ));
    }

    #[test]
    fn missing_folder_is_a_usage_error() {
        let error = run(vec![OsString::from("library"), OsString::from("scan")])
            .expect_err("missing folder must fail");
        assert!(matches!(error, CliError::Usage));
    }

    #[test]
    fn unknown_command_is_a_usage_error() {
        let error = run(vec![OsString::from("record")]).expect_err("unknown command must fail");
        assert!(matches!(error, CliError::Usage));
    }

    #[test]
    fn diagnose_takes_only_the_json_flag() {
        for bad in [
            vec!["diagnose", "--bogus"],
            vec!["diagnose", "--json", "extra"],
        ] {
            let error = run(bad.into_iter().map(OsString::from).collect()).expect_err("must fail");
            assert!(matches!(error, CliError::Usage));
        }
    }

    #[test]
    fn local_path_command_uses_the_shared_engine_command() {
        run(vec![
            OsString::from("library"),
            OsString::from("path"),
            OsString::from("/clips/example.mp4"),
        ])
        .expect("local path command succeeds");
    }

    #[test]
    fn local_path_command_rejects_empty_paths() {
        let error = run(vec![
            OsString::from("library"),
            OsString::from("path"),
            OsString::from(""),
        ])
        .expect_err("empty path must fail");

        assert!(matches!(error, CliError::Engine(_)));
    }
}

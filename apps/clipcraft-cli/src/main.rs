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
        _ => Err(CliError::Usage),
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

fn run_recording(arguments: &[OsString]) -> Result<(), CliError> {
    if arguments.len() != 1 || arguments[0].to_str() != Some("settings") {
        return Err(CliError::Usage);
    }
    let path = clipcraft_engine::default_settings_path().ok_or(CliError::Usage)?;
    match execute(Command::LoadRecordingSettings { path }).map_err(CliError::Engine)? {
        CommandOutput::RecordingSettings(settings) => write_json(&settings),
        _ => Err(CliError::UnexpectedOutput),
    }
}

fn print_help() -> Result<(), CliError> {
    let help = concat!(
        "ClipCraft local tools\n",
        "\n",
        "Usage:\n",
        "  clipcraft-cli commands\n",
        "  clipcraft-cli library scan <folder>\n",
        "  clipcraft-cli library path <file>\n",
        "  clipcraft-cli library reveal <file>\n",
        "  clipcraft-cli library probe <file>\n",
        "  clipcraft-cli library thumbnail <file>\n",
        "  clipcraft-cli recording settings\n",
        "  clipcraft-cli version\n",
        "\n",
        "All commands run locally. A library scan is non-recursive and bounded.\n",
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
            Self::Usage | Self::UnexpectedOutput => None,
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

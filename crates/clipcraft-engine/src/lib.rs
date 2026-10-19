//! Shared command and local library behavior for the ClipCraft UI and CLI.

use serde::Serialize;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Maximum directory entries inspected by one library scan.
pub const MAX_DIRECTORY_ENTRIES: usize = 20_000;
/// Maximum number of recognized videos returned by one scan.
pub const MAX_LIBRARY_RESULTS: usize = 5_000;

const VIDEO_EXTENSIONS: [&str; 7] = ["avi", "m4v", "mkv", "mov", "mp4", "webm", "wmv"];

/// A typed operation exposed by the shared command registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    /// Scans one local directory without recursing into child folders.
    ScanLibrary { directory: PathBuf },
    /// Returns a local media path for display or clipboard use.
    LocalMediaPath { path: PathBuf },
}

impl Command {
    /// Returns this operation's stable command ID.
    pub const fn id(&self) -> &'static str {
        match self {
            Self::ScanLibrary { .. } => "library.scan",
            Self::LocalMediaPath { .. } => "library.path",
        }
    }
}

/// Result of one local library scan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LibraryScanResult {
    /// Folder scanned by the command.
    pub directory: String,
    /// Video files discovered in the folder.
    pub entries: Vec<MediaEntry>,
}

/// Local metadata that can be shown in a library card.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MediaEntry {
    /// Full local path. This is never sent outside the process.
    pub path: String,
    /// Display name of the file.
    pub title: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Unix timestamp when available.
    pub modified_unix_seconds: Option<u64>,
    /// Lowercase file extension without a leading dot.
    pub extension: String,
}

/// Output produced by a registered engine command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CommandOutput {
    /// Result of scanning a local directory.
    LibraryScan(LibraryScanResult),
    /// A local file path requested by the library UI or CLI.
    LocalMediaPath { path: String },
}

/// Actionable errors returned by engine commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineError {
    /// The selected folder does not exist or cannot be read.
    DirectoryUnavailable,
    /// The selected path is not a directory.
    NotADirectory,
    /// The directory contains more entries than the bounded scanner accepts.
    DirectoryEntryLimit,
    /// The scan would return more results than the bounded library accepts.
    ResultLimit,
    /// A local media path was empty.
    EmptyMediaPath,
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectoryUnavailable => formatter.write_str(
                "The selected folder could not be read. Check that it exists and that ClipCraft has permission.",
            ),
            Self::NotADirectory => formatter.write_str("Choose a folder to scan."),
            Self::DirectoryEntryLimit => formatter.write_str(
                "This folder has too many entries to scan safely. Choose a narrower folder.",
            ),
            Self::ResultLimit => formatter.write_str(
                "This folder contains more videos than one scan can show. Choose a narrower folder.",
            ),
            Self::EmptyMediaPath => formatter.write_str("Choose a local media file first."),
        }
    }
}

impl std::error::Error for EngineError {}

/// Executes a typed user operation through the shared UI/CLI command path.
pub fn execute(command: Command) -> Result<CommandOutput, EngineError> {
    match command {
        Command::ScanLibrary { directory } => {
            scan_library_directory(&directory).map(CommandOutput::LibraryScan)
        }
        Command::LocalMediaPath { path } => {
            if path.as_os_str().is_empty() {
                return Err(EngineError::EmptyMediaPath);
            }
            Ok(CommandOutput::LocalMediaPath {
                path: path.to_string_lossy().into_owned(),
            })
        }
    }
}

/// Scans one folder for common local video containers.
pub fn scan_library_directory(directory: &Path) -> Result<LibraryScanResult, EngineError> {
    scan_library_directory_with_limit(directory, MAX_DIRECTORY_ENTRIES, MAX_LIBRARY_RESULTS)
}

fn scan_library_directory_with_limit(
    directory: &Path,
    max_entries: usize,
    max_results: usize,
) -> Result<LibraryScanResult, EngineError> {
    let directory_metadata =
        fs::metadata(directory).map_err(|_| EngineError::DirectoryUnavailable)?;
    if !directory_metadata.is_dir() {
        return Err(EngineError::NotADirectory);
    }

    let read_dir = fs::read_dir(directory).map_err(|_| EngineError::DirectoryUnavailable)?;
    let mut seen_entries = 0usize;
    let mut media = Vec::new();

    for directory_entry in read_dir {
        seen_entries = seen_entries
            .checked_add(1)
            .ok_or(EngineError::DirectoryEntryLimit)?;
        if seen_entries > max_entries {
            return Err(EngineError::DirectoryEntryLimit);
        }

        let directory_entry = directory_entry.map_err(|_| EngineError::DirectoryUnavailable)?;
        let path = directory_entry.path();
        let file_type = directory_entry
            .file_type()
            .map_err(|_| EngineError::DirectoryUnavailable)?;
        if !file_type.is_file() {
            continue;
        }

        let metadata =
            fs::symlink_metadata(&path).map_err(|_| EngineError::DirectoryUnavailable)?;
        if !metadata.file_type().is_file() {
            continue;
        }

        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        let Some(extension) = extension.filter(|value| VIDEO_EXTENSIONS.contains(&value.as_str()))
        else {
            continue;
        };

        if media.len() >= max_results {
            return Err(EngineError::ResultLimit);
        }

        let title = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled clip".to_owned());
        let modified_unix_seconds = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());

        media.push(MediaEntry {
            path: path.to_string_lossy().into_owned(),
            title,
            size_bytes: metadata.len(),
            modified_unix_seconds,
            extension,
        });
    }

    media.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.path.cmp(&right.path))
    });

    Ok(LibraryScanResult {
        directory: directory.to_string_lossy().into_owned(),
        entries: media,
    })
}

/// Lists stable user-facing command IDs available to the first application slice.
pub const fn command_ids() -> &'static [&'static str] {
    &["library.scan", "library.path"]
}

#[cfg(test)]
mod tests {
    use super::{
        Command, CommandOutput, EngineError, MAX_DIRECTORY_ENTRIES, MAX_LIBRARY_RESULTS,
        scan_library_directory, scan_library_directory_with_limit,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("clipcraft-engine-test-{}-{id}", std::process::id()));
            fs::create_dir(&path).expect("test directory can be created");
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn scan_finds_videos_sorts_them_and_ignores_other_files() {
        let directory = TestDirectory::new();
        fs::write(directory.path().join("zeta.MP4"), b"1234").expect("video fixture writes");
        fs::write(directory.path().join("alpha.mkv"), b"12").expect("video fixture writes");
        fs::write(directory.path().join("notes.txt"), b"ignore").expect("text fixture writes");
        fs::create_dir(directory.path().join("nested")).expect("nested directory creates");

        let result = scan_library_directory(directory.path()).expect("folder scan succeeds");

        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.entries[0].title, "alpha.mkv");
        assert_eq!(result.entries[0].size_bytes, 2);
        assert_eq!(result.entries[1].extension, "mp4");
        assert_eq!(result.entries[1].size_bytes, 4);
    }

    #[test]
    fn scan_rejects_a_file_and_a_missing_folder() {
        let directory = TestDirectory::new();
        let file = directory.path().join("clip.mp4");
        fs::write(&file, b"video").expect("fixture writes");
        assert_eq!(
            scan_library_directory(&file),
            Err(EngineError::NotADirectory)
        );

        let missing = directory.path().join("missing");
        assert_eq!(
            scan_library_directory(&missing),
            Err(EngineError::DirectoryUnavailable)
        );
    }

    #[test]
    fn scan_enforces_directory_and_result_bounds() {
        let directory = TestDirectory::new();
        fs::write(directory.path().join("one.mp4"), b"1").expect("fixture writes");
        fs::write(directory.path().join("two.mp4"), b"2").expect("fixture writes");
        assert_eq!(
            scan_library_directory_with_limit(directory.path(), 1, MAX_LIBRARY_RESULTS),
            Err(EngineError::DirectoryEntryLimit)
        );
        assert_eq!(
            scan_library_directory_with_limit(directory.path(), MAX_DIRECTORY_ENTRIES, 1),
            Err(EngineError::ResultLimit)
        );
    }

    #[cfg(unix)]
    #[test]
    fn scan_ignores_symbolic_links() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let target = directory.path().join("real.mp4");
        let link = directory.path().join("linked.mp4");
        fs::write(&target, b"video").expect("fixture writes");
        symlink(&target, &link).expect("symbolic link creates");

        let result = scan_library_directory(directory.path()).expect("folder scan succeeds");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].title, "real.mp4");
    }

    #[test]
    fn ui_and_cli_share_the_same_typed_command() {
        let directory = TestDirectory::new();
        let command = Command::ScanLibrary {
            directory: directory.0.clone(),
        };
        assert_eq!(command.id(), "library.scan");
        assert!(matches!(
            super::execute(command),
            Ok(CommandOutput::LibraryScan(_))
        ));
    }

    #[test]
    fn local_media_path_command_returns_path_and_rejects_empty_input() {
        let path = PathBuf::from("/clips/example.mp4");
        let command = Command::LocalMediaPath { path: path.clone() };

        assert_eq!(command.id(), "library.path");
        assert_eq!(
            super::execute(command),
            Ok(CommandOutput::LocalMediaPath {
                path: path.to_string_lossy().into_owned()
            })
        );
        assert_eq!(
            super::execute(Command::LocalMediaPath {
                path: PathBuf::new()
            }),
            Err(EngineError::EmptyMediaPath)
        );
    }
}

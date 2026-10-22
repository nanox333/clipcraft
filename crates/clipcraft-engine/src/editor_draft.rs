//! Internal recovery metadata. Sources and ordinary exported videos are never written here.

use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use clipcraft_time::Tick;
use serde::{Deserialize, Serialize};

use crate::{EditorSession, ExportPreset, MAX_LIBRARY_RESULTS};

pub const EDITOR_DRAFT_FILE_NAME: &str = "editor-draft.json";
pub const MAX_EDITOR_DRAFT_BYTES: u64 = 16 * 1024 * 1024;
const FORMAT: &str = "clipcraft-editor-draft";
const VERSION: u32 = 1;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// A private, replaceable recovery record. It is not an import/export project format.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EditorDraft {
    format: String,
    pub version: u32,
    pub session: EditorSession,
    /// Canonical/absolute references only; missing source files remain recoverable references.
    pub media: Vec<PathBuf>,
    pub title: String,
    pub playhead: Tick,
    pub preset: Option<ExportPreset>,
}

impl Default for EditorDraft {
    fn default() -> Self {
        Self {
            format: FORMAT.into(),
            version: VERSION,
            session: EditorSession::default(),
            media: Vec::new(),
            title: "Untitled edit".into(),
            playhead: Tick::ZERO,
            preset: None,
        }
    }
}

impl EditorDraft {
    /// Captures current edits without undo/redo history and clamps a display playhead to them.
    pub fn new(
        session: &EditorSession,
        media: Vec<PathBuf>,
        title: String,
        playhead: Tick,
        preset: Option<ExportPreset>,
    ) -> Result<Self, EditorDraftError> {
        Self {
            session: session
                .validated_recovery_snapshot()
                .map_err(|_| EditorDraftError::Invalid)?,
            media,
            title,
            playhead,
            preset,
            ..Self::default()
        }
        .normalized()
    }

    pub fn is_empty(&self) -> bool {
        self.session.clips().is_empty() && self.session.texts().is_empty() && self.media.is_empty()
    }

    fn normalized(&self) -> Result<Self, EditorDraftError> {
        if self.format != FORMAT {
            return Err(EditorDraftError::Invalid);
        }
        if self.version != VERSION {
            return Err(EditorDraftError::UnsupportedVersion);
        }
        if self.title.chars().count() > 100
            || self.title.chars().any(char::is_control)
            || self.media.len() > MAX_LIBRARY_RESULTS
            || self
                .preset
                .is_some_and(|preset| preset.target_bytes().is_err())
        {
            return Err(EditorDraftError::Invalid);
        }
        let session = self
            .session
            .validated_recovery_snapshot()
            .map_err(|_| EditorDraftError::Invalid)?;
        let mut media = Vec::new();
        let mut seen = HashSet::new();
        for path in self
            .media
            .iter()
            .chain(session.clips().iter().map(|clip| &clip.input))
        {
            if !media_path(path) {
                return Err(EditorDraftError::Invalid);
            }
            if seen.insert(path.clone()) {
                if media.len() == MAX_LIBRARY_RESULTS {
                    return Err(EditorDraftError::Invalid);
                }
                media.push(path.clone());
            }
        }
        let playhead = self.playhead.clamp(Tick::ZERO, session.duration());
        Ok(Self {
            format: FORMAT.into(),
            version: VERSION,
            session,
            media,
            title: self.title.clone(),
            playhead,
            preset: self.preset,
        })
    }
}

fn media_path(path: &Path) -> bool {
    path.is_absolute()
        && path.as_os_str().len() <= 4096
        && path
            .to_str()
            .is_some_and(|s| !s.chars().any(char::is_control))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorDraftError {
    InvalidPath,
    Invalid,
    UnsupportedVersion,
    TooLarge,
    Unreadable,
    SaveFailed,
    DiscardFailed,
}

impl fmt::Display for EditorDraftError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPath => "Editor recovery needs its own editor-draft.json file in a local app-data folder, without symbolic links.",
            Self::Invalid => "The editor recovery data is damaged or outside its limits. It was kept unchanged; discard recovery explicitly to start a new draft.",
            Self::UnsupportedVersion => "This editor recovery data is from an unsupported version. It was kept unchanged; use a compatible ClipCraft version or discard recovery explicitly.",
            Self::TooLarge => "Editor recovery exceeds its 16 MiB metadata limit. The previous draft was kept; use fewer imported references or shorter metadata.",
            Self::Unreadable => "Editor recovery could not be read. Check access to the app-data folder; the saved draft was kept.",
            Self::SaveFailed => "Editor recovery could not be saved. Check free space and app-data access; the previous draft and source videos were kept.",
            Self::DiscardFailed => "Editor recovery could not be discarded. Check access to the app-data folder; source videos were not changed.",
        })
    }
}

impl std::error::Error for EditorDraftError {}

pub fn default_editor_draft_path() -> Option<PathBuf> {
    let path = crate::default_index_path()?.with_file_name(EDITOR_DRAFT_FILE_NAME);
    path.is_absolute().then_some(path)
}

/// Accept only the owned recovery filename and refuse links in existing parent components.
fn guard_path(path: &Path) -> Result<(), EditorDraftError> {
    if !path.is_absolute()
        || path
            .file_name()
            .is_none_or(|name| name != EDITOR_DRAFT_FILE_NAME)
        || path.as_os_str().len() > 4096
        || path
            .to_str()
            .is_none_or(|s| s.chars().any(char::is_control))
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(EditorDraftError::InvalidPath);
    }
    for parent in path.ancestors() {
        match fs::symlink_metadata(parent) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(EditorDraftError::InvalidPath);
            }
            Ok(metadata) if parent == path && !metadata.is_file() => {
                return Err(EditorDraftError::InvalidPath);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(EditorDraftError::Unreadable),
        }
    }
    Ok(())
}

/// Loads one bounded recovery record without changing it on any error.
pub fn load_editor_draft(path: &Path) -> Result<Option<EditorDraft>, EditorDraftError> {
    guard_path(path)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(EditorDraftError::Unreadable),
    };
    let metadata = file.metadata().map_err(|_| EditorDraftError::Unreadable)?;
    if !metadata.is_file() {
        return Err(EditorDraftError::InvalidPath);
    }
    if metadata.len() > MAX_EDITOR_DRAFT_BYTES {
        return Err(EditorDraftError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(MAX_EDITOR_DRAFT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| EditorDraftError::Unreadable)?;
    if bytes.len() as u64 > MAX_EDITOR_DRAFT_BYTES {
        return Err(EditorDraftError::TooLarge);
    }
    let draft: EditorDraft =
        serde_json::from_slice(&bytes).map_err(|_| EditorDraftError::Invalid)?;
    draft.normalized().map(Some)
}

struct BoundedBytes {
    bytes: Vec<u8>,
    overflowed: bool,
}
impl Write for BoundedBytes {
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        if (self.bytes.len() as u64).saturating_add(input.len() as u64) > MAX_EDITOR_DRAFT_BYTES {
            self.overflowed = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "recovery metadata limit",
            ));
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct OwnedTemporary(PathBuf);
impl Drop for OwnedTemporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Atomically replaces valid internal metadata, keeping the previous draft on failure.
pub fn save_editor_draft(
    path: &Path,
    draft: &EditorDraft,
) -> Result<EditorDraft, EditorDraftError> {
    guard_path(path)?;
    let draft = draft.normalized()?;
    // Never overwrite a corrupt, unrelated or future-version record with a fresh draft.
    load_editor_draft(path)?;
    let mut output = BoundedBytes {
        bytes: Vec::new(),
        overflowed: false,
    };
    if serde_json::to_writer(&mut output, &draft).is_err() {
        return Err(if output.overflowed {
            EditorDraftError::TooLarge
        } else {
            EditorDraftError::SaveFailed
        });
    }
    let folder = path.parent().ok_or(EditorDraftError::InvalidPath)?;
    fs::create_dir_all(folder).map_err(|_| EditorDraftError::SaveFailed)?;
    guard_path(path)?;
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temporary_path = folder.join(format!(".editor-draft-{}-{id}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary_path)
        .map_err(|_| EditorDraftError::SaveFailed)?;
    let temporary = OwnedTemporary(temporary_path);
    file.write_all(&output.bytes)
        .map_err(|_| EditorDraftError::SaveFailed)?;
    file.sync_all().map_err(|_| EditorDraftError::SaveFailed)?;
    drop(file);
    guard_path(path)?;
    // The final recheck protects a corrupt/future draft created while the temp was written.
    load_editor_draft(path)?;
    fs::rename(&temporary.0, path).map_err(|_| EditorDraftError::SaveFailed)?;
    Ok(draft)
}

/// Removes only the fixed recovery file, including a corrupt file explicitly discarded by its user.
pub fn discard_editor_draft(path: &Path) -> Result<(), EditorDraftError> {
    guard_path(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(EditorDraftError::DiscardFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EditorAction, Fade, SegmentLook};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let base = fs::canonicalize(std::env::temp_dir()).unwrap();
            let directory = base.join(format!(
                "clipcraft-draft-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            Self(directory)
        }
        fn path(&self) -> PathBuf {
            self.0.join(EDITOR_DRAFT_FILE_NAME)
        }
        fn source(&self) -> PathBuf {
            self.0.join("missing-source.mp4")
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn session(dir: &Scratch) -> EditorSession {
        EditorSession::default()
            .apply(EditorAction::Append {
                input: dir.source(),
                name: "Synthetic source".into(),
                duration: Tick::from_units(5, 1),
            })
            .unwrap()
            .apply(EditorAction::Split {
                at: Tick::from_units(2, 1),
            })
            .unwrap()
            .apply(EditorAction::Look {
                id: 1,
                look: SegmentLook {
                    fade_in: Fade::Black,
                    ..Default::default()
                },
            })
            .unwrap()
            .apply(EditorAction::AddText {
                start: Tick::ZERO,
                duration: Tick::from_units(2, 1),
                text: "Last round".into(),
            })
            .unwrap()
    }
    fn draft(dir: &Scratch) -> EditorDraft {
        EditorDraft::new(
            &session(dir),
            vec![dir.source(), dir.source()],
            "Highlights".into(),
            Tick(i64::MAX),
            Some(ExportPreset::Custom { megabytes: 25 }),
        )
        .unwrap()
    }

    #[test]
    fn recovery_round_trip_keeps_cuts_text_and_choices_but_starts_new_history() {
        let dir = Scratch::new();
        let original = session(&dir);
        assert!(original.can_undo());
        let value = draft(&dir);
        assert_eq!(value.media, vec![dir.source()]);
        assert_eq!(value.playhead, original.duration());
        assert_eq!(value.session.clips(), original.clips());
        assert_eq!(value.session.texts(), original.texts());
        assert!(!value.session.can_undo());
        assert!(!value.session.can_redo());
        assert!(!value.is_empty());
        assert_eq!(save_editor_draft(&dir.path(), &value).unwrap(), value);
        assert_eq!(load_editor_draft(&dir.path()).unwrap(), Some(value.clone()));
        assert!(
            !dir.source().exists(),
            "missing source references are retained without creating media"
        );
        let mut next = value.clone();
        next.title = "Updated title".into();
        save_editor_draft(&dir.path(), &next).unwrap();
        assert_eq!(load_editor_draft(&dir.path()).unwrap(), Some(next));
        assert_eq!(
            fs::read_dir(&dir.0).unwrap().count(),
            1,
            "owned temporaries are removed"
        );
        discard_editor_draft(&dir.path()).unwrap();
        assert_eq!(load_editor_draft(&dir.path()).unwrap(), None);
        discard_editor_draft(&dir.path()).unwrap();
        assert!(EditorDraft::default().is_empty());
    }

    #[test]
    fn hostile_metadata_and_session_state_are_rejected_before_recovery() {
        let dir = Scratch::new();
        let good = serde_json::to_value(draft(&dir)).unwrap();
        let mut invalids = Vec::new();
        let mut bad = good.clone();
        bad["title"] = serde_json::json!("x".repeat(101));
        invalids.push(bad);
        let mut bad = good.clone();
        bad["title"] = serde_json::json!("line\nfeed");
        invalids.push(bad);
        let mut bad = good.clone();
        bad["media"] = serde_json::json!(["relative.mp4"]);
        invalids.push(bad);
        let mut bad = good.clone();
        bad["session"]["next_id"] = serde_json::json!(0);
        invalids.push(bad);
        let mut bad = good.clone();
        bad["session"]["clips"][0]["end"] = serde_json::json!(i64::MAX);
        invalids.push(bad);
        let mut bad = good.clone();
        bad["session"]["texts"][0]["id"] = serde_json::json!(1);
        invalids.push(bad);
        let mut bad = good.clone();
        bad["preset"] = serde_json::json!({"preset":"custom","megabytes":0});
        invalids.push(bad);
        let mut bad = good.clone();
        bad["format"] = serde_json::json!("unrelated data");
        invalids.push(bad);
        let mut bad = good.clone();
        let mut history_clip = bad["session"]["clips"][0].clone();
        history_clip["end"] = serde_json::json!(i64::MAX);
        bad["session"]["undo"] = serde_json::json!([{"clips":[history_clip],"texts":[]}]);
        invalids.push(bad);
        for invalid in invalids {
            let bytes = serde_json::to_vec(&invalid).unwrap();
            fs::write(dir.path(), &bytes).unwrap();
            assert_eq!(
                load_editor_draft(&dir.path()),
                Err(EditorDraftError::Invalid)
            );
            assert_eq!(fs::read(dir.path()).unwrap(), bytes);
        }
        assert!(
            EditorDraft::new(
                &session(&dir),
                vec![dir.source(); MAX_LIBRARY_RESULTS + 1],
                "Too many".into(),
                Tick::ZERO,
                None
            )
            .is_err()
        );
        let negative =
            EditorDraft::new(&session(&dir), vec![], "Start".into(), Tick(-1), None).unwrap();
        assert_eq!(negative.playhead, Tick::ZERO);
    }

    #[test]
    fn corrupt_future_and_oversized_files_stay_unchanged_until_explicit_discard() {
        let dir = Scratch::new();
        let value = draft(&dir);
        let mut future = serde_json::to_value(&value).unwrap();
        future["version"] = serde_json::json!(99);
        let bytes = serde_json::to_vec(&future).unwrap();
        fs::write(dir.path(), &bytes).unwrap();
        assert_eq!(
            load_editor_draft(&dir.path()),
            Err(EditorDraftError::UnsupportedVersion)
        );
        assert_eq!(
            save_editor_draft(&dir.path(), &value),
            Err(EditorDraftError::UnsupportedVersion)
        );
        assert_eq!(fs::read(dir.path()).unwrap(), bytes);
        fs::write(
            dir.path(),
            b"{\"format\":\"clipcraft-editor-draft\",\"version\":",
        )
        .unwrap();
        let truncated = fs::read(dir.path()).unwrap();
        assert_eq!(
            save_editor_draft(&dir.path(), &value),
            Err(EditorDraftError::Invalid)
        );
        assert_eq!(fs::read(dir.path()).unwrap(), truncated);
        let file = File::create(dir.path()).unwrap();
        file.set_len(MAX_EDITOR_DRAFT_BYTES + 1).unwrap();
        assert_eq!(
            load_editor_draft(&dir.path()),
            Err(EditorDraftError::TooLarge)
        );
        assert_eq!(
            fs::metadata(dir.path()).unwrap().len(),
            MAX_EDITOR_DRAFT_BYTES + 1
        );
        discard_editor_draft(&dir.path()).unwrap();
        save_editor_draft(&dir.path(), &value).unwrap();
        assert_eq!(load_editor_draft(&dir.path()).unwrap(), Some(value));
    }

    #[test]
    fn a_failed_large_save_preserves_the_previous_draft_and_all_other_files() {
        let dir = Scratch::new();
        let value = draft(&dir);
        save_editor_draft(&dir.path(), &value).unwrap();
        let old = fs::read(dir.path()).unwrap();
        let mut huge = value.clone();
        let padding = 4090_usize.saturating_sub(dir.0.as_os_str().len() + 16);
        huge.media = (0..MAX_LIBRARY_RESULTS - 1)
            .map(|i| dir.0.join(format!("{}-{i}.mp4", "a".repeat(padding))))
            .collect();
        assert_eq!(
            save_editor_draft(&dir.path(), &huge),
            Err(EditorDraftError::TooLarge)
        );
        assert_eq!(fs::read(dir.path()).unwrap(), old);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
        let unrelated = dir.0.join("ordinary-video.mp4");
        fs::write(&unrelated, b"preserved").unwrap();
        assert_eq!(
            discard_editor_draft(&unrelated),
            Err(EditorDraftError::InvalidPath)
        );
        assert_eq!(
            save_editor_draft(&unrelated, &value),
            Err(EditorDraftError::InvalidPath)
        );
        assert_eq!(fs::read(unrelated).unwrap(), b"preserved");
    }

    #[test]
    fn exclusive_temporary_collisions_preserve_the_previous_record_and_colliding_files() {
        let dir = Scratch::new();
        let value = draft(&dir);
        save_editor_draft(&dir.path(), &value).unwrap();
        let previous = fs::read(dir.path()).unwrap();
        // Other recovery tests can allocate concurrently. Reserve more than their total saves,
        // so whichever ID this operation receives is an existing, unowned temporary file.
        let first = NEXT_TEMP.load(Ordering::Relaxed);
        let collisions: Vec<_> = (first..first + 64)
            .map(|id| {
                let path = dir
                    .0
                    .join(format!(".editor-draft-{}-{id}.tmp", std::process::id()));
                fs::write(&path, b"unowned temporary").unwrap();
                path
            })
            .collect();
        assert_eq!(
            save_editor_draft(&dir.path(), &value),
            Err(EditorDraftError::SaveFailed)
        );
        assert_eq!(fs::read(dir.path()).unwrap(), previous);
        for path in collisions {
            assert_eq!(fs::read(path).unwrap(), b"unowned temporary");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_do_not_redirect_recovery_reads_writes_or_discard() {
        use std::os::unix::fs::symlink;
        let dir = Scratch::new();
        let outside = Scratch::new();
        let source = outside.0.join("other.json");
        fs::write(&source, b"private").unwrap();
        symlink(&source, dir.path()).unwrap();
        let value = draft(&dir);
        assert_eq!(
            load_editor_draft(&dir.path()),
            Err(EditorDraftError::InvalidPath)
        );
        assert_eq!(
            save_editor_draft(&dir.path(), &value),
            Err(EditorDraftError::InvalidPath)
        );
        assert_eq!(
            discard_editor_draft(&dir.path()),
            Err(EditorDraftError::InvalidPath)
        );
        assert_eq!(fs::read(&source).unwrap(), b"private");
        let linked_parent = dir.0.join("linked");
        symlink(&outside.0, &linked_parent).unwrap();
        assert_eq!(
            save_editor_draft(&linked_parent.join(EDITOR_DRAFT_FILE_NAME), &value),
            Err(EditorDraftError::InvalidPath)
        );
        assert!(!outside.path().exists());
    }
}

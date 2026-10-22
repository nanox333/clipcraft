//! Bounded in-memory single-track edits. No document format or source writes.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use clipcraft_time::Tick;
use filmcraft_project::{ClipId, ItemId, Track, TrackId, TrackItem, TrackKind};
use serde::{Deserialize, Serialize};

use crate::EngineError;

// Derived from the duration/frame budget, not a separate user-facing segment cap.
const MAX_TIMELINE_FRAMES: usize = 30 * 1800;
const HISTORY_BYTES: usize = 8 * 1024 * 1024;
const SESSION_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_EDITOR_HISTORY: usize = 64;
pub const MAX_EDITOR_DURATION: Tick = Tick(clipcraft_time::TICKS_PER_SECOND * 1800);
pub const MIN_EDITOR_CLIP: Tick = Tick(clipcraft_time::TICKS_PER_SECOND / 30);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EditorClip {
    pub id: u64,
    pub input: PathBuf,
    pub name: String,
    pub source_duration: Tick,
    pub start: Tick,
    pub end: Tick,
    pub volume_percent: u16,
}
impl EditorClip {
    pub fn duration(&self) -> Tick {
        self.end - self.start
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EditorSession {
    clips: Vec<EditorClip>,
    undo: Vec<Vec<EditorClip>>,
    redo: Vec<Vec<EditorClip>>,
    next_id: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum EditorAction {
    Append {
        input: PathBuf,
        name: String,
        duration: Tick,
    },
    Split {
        at: Tick,
    },
    Trim {
        id: u64,
        start: Tick,
        end: Tick,
    },
    Move {
        id: u64,
        index: usize,
    },
    Duplicate {
        id: u64,
    },
    Delete {
        id: u64,
    },
    Volume {
        id: u64,
        percent: u16,
    },
    Undo,
    Redo,
    Reset,
}

fn invalid() -> EngineError {
    EngineError::EditorInvalid
}

fn metadata_bytes(clips: &[EditorClip]) -> usize {
    clips.iter().fold(0_usize, |sum, c| {
        sum.saturating_add(std::mem::size_of::<EditorClip>())
            .saturating_add(c.name.len())
            .saturating_add(c.input.as_os_str().len())
    })
}

fn validate(clips: &[EditorClip]) -> Result<(), EngineError> {
    if clips.len() > MAX_TIMELINE_FRAMES || metadata_bytes(clips) > SESSION_BYTES {
        return Err(invalid());
    }
    let mut duration = Tick::ZERO;
    let mut ids = HashSet::new();
    for clip in clips {
        if clip.input.as_os_str().is_empty()
            || clip.input.as_os_str().len() > 4096
            || clip.name.is_empty()
            || clip.name.len() > 512
            || clip.name.chars().any(char::is_control)
            || clip.id == 0
            || !ids.insert(clip.id)
            || clip.source_duration > Tick::from_units(86_400, 1)
            || clip.start < Tick::ZERO
            || clip.end > clip.source_duration
            || clip.end <= clip.start
            || clip.duration() < MIN_EDITOR_CLIP
            || clip.volume_percent > 100
        {
            return Err(invalid());
        }
        duration = Tick(
            duration
                .0
                .checked_add(clip.duration().0)
                .ok_or_else(invalid)?,
        );
        if duration > MAX_EDITOR_DURATION {
            return Err(invalid());
        }
    }
    Ok(())
}

impl EditorSession {
    pub fn clips(&self) -> &[EditorClip] {
        &self.clips
    }
    pub fn duration(&self) -> Tick {
        self.clips
            .iter()
            .fold(Tick::ZERO, |sum, c| sum + c.duration())
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// Timeline position resolves to a segment and exact source time; the last endpoint holds.
    pub fn at(&self, position: Tick) -> Option<(&EditorClip, Tick)> {
        let mut offset = Tick::ZERO;
        for (index, clip) in self.clips.iter().enumerate() {
            let end = offset + clip.duration();
            if position < end || index + 1 == self.clips.len() {
                return Some((
                    clip,
                    clip.start + (position - offset).clamp(Tick::ZERO, clip.duration() - Tick(1)),
                ));
            }
            offset = end;
        }
        None
    }
    pub fn offset_of(&self, id: u64) -> Option<Tick> {
        let mut offset = Tick::ZERO;
        for clip in &self.clips {
            if clip.id == id {
                return Some(offset);
            }
            offset += clip.duration();
        }
        None
    }
    fn alloc(&mut self) -> Result<u64, EngineError> {
        self.next_id = self.next_id.checked_add(1).ok_or_else(invalid)?;
        Ok(self.next_id)
    }
    /// Validate caller-supplied state before applying FilmCraft operations to a private copy.
    pub(crate) fn apply(mut self, action: EditorAction) -> Result<Self, EngineError> {
        if self.undo.len() > MAX_EDITOR_HISTORY
            || self.redo.len() > MAX_EDITOR_HISTORY
            || self.undo.len() + self.redo.len() > MAX_EDITOR_HISTORY
        {
            return Err(invalid());
        }
        validate(&self.clips)?;
        if self
            .undo
            .iter()
            .chain(&self.redo)
            .fold(0_usize, |sum, s| sum.saturating_add(metadata_bytes(s)))
            > HISTORY_BYTES
        {
            return Err(invalid());
        }
        for snapshot in self.undo.iter().chain(self.redo.iter()) {
            validate(snapshot)?;
        }
        if self
            .clips
            .iter()
            .chain(self.undo.iter().flatten())
            .chain(self.redo.iter().flatten())
            .any(|c| c.id > self.next_id)
        {
            return Err(invalid());
        }
        if action == EditorAction::Undo {
            if let Some(previous) = self.undo.pop() {
                self.redo.push(std::mem::replace(&mut self.clips, previous));
            }
            self.cap_history();
            return Ok(self);
        }
        if action == EditorAction::Redo {
            if let Some(next) = self.redo.pop() {
                self.undo.push(std::mem::replace(&mut self.clips, next));
            }
            self.cap_history();
            return Ok(self);
        }
        let before = self.clips.clone();
        match action {
            EditorAction::Append {
                input,
                name,
                duration,
            } => {
                let id = self.alloc()?;
                self.clips.push(EditorClip {
                    id,
                    input,
                    name,
                    source_duration: duration,
                    start: Tick::ZERO,
                    end: duration,
                    volume_percent: 100,
                });
            }
            EditorAction::Split { at } => {
                let (clip, source_time) = self.at(at).ok_or_else(invalid)?;
                let clip = clip.clone();
                if at < Tick::ZERO
                    || at >= self.duration()
                    || source_time - clip.start < MIN_EDITOR_CLIP
                    || clip.end - source_time < MIN_EDITOR_CLIP
                {
                    return Err(invalid());
                }
                let index = self.index(clip.id)?;
                let mut track = Track::new(TrackId(1), TrackKind::Video, "V1".into());
                track.items.push(track_item(&clip));
                self.next_id.checked_add(2).ok_or_else(invalid)?;
                let mut next_id = self.next_id + 1;
                let mut ctx = filmcraft_edit::EditCtx {
                    next_id: &mut next_id,
                    media_duration: &|_| Some(clip.source_duration),
                    media_start: &|_| Tick::ZERO,
                    min_duration: MIN_EDITOR_CLIP,
                };
                let right_id = filmcraft_edit::split_track_at(
                    &mut track,
                    source_time - clip.start,
                    &mut ctx,
                    &mut HashMap::new(),
                )
                .ok_or_else(invalid)?;
                self.next_id = next_id;
                let mut right = clip.clone();
                right.id = right_id.0;
                // This slice only supports speed 1. Keep source boundaries in integer ticks,
                // including where FilmCraft's general speed-aware algebra uses floating point.
                right.start = source_time;
                self.clips[index].end = source_time;
                self.clips.insert(index + 1, right);
            }
            EditorAction::Trim { id, start, end } => {
                let index = self.index(id)?;
                self.clips[index].start = start;
                self.clips[index].end = end;
            }
            EditorAction::Move { id, index } => {
                if index >= self.clips.len() {
                    return Err(invalid());
                }
                let from = self.index(id)?;
                let clip = self.clips.remove(from);
                self.clips.insert(index, clip);
            }
            EditorAction::Duplicate { id } => {
                let index = self.index(id)?;
                let mut clip = self.clips[index].clone();
                clip.id = self.alloc()?;
                self.clips.insert(index + 1, clip);
            }
            EditorAction::Delete { id } => {
                let index = self.index(id)?;
                self.clips.remove(index);
            }
            EditorAction::Volume { id, percent } => {
                let index = self.index(id)?;
                self.clips[index].volume_percent = percent;
            }
            EditorAction::Reset => self.clips.clear(),
            EditorAction::Undo | EditorAction::Redo => {}
        }
        validate(&self.clips)?;
        if self.clips != before {
            if self.undo.len() == MAX_EDITOR_HISTORY {
                self.undo.remove(0);
            }
            self.undo.push(before);
            self.redo.clear();
            self.cap_history();
        }
        Ok(self)
    }
    fn index(&self, id: u64) -> Result<usize, EngineError> {
        self.clips
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(invalid)
    }
    fn cap_history(&mut self) {
        while self
            .undo
            .iter()
            .chain(&self.redo)
            .fold(0_usize, |sum, s| sum.saturating_add(metadata_bytes(s)))
            > HISTORY_BYTES
        {
            if !self.undo.is_empty() {
                self.undo.remove(0);
            } else if !self.redo.is_empty() {
                self.redo.remove(0);
            } else {
                break;
            }
        }
    }
}

fn track_item(clip: &EditorClip) -> TrackItem {
    TrackItem {
        id: ClipId(clip.id),
        item: ItemId(clip.id),
        name: clip.name.clone(),
        label: filmcraft_project::Label::Blue,
        start: Tick::ZERO,
        duration: clip.duration(),
        source_in: clip.start,
        speed: 1.0,
        reverse: false,
        enabled: true,
        link: None,
        group: None,
        effects: Vec::new(),
        markers: Vec::new(),
        gain_db: 0.0,
        frame_hold: None,
        scale_to_frame: true,
        essential: None,
        multicam: None,
        time_interpolation: Default::default(),
        hold_filters: false,
        field_options: None,
        source_channels: Vec::new(),
        audio_stream: 0,
        graphic: None,
    }
}

/// Import local references and facts only. An invalid batch leaves the editor library unchanged.
pub(crate) fn import_media(paths: Vec<PathBuf>) -> Result<Vec<crate::MediaEntry>, EngineError> {
    if paths.len() > crate::MAX_LIBRARY_RESULTS {
        return Err(EngineError::ResultLimit);
    }
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        if path.as_os_str().is_empty() || path.as_os_str().len() > 4096 {
            return Err(EngineError::LibraryPathInvalid);
        }
        let path = std::fs::canonicalize(path).map_err(|_| EngineError::FileUnavailable)?;
        let text = path
            .to_str()
            .ok_or(EngineError::LibraryPathInvalid)?
            .to_owned();
        if !seen.insert(text.clone()) {
            continue;
        }
        let metadata = std::fs::metadata(&path).map_err(|_| EngineError::FileUnavailable)?;
        if !metadata.is_file() {
            return Err(EngineError::FileUnavailable);
        }
        let crate::CommandOutput::ClipProbe(probe) =
            crate::execute(crate::Command::ProbeClip { path: path.clone() })?
        else {
            return Err(EngineError::MediaUnsupported);
        };
        if probe.video_codec.is_none()
            || probe.width.is_none_or(|w| w == 0)
            || probe.height.is_none_or(|h| h == 0)
            || probe.duration.is_none_or(|d| d <= Tick::ZERO)
        {
            return Err(EngineError::MediaUnsupported);
        }
        entries.push(crate::MediaEntry {
            path: text,
            title: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            size_bytes: metadata.len(),
            modified_unix_seconds: metadata
                .modified()
                .ok()
                .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            extension: path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase(),
            offline: false,
            favorite: false,
            tags: Vec::new(),
            game: None,
            recorded_game: None,
            collections: Vec::new(),
            probe: Some(probe),
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn append(state: EditorSession) -> EditorSession {
        state
            .apply(EditorAction::Append {
                input: "generated.mp4".into(),
                name: "Generated".into(),
                duration: Tick::from_units(2, 1),
            })
            .unwrap()
    }
    #[test]
    fn filmcraft_split_reorder_trim_delete_and_history_keep_exact_source_ranges() {
        let original = append(EditorSession::default());
        let at = Tick::from_units(1, 1) + Tick(1);
        let split = original.clone().apply(EditorAction::Split { at }).unwrap();
        assert_eq!(split.clips[0].end, at);
        assert_eq!(split.clips[1].start, at);
        assert_eq!(split.duration(), original.duration());
        let id = split.clips[1].id;
        let moved = split.apply(EditorAction::Move { id, index: 0 }).unwrap();
        assert_eq!(moved.at(Tick::ZERO).unwrap().1, at);
        let undo = moved.clone().apply(EditorAction::Undo).unwrap();
        assert_eq!(undo.clips[1].id, id);
        assert_eq!(undo.apply(EditorAction::Redo).unwrap(), moved);
        let trimmed = moved
            .apply(EditorAction::Trim {
                id,
                start: at,
                end: Tick::from_units(1500, 1000),
            })
            .unwrap();
        let removed = trimmed.apply(EditorAction::Delete { id }).unwrap();
        assert_eq!(removed.clips.len(), 1);
        assert!(removed.at(Tick::ZERO).is_some());
    }
    #[test]
    fn invalid_operations_do_not_change_the_original_and_work_is_bounded() {
        let state = append(EditorSession::default());
        for at in [Tick::ZERO, Tick(-1), Tick(i64::MAX), Tick::from_units(2, 1)] {
            assert!(state.clone().apply(EditorAction::Split { at }).is_err());
        }
        assert!(
            state
                .clone()
                .apply(EditorAction::Trim {
                    id: 1,
                    start: Tick(i64::MIN),
                    end: Tick(i64::MAX)
                })
                .is_err()
        );
        assert!(
            state
                .clone()
                .apply(EditorAction::Volume {
                    id: 1,
                    percent: 101
                })
                .is_err()
        );
        let mut repeated = state.clone();
        for _ in 1..32 {
            repeated = append(repeated);
        }
        assert_eq!(append_checked(repeated).unwrap().clips.len(), 33);
        assert!(
            state
                .clone()
                .apply(EditorAction::Append {
                    input: "large.mp4".into(),
                    name: "Long".into(),
                    duration: Tick::from_units(1800, 1)
                })
                .is_err()
        );
        assert_eq!(state.clips.len(), 1);
        let mut history = state;
        for i in 0..100 {
            history = history
                .apply(EditorAction::Volume {
                    id: 1,
                    percent: i % 2,
                })
                .unwrap();
        }
        assert_eq!(history.undo.len(), 64);
        history = history.apply(EditorAction::Undo).unwrap();
        assert!(history.can_redo());
        history = history
            .apply(EditorAction::Volume { id: 1, percent: 42 })
            .unwrap();
        assert!(!history.can_redo());
    }
    #[test]
    fn hostile_identifiers_and_import_paths_are_rejected_without_panic() {
        let mut state = append(EditorSession::default());
        state.next_id = u64::MAX;
        assert!(
            state
                .apply(EditorAction::Split {
                    at: Tick::from_units(1, 1)
                })
                .is_err()
        );
        let mut duplicate = append(EditorSession::default());
        duplicate.clips.push(duplicate.clips[0].clone());
        assert!(duplicate.apply(EditorAction::Undo).is_err());
        assert!(import_media(vec![PathBuf::new()]).is_err());
        assert!(import_media(vec![PathBuf::from("missing-editor-video.mp4")]).is_err());
        assert!(import_media(vec![PathBuf::from(".")]).is_err());
        assert!(import_media(vec![PathBuf::new(); crate::MAX_LIBRARY_RESULTS + 1]).is_err());
        assert!(import_media(Vec::new()).unwrap().is_empty());
    }
    fn append_checked(state: EditorSession) -> Result<EditorSession, EngineError> {
        state.apply(EditorAction::Append {
            input: "generated.mp4".into(),
            name: "Generated".into(),
            duration: Tick::from_units(2, 1),
        })
    }
}

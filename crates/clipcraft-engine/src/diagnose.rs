//! What a person (or someone helping them) needs to know when recording does not work: the
//! system, the folders and files ClipCraft uses, and, filled in by the app or the command line,
//! the screen, sound, and encoder facts only they can look up. Everything is read locally and
//! printed as text to copy; nothing is sent anywhere.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{RecordingSettings, default_capture_folder, default_index_path, default_settings_path};

/// One thing that was checked.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Check {
    /// What was checked.
    pub name: String,
    /// Whether it is as it should be.
    pub ok: bool,
    /// The details, in words.
    pub detail: String,
}

impl Check {
    /// A passed check.
    pub fn pass(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            ok: true,
            detail: detail.into(),
        }
    }

    /// A failed check, and why.
    pub fn fail(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            ok: false,
            detail: detail.into(),
        }
    }
}

/// A report: sections of checks.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Diagnostics {
    /// ClipCraft's version.
    pub version: String,
    /// Operating system and processor type.
    pub system: String,
    /// The folders and files ClipCraft uses.
    pub storage: Vec<Check>,
    /// Screens and windows that can be captured.
    pub capture: Vec<Check>,
    /// Microphones and the computer's own sound.
    pub audio: Vec<Check>,
    /// Video encoders.
    pub video: Vec<Check>,
}

fn file_state(name: &str, path: Option<PathBuf>) -> Check {
    let Some(path) = path else {
        return Check::fail(
            name,
            "no home folder is known, so nothing can be saved there",
        );
    };
    let mut damaged = path.as_os_str().to_owned();
    damaged.push(".damaged");
    let damaged_note = if Path::new(&damaged).exists() {
        " A damaged earlier copy is kept beside it (.damaged)."
    } else {
        ""
    };
    match fs::metadata(&path) {
        Ok(meta) => Check::pass(
            name,
            format!("{} ({} bytes).{damaged_note}", path.display(), meta.len()),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Check::pass(
            name,
            format!(
                "{} does not exist yet; it is made on first use.{damaged_note}",
                path.display()
            ),
        ),
        Err(error) => Check::fail(name, format!("{} cannot be read: {error}", path.display())),
    }
}

/// Whether a file can be made in `folder`, by making and removing one.
fn writable(folder: &Path) -> Result<(), String> {
    let probe = folder.join(".clipcraft-write-test");
    fs::write(&probe, b"x").map_err(|error| error.to_string())?;
    let _ = fs::remove_file(&probe);
    Ok(())
}

/// Counts unfinished save files (`.partial`, `.recovery`) in `folder`.
fn leftovers(folder: &Path) -> usize {
    fs::read_dir(folder).map_or(0, |listing| {
        listing
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.ends_with(".partial") || name.ends_with(".recovery")
            })
            .count()
    })
}

impl Diagnostics {
    /// The system, folders, and files; the other sections are left for the caller.
    pub fn collect(settings: &RecordingSettings) -> Self {
        let mut report = Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            system: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            ..Self::default()
        };
        let folder = if settings.capture_folder.is_empty() {
            default_capture_folder()
        } else {
            Some(PathBuf::from(&settings.capture_folder))
        };
        report.storage.push(match folder {
            None => Check::fail("Clips folder", "no folder is chosen and none can be picked by default"),
            Some(folder) => {
                let usage = crate::storage_usage(&folder);
                let room = usage
                    .free_bytes
                    .map_or_else(|| "free space unknown".to_owned(), |free| format!("{} free", crate::format_bytes(free)));
                if folder.exists() && !folder.is_dir() {
                    Check::fail("Clips folder", format!("{} is a file, not a folder", folder.display()))
                } else if !usage.exists {
                    Check::pass("Clips folder", format!("{} does not exist yet; it is made when the first clip is saved ({room}).", folder.display()))
                } else {
                    match writable(&folder) {
                        Ok(()) => {
                            let extra = match leftovers(&folder) {
                                0 => String::new(),
                                n => format!(" {n} unfinished save file(s) are waiting to be cleaned up."),
                            };
                            Check::pass(
                                "Clips folder",
                                format!("{} holds {} clips, {room}.{extra}", folder.display(), usage.clip_count),
                            )
                        }
                        Err(error) => Check::fail(
                            "Clips folder",
                            format!("{} cannot be written to: {error}", folder.display()),
                        ),
                    }
                }
            }
        });
        report
            .storage
            .push(file_state("Settings file", default_settings_path()));
        report
            .storage
            .push(file_state("Library index", default_index_path()));
        report
    }

    /// The report as plain text, for pasting into a message.
    pub fn text(&self) -> String {
        let mut out = format!("ClipCraft {} on {}\n", self.version, self.system);
        for (title, checks) in [
            ("Storage", &self.storage),
            ("Monitor capture", &self.capture),
            ("Sound", &self.audio),
            ("Video encoder", &self.video),
        ] {
            if checks.is_empty() {
                continue;
            }
            out.push_str(&format!("\n{title}\n"));
            for check in checks {
                out.push_str(&format!(
                    "  [{}] {}: {}\n",
                    if check.ok { "ok" } else { "!!" },
                    check.name,
                    check.detail
                ));
            }
        }
        out
    }

    /// True when every check passed.
    pub fn healthy(&self) -> bool {
        [&self.storage, &self.capture, &self.audio, &self.video]
            .into_iter()
            .flatten()
            .all(|check| check.ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_capture_folder_is_fine_and_an_unwritable_one_is_reported() {
        let dir = std::env::temp_dir().join(format!("clipcraft-diagnose-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let settings = RecordingSettings {
            capture_folder: dir.to_string_lossy().into_owned(),
            ..RecordingSettings::default()
        };
        let missing = Diagnostics::collect(&settings);
        assert!(missing.storage[0].ok && missing.storage[0].detail.contains("does not exist yet"));

        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("ClipCraft x.mp4.partial"), b"x").expect("leftover");
        let present = Diagnostics::collect(&settings);
        assert!(present.storage[0].ok);
        assert!(
            present.storage[0].detail.contains("1 unfinished save file"),
            "{}",
            present.storage[0].detail
        );
        assert!(
            !dir.join(".clipcraft-write-test").exists(),
            "the write test cleans up after itself"
        );

        // A path that is a file, not a folder, cannot hold clips.
        let file = dir.join("not-a-folder");
        fs::write(&file, b"x").expect("file");
        let broken = Diagnostics::collect(&RecordingSettings {
            capture_folder: file.to_string_lossy().into_owned(),
            ..RecordingSettings::default()
        });
        assert!(
            !broken.healthy()
                || !broken.storage[0].ok
                || broken.storage[0].detail.contains("does not exist")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_text_lists_each_section_with_ok_and_problem_marks() {
        let mut report = Diagnostics {
            version: "1.2.3".to_owned(),
            system: "linux x86_64".to_owned(),
            ..Diagnostics::default()
        };
        report.capture.push(Check::pass("Monitors", "1 found"));
        report.audio.push(Check::fail("Microphone", "none found"));
        let text = report.text();
        assert!(text.starts_with("ClipCraft 1.2.3 on linux x86_64"));
        assert!(text.contains("[ok] Monitors: 1 found"));
        assert!(text.contains("[!!] Microphone: none found"));
        assert!(
            !text.contains("Video encoder"),
            "empty sections are left out"
        );
        assert!(!report.healthy());
    }
}

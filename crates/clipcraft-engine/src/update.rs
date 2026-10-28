//! Looking for a newer ClipCraft and fetching its setup program (ADR 0032).
//!
//! The only network use in ClipCraft, and always optional: one request for the latest published
//! release (nothing about the person or the computer is sent), and, only when the person asks,
//! the setup program plus its checksum list. The download is checked against `SHA256SUMS.txt` before
//! it can be run. The engine cannot reach the network itself (that code lives a layer up), so the
//! program registers a fetcher at start-up; without one every update command says so.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::EngineError;

/// Where the latest published release is described.
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/nanox333/clipcraft/releases/latest";
/// Downloads must come from this project's releases and nowhere else.
const DOWNLOAD_PREFIX: &str = "https://github.com/nanox333/clipcraft/releases/download/";
/// The release description is small.
const MAX_RELEASE_BYTES: u64 = 2 * 1024 * 1024;
/// The checksum list is tiny.
const MAX_SUMS_BYTES: u64 = 64 * 1024;
/// The setup program is about 40 MB; this leaves generous room and still bounds the download.
const MAX_SETUP_BYTES: u64 = 400 * 1024 * 1024;

/// Fetches a web address, giving up past the byte limit.
pub type HttpFetcher = fn(&str, u64) -> Result<Vec<u8>, String>;

static FETCHER: Mutex<Option<HttpFetcher>> = Mutex::new(None);

/// Registers how this program fetches web addresses over HTTPS.
pub fn set_http_fetcher(fetcher: HttpFetcher) {
    if let Ok(mut slot) = FETCHER.lock() {
        *slot = Some(fetcher);
    }
}

fn fetch(url: &str, limit: u64) -> Result<Vec<u8>, EngineError> {
    let fetcher = FETCHER
        .lock()
        .ok()
        .and_then(|slot| *slot)
        .ok_or(EngineError::UpdateUnavailable)?;
    fetcher(url, limit).map_err(|_| EngineError::UpdateUnavailable)
}

/// A newer release and where to get it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UpdateInfo {
    /// The version number, such as `0.2.0`.
    pub version: String,
    /// What changed, as the release describes it.
    pub notes: String,
    /// The release's web page.
    pub page_url: String,
    /// The setup program's file name.
    pub setup_name: String,
    /// Where the setup program is downloaded from.
    pub setup_url: String,
    /// Where the checksum list is downloaded from.
    pub sums_url: String,
}

/// The answer to "is there a newer ClipCraft?".
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", content = "update", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// This is the newest published version.
    UpToDate,
    /// A newer version exists.
    Available(UpdateInfo),
}

/// `1.2.3` as numbers, ignoring a leading `v` and anything after a `-` or `+`.
fn numbers(version: &str) -> Option<Vec<u64>> {
    let core = version.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    let parts: Option<Vec<u64>> = core.split('.').map(|part| part.parse().ok()).collect();
    parts.filter(|parts| !parts.is_empty() && parts.len() <= 4)
}

/// True when `candidate` is a higher version than `current`. Anything unreadable is not newer.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (Some(mut candidate), Some(mut current)) = (numbers(candidate), numbers(current)) else {
        return false;
    };
    let length = candidate.len().max(current.len());
    candidate.resize(length, 0);
    current.resize(length, 0);
    candidate > current
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Reads the latest-release answer. A release with no Windows setup program is not an update.
fn read_release(body: &[u8], current: &str) -> Result<UpdateStatus, EngineError> {
    let release: Release = serde_json::from_slice(body).map_err(|_| EngineError::UpdateInvalid)?;
    if release.draft || release.prerelease || !is_newer(&release.tag_name, current) {
        return Ok(UpdateStatus::UpToDate);
    }
    let find = |wanted: &dyn Fn(&str) -> bool| {
        release.assets.iter().find(|asset| {
            wanted(&asset.name) && asset.browser_download_url.starts_with(DOWNLOAD_PREFIX)
        })
    };
    let setup = find(&|name| name.starts_with("ClipCraft-Setup-") && name.ends_with(".exe"));
    let sums = find(&|name| name == "SHA256SUMS.txt");
    let (Some(setup), Some(sums)) = (setup, sums) else {
        return Ok(UpdateStatus::UpToDate);
    };
    Ok(UpdateStatus::Available(UpdateInfo {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_owned(),
        notes: release.body.chars().take(4000).collect(),
        page_url: if release
            .html_url
            .starts_with("https://github.com/nanox333/clipcraft/")
        {
            release.html_url
        } else {
            "https://github.com/nanox333/clipcraft/releases".to_owned()
        },
        setup_name: setup.name.clone(),
        setup_url: setup.browser_download_url.clone(),
        sums_url: sums.browser_download_url.clone(),
    }))
}

/// Asks whether a version newer than `current` has been published.
pub(crate) fn check(current: &str) -> Result<UpdateStatus, EngineError> {
    let body = fetch(LATEST_RELEASE_URL, MAX_RELEASE_BYTES)?;
    read_release(&body, current)
}

/// The expected checksum of `name` in a `sha256sum` list.
fn expected_checksum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        let file = file.trim().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

/// A file name that cannot point anywhere but into the update folder.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !name.starts_with('.')
}

/// Downloads the setup program, checks it against the published checksum, and returns where it is.
pub(crate) fn download(update: &UpdateInfo) -> Result<PathBuf, EngineError> {
    if !update.setup_url.starts_with(DOWNLOAD_PREFIX)
        || !update.sums_url.starts_with(DOWNLOAD_PREFIX)
        || !safe_name(&update.setup_name)
        || !update.setup_name.ends_with(".exe")
    {
        return Err(EngineError::UpdateInvalid);
    }
    let sums = fetch(&update.sums_url, MAX_SUMS_BYTES)?;
    let sums = String::from_utf8_lossy(&sums);
    let wanted = expected_checksum(&sums, &update.setup_name).ok_or(EngineError::UpdateChecksum)?;
    let bytes = fetch(&update.setup_url, MAX_SETUP_BYTES)?;
    if format!("{:x}", Sha256::digest(&bytes)) != wanted {
        return Err(EngineError::UpdateChecksum);
    }
    let folder = std::env::temp_dir().join("ClipCraft-update");
    fs::create_dir_all(&folder).map_err(|_| EngineError::UpdateUnavailable)?;
    let path = folder.join(&update.setup_name);
    let partial = folder.join(format!("{}.partial", update.setup_name));
    fs::write(&partial, &bytes).map_err(|_| EngineError::UpdateUnavailable)?;
    fs::rename(&partial, &path).map_err(|_| EngineError::UpdateUnavailable)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("not a version", "0.1.0"));
        assert!(!is_newer("0.2.0", "garbage"));
    }

    fn release(tag: &str, extra: &str) -> Vec<u8> {
        format!(
            r#"{{"tag_name":"{tag}","body":"Notes","html_url":"https://github.com/nanox333/clipcraft/releases/tag/{tag}",{extra}
            "assets":[
              {{"name":"ClipCraft-Setup-0.2.0.exe","browser_download_url":"https://github.com/nanox333/clipcraft/releases/download/{tag}/ClipCraft-Setup-0.2.0.exe"}},
              {{"name":"SHA256SUMS.txt","browser_download_url":"https://github.com/nanox333/clipcraft/releases/download/{tag}/SHA256SUMS.txt"}}]}}"#
        )
        .into_bytes()
    }

    #[test]
    fn a_newer_release_with_a_setup_program_is_an_update() {
        let UpdateStatus::Available(update) =
            read_release(&release("v0.2.0", ""), "0.1.0").unwrap()
        else {
            panic!("expected an update");
        };
        assert_eq!(update.version, "0.2.0");
        assert_eq!(update.setup_name, "ClipCraft-Setup-0.2.0.exe");
        assert_eq!(
            read_release(&release("v0.1.0", ""), "0.1.0").unwrap(),
            UpdateStatus::UpToDate
        );
        assert_eq!(
            read_release(&release("v0.2.0", r#""prerelease":true,"#), "0.1.0").unwrap(),
            UpdateStatus::UpToDate
        );
        assert_eq!(
            read_release(b"nonsense", "0.1.0"),
            Err(EngineError::UpdateInvalid)
        );
    }

    #[test]
    fn downloads_from_other_places_are_refused_and_checksums_are_read_exactly() {
        let mut update = match read_release(&release("v0.2.0", ""), "0.1.0").unwrap() {
            UpdateStatus::Available(update) => update,
            UpdateStatus::UpToDate => panic!("expected an update"),
        };
        update.setup_url = "https://example.com/ClipCraft-Setup-0.2.0.exe".into();
        assert_eq!(download(&update), Err(EngineError::UpdateInvalid));
        update.setup_url = format!("{DOWNLOAD_PREFIX}v0.2.0/x.exe");
        update.setup_name = "..\\evil.exe".into();
        assert_eq!(download(&update), Err(EngineError::UpdateInvalid));

        let hash = "a".repeat(64);
        let sums =
            format!("{hash}  ClipCraft-Setup-0.2.0.exe\n{hash} *other.zip\nbad  short.exe\n");
        assert_eq!(
            expected_checksum(&sums, "ClipCraft-Setup-0.2.0.exe"),
            Some(hash.clone())
        );
        assert_eq!(expected_checksum(&sums, "other.zip"), Some(hash));
        assert_eq!(expected_checksum(&sums, "short.exe"), None);
        assert_eq!(expected_checksum(&sums, "missing.exe"), None);
    }

    #[test]
    fn a_download_must_match_its_published_checksum() {
        fn fetcher(url: &str, _: u64) -> Result<Vec<u8>, String> {
            if url.ends_with("SHA256SUMS.txt") {
                Ok(format!("{}  ClipCraft-Setup-0.2.0.exe\n", "0".repeat(64)).into_bytes())
            } else {
                Ok(b"setup program".to_vec())
            }
        }
        set_http_fetcher(fetcher);
        let update = match read_release(&release("v0.2.0", ""), "0.1.0").unwrap() {
            UpdateStatus::Available(update) => update,
            UpdateStatus::UpToDate => panic!("expected an update"),
        };
        assert_eq!(download(&update), Err(EngineError::UpdateChecksum));
    }
}

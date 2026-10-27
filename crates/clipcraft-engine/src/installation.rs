//! Bounded offline bundle format and owned-file installation (ADR 0031).
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 16] = b"CUT-INSTALL-0001";
const LIMIT: u64 = 512 * 1024 * 1024;
const MARKER: &str = ".clipcraft-install.json";
/// Only these files may be unpacked or removed. No archive-controlled paths are accepted.
pub const FILES: [&str; 7] = [
    "bin/clipcraft.exe",
    "bin/clipcraft-cli.exe",
    "README.md",
    "ATTRIBUTION.md",
    "LICENSE-MIT",
    "THIRD-PARTY-LICENSES.txt",
    "docs/licenses/nnnoiseless-BSD-3-Clause.txt",
];

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    releases: Vec<String>,
}

fn io(error: std::io::Error) -> String {
    format!(
        "Installation could not finish ({:?}). Close ClipCraft, check free disk space and folder permissions, then retry. Your clips and settings are kept.",
        error.kind()
    )
}

fn plain_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.parent().is_none() {
        return Err("Choose an absolute application folder, not a drive root.".into());
    }
    for parent in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(parent) {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("Choose a folder without junctions or symbolic links.".into());
                }
            }
            if metadata.is_symlink() {
                return Err("Choose a folder without symbolic links.".into());
            }
        }
    }
    Ok(())
}

fn manifest(root: &Path) -> Result<Manifest, String> {
    plain_path(root)?;
    let path = root.join(MARKER);
    plain_path(&path)?;
    let bytes = read_capped(&path, 16_384)?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|_| "The installation record is damaged; no files were removed.".to_owned())?;
    if manifest.version != 1
        || manifest.releases.is_empty()
        || manifest.releases.len() > 8
        || manifest.releases.iter().any(|name| {
            !name.starts_with("release-")
                || name.len() > 64
                || !name[8..].bytes().all(|b| b.is_ascii_digit())
                || name.len() == 8
        })
    {
        return Err("The installation record is invalid; no files were removed.".into());
    }
    Ok(manifest)
}

fn read_capped(path: &Path, cap: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io)?
        .take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > cap {
        return Err("The installation file exceeds its size limit.".into());
    }
    Ok(bytes)
}

/// Appends a deterministic gzip payload and a versioned length footer to the installer stub.
pub fn bundle(stub: &Path, payload: &Path, destination: &Path) -> Result<(), String> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(io)?;
    std::io::copy(&mut File::open(stub).map_err(io)?, &mut output).map_err(io)?;
    let start = output.stream_position().map_err(io)?;
    let mut gzip = GzEncoder::new(output, Compression::default());
    gzip.write_all(&(FILES.len() as u32).to_le_bytes())
        .map_err(io)?;
    let mut total = 0u64;
    for name in FILES {
        let mut file = File::open(payload.join(name)).map_err(io)?;
        let length = file.metadata().map_err(io)?.len();
        total = total
            .checked_add(length)
            .filter(|n| *n <= LIMIT)
            .ok_or("The application payload is too large.")?;
        gzip.write_all(&length.to_le_bytes()).map_err(io)?;
        let copied =
            std::io::copy(&mut Read::by_ref(&mut file).take(length), &mut gzip).map_err(io)?;
        if copied != length {
            return Err("An application file changed during packaging.".into());
        }
    }
    let mut output = gzip.finish().map_err(io)?;
    let length = output
        .stream_position()
        .map_err(io)?
        .checked_sub(start)
        .ok_or("Invalid bundle length.")?;
    output.write_all(&length.to_le_bytes()).map_err(io)?;
    output.write_all(MAGIC).map_err(io)?;
    output.sync_all().map_err(io)
}

fn payload(file: &mut File) -> Result<(u64, u64), String> {
    let length = file.metadata().map_err(io)?.len();
    if length < 24 {
        return Err(
            "This installer has no application payload. Download the full setup executable.".into(),
        );
    }
    file.seek(SeekFrom::End(-24)).map_err(io)?;
    let mut footer = [0; 24];
    file.read_exact(&mut footer).map_err(io)?;
    let mut size = [0; 8];
    size.copy_from_slice(&footer[..8]);
    let size = u64::from_le_bytes(size);
    if &footer[8..] != MAGIC || size == 0 || size > LIMIT || size > length - 24 {
        return Err(
            "This installer is incomplete or damaged. Download it again and verify its checksum."
                .into(),
        );
    }
    Ok((length - 24 - size, size))
}

/// Installs into a new version directory. The previous install record changes only after all writes succeed.
pub(crate) fn install(bundle: &Path, root: &Path) -> Result<PathBuf, String> {
    plain_path(root)?;
    let mut record = if root.join(MARKER).exists() {
        manifest(root)?
    } else {
        if root.exists() && fs::read_dir(root).map_err(io)?.next().is_some() {
            return Err(
                "Choose an empty application folder. Existing files will not be overwritten."
                    .into(),
            );
        }
        Manifest {
            version: 1,
            releases: Vec::new(),
        }
    };
    if record.releases.len() >= 8 {
        return Err("Eight application versions are installed. Uninstall ClipCraft before reinstalling; clips and settings will be kept.".into());
    }
    let mut source = File::open(bundle).map_err(io)?;
    let (start, length) = payload(&mut source)?;
    fs::create_dir_all(root).map_err(io)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "The system clock is invalid.")?
        .as_nanos();
    let name = format!("release-{stamp}");
    let release = root.join(&name);
    fs::create_dir(&release).map_err(io)?;
    let result = (|| {
        source.seek(SeekFrom::Start(start)).map_err(io)?;
        let mut gzip = GzDecoder::new(Read::by_ref(&mut source).take(length));
        let mut count = [0; 4];
        gzip.read_exact(&mut count).map_err(io)?;
        if u32::from_le_bytes(count) != FILES.len() as u32 {
            return Err("The application payload version is unsupported.".into());
        }
        let mut total = 0u64;
        for name in FILES {
            let mut size = [0; 8];
            gzip.read_exact(&mut size).map_err(io)?;
            let size = u64::from_le_bytes(size);
            total = total
                .checked_add(size)
                .filter(|n| *n <= LIMIT)
                .ok_or("The unpacked application exceeds its size limit.")?;
            let path = release.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(io)?;
            }
            let mut target = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)
                .map_err(io)?;
            if std::io::copy(&mut gzip.by_ref().take(size), &mut target).map_err(io)? != size {
                return Err("The application payload is truncated.".into());
            }
            target.sync_all().map_err(io)?;
        }
        let mut extra = [0; 1];
        if gzip.read(&mut extra).map_err(io)? != 0 {
            return Err("Unexpected data in the application payload.".into());
        }
        drop(gzip);
        source.seek(SeekFrom::Start(0)).map_err(io)?;
        let mut uninstaller = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(release.join("uninstall.exe"))
            .map_err(io)?;
        if std::io::copy(&mut source.take(start), &mut uninstaller).map_err(io)? != start {
            return Err("The uninstaller could not be copied.".into());
        }
        uninstaller.sync_all().map_err(io)?;
        record.releases.push(name);
        let bytes = serde_json::to_vec(&record)
            .map_err(|_| "The installation record could not be saved.")?;
        let temporary = root.join(".clipcraft-install.pending");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(io)?;
        let write = file.write_all(&bytes).and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = write.and_then(|()| fs::rename(&temporary, root.join(MARKER))) {
            let _ = fs::remove_file(temporary);
            return Err(io(error));
        }
        Ok(release.join("bin/clipcraft.exe"))
    })();
    if result.is_err() {
        let _ = remove_release(&release);
    }
    result
}

fn remove_release(release: &Path) -> Result<(), String> {
    // Preflight the entire fixed tree before deleting anything; junctions must never be traversed.
    for name in FILES.into_iter().chain(["uninstall.exe"]) {
        plain_path(&release.join(name))?;
    }
    for name in FILES.into_iter().chain(["uninstall.exe"]) {
        match fs::remove_file(release.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
    }
    for name in ["docs/licenses", "docs", "bin", ""] {
        let _ = fs::remove_dir(release.join(name));
    }
    Ok(())
}

pub(crate) fn uninstall(root: &Path) -> Result<(), String> {
    let record = manifest(root)?;
    for name in &record.releases {
        for file in FILES.into_iter().chain(["uninstall.exe"]) {
            plain_path(&root.join(name).join(file))?;
        }
    }
    for name in record.releases {
        remove_release(&root.join(name))?;
    }
    fs::remove_file(root.join(MARKER)).map_err(io)?;
    // Unknown files and all data outside the installation are deliberately preserved.
    let _ = fs::remove_dir(root);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cut-install-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn install_upgrade_uninstall_preserve_unknown_files_and_settings() {
        let base = fixture();
        let files = base.join("payload");
        for name in FILES {
            let path = files.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, name).unwrap();
        }
        fs::write(base.join("stub"), b"test stub").unwrap();
        let setup = base.join("setup.exe");
        bundle(&base.join("stub"), &files, &setup).unwrap();
        let root = base.join("installed");
        let first = install(&setup, &root).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"bin/clipcraft.exe");
        let second = install(&setup, &root).unwrap();
        assert_ne!(first, second);
        assert!(first.exists());
        fs::write(root.join("my-clip.mp4"), b"keep").unwrap();
        uninstall(&root).unwrap();
        assert!(!first.exists());
        assert!(!second.exists());
        assert_eq!(fs::read(root.join("my-clip.mp4")).unwrap(), b"keep");
        assert!(install(&setup, &root).is_err());
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn malformed_bundles_and_hostile_records_do_not_touch_existing_files() {
        let base = fixture();
        let root = base.join("installed");
        fs::create_dir(&root).unwrap();
        let bundle = base.join("bad");
        fs::write(&bundle, [0; 24]).unwrap();
        assert!(install(&bundle, &root).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::write(
            root.join(MARKER),
            br#"{"version":1,"releases":["release-../../outside"]}"#,
        )
        .unwrap();
        assert!(uninstall(&root).is_err());
        assert!(root.join(MARKER).exists());
        fs::write(root.join(MARKER), [b'x'; 20_000]).unwrap();
        assert!(uninstall(&root).is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn truncated_and_oversized_payloads_roll_back_and_leave_a_previous_install_usable() {
        let base = fixture();
        let files = base.join("payload");
        for name in FILES {
            let path = files.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"original").unwrap();
        }
        fs::write(base.join("stub"), b"stub").unwrap();
        let setup = base.join("setup");
        bundle(&base.join("stub"), &files, &setup).unwrap();
        let root = base.join("app");
        let executable = install(&setup, &root).unwrap();
        let record = fs::read(root.join(MARKER)).unwrap();
        for oversized in [false, true] {
            let bad = base.join(if oversized { "oversized" } else { "truncated" });
            let mut encoded = GzEncoder::new(Vec::new(), Compression::default());
            encoded
                .write_all(&(FILES.len() as u32).to_le_bytes())
                .unwrap();
            encoded
                .write_all(&(if oversized { LIMIT + 1 } else { 100 }).to_le_bytes())
                .unwrap();
            let bytes = encoded.finish().unwrap();
            let mut file = File::create(&bad).unwrap();
            file.write_all(b"stub").unwrap();
            file.write_all(&bytes).unwrap();
            file.write_all(&(bytes.len() as u64).to_le_bytes()).unwrap();
            file.write_all(MAGIC).unwrap();
            drop(file);
            assert!(install(&bad, &root).is_err());
            assert_eq!(fs::read(root.join(MARKER)).unwrap(), record);
            assert_eq!(fs::read(&executable).unwrap(), b"original");
            assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        }
        fs::remove_dir_all(base).unwrap();
    }
}

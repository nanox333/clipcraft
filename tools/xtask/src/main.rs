use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type TaskResult<T> = Result<T, TaskError>;

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: Vec<OsString>) -> TaskResult<()> {
    let task = arguments
        .first()
        .and_then(|argument| argument.to_str())
        .ok_or(TaskError::Usage)?;
    match task {
        "ci" => ci(),
        "docs-check" => {
            if arguments.len() != 2 {
                return Err(TaskError::DocsCheckUsage);
            }
            let base_ref = arguments[1]
                .to_str()
                .filter(|value| !value.is_empty())
                .ok_or(TaskError::DocsCheckUsage)?;
            docs_check(base_ref)
        }
        "layers" => check_layers(),
        "assets" => check_assets(),
        "licenses" => check_licenses(),
        "package" => package(),
        "help" | "--help" | "-h" => {
            println!(
                "ClipCraft Rust tasks: ci, docs-check <base-ref>, layers, assets, licenses, package"
            );
            Ok(())
        }
        _ => Err(TaskError::Usage),
    }
}

fn ci() -> TaskResult<()> {
    run_cargo(&["fmt", "--all", "--", "--check"])?;
    run_cargo(&["test", "--workspace", "--all-targets"])?;
    run_cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    check_layers()?;
    check_assets()?;
    check_licenses()
}

/// Enforces the minimum user-facing documentation updates for implementation pull requests.
///
/// The base ref must be fetched before running this command. Three-dot diff compares the PR head
/// to the merge base, so it works both with a merge checkout and a normal local feature branch.
fn docs_check(base_ref: &str) -> TaskResult<()> {
    let workspace = workspace_root()?;
    let range = format!("{base_ref}...HEAD");
    let mut changed_paths = git_paths(&workspace, &["diff", "--name-only", "-z", &range])?;
    // Include local staged/unstaged work and new files so contributors can run this before commit.
    changed_paths.extend(git_paths(
        &workspace,
        &["diff", "--name-only", "-z", "HEAD"],
    )?);
    changed_paths.extend(git_paths(
        &workspace,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?);
    let missing = missing_documentation(&changed_paths);
    if missing.is_empty() {
        println!("Documentation check passed.");
        Ok(())
    } else {
        for path in missing {
            eprintln!("implementation changes require an update to {path}");
        }
        Err(TaskError::DocumentationMissing)
    }
}

fn git_paths(workspace: &Path, arguments: &[&str]) -> TaskResult<Vec<String>> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(arguments)
        .output()
        .map_err(|_| TaskError::CommandUnavailable)?;
    if !output.status.success() {
        if !output.stderr.is_empty() {
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
        }
        return Err(TaskError::GitDiffFailed);
    }

    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).replace('\\', "/"))
        .collect())
}

fn missing_documentation(changed_paths: &[String]) -> Vec<&'static str> {
    let has_implementation = changed_paths.iter().any(|path| {
        path.starts_with("apps/")
            || path.starts_with("crates/")
            || path.starts_with("tools/")
            || matches!(path.as_str(), "Cargo.toml" | "Cargo.lock")
    });
    if !has_implementation {
        return Vec::new();
    }

    let has_readme = changed_paths.iter().any(|path| path == "README.md");
    let has_gaps = changed_paths.iter().any(|path| path == "docs/gaps.md");
    let mut missing = Vec::new();
    if !has_readme {
        missing.push("README.md");
    }
    if !has_gaps {
        missing.push("docs/gaps.md");
    }
    missing
}

fn run_cargo(arguments: &[&str]) -> TaskResult<()> {
    let workspace = workspace_root()?;
    let status = Command::new("cargo")
        .current_dir(workspace)
        .args(arguments)
        .status()
        .map_err(|_| TaskError::CommandUnavailable)?;
    if status.success() {
        Ok(())
    } else {
        Err(TaskError::CommandFailed)
    }
}

fn check_layers() -> TaskResult<()> {
    let workspace = workspace_root()?;
    let output = Command::new("cargo")
        .current_dir(&workspace)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|_| TaskError::CommandUnavailable)?;
    if !output.status.success() {
        return Err(TaskError::MetadataUnavailable);
    }
    let metadata: Value =
        serde_json::from_slice(&output.stdout).map_err(|_| TaskError::MetadataInvalid)?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or(TaskError::MetadataInvalid)?;
    let violations = find_layer_violations(packages)?;
    if violations.is_empty() {
        println!(
            "Layer check passed for {} workspace packages.",
            packages.len()
        );
        Ok(())
    } else {
        for violation in violations {
            eprintln!("layer violation: {violation}");
        }
        Err(TaskError::LayerViolation)
    }
}

fn find_layer_violations(packages: &[Value]) -> TaskResult<Vec<String>> {
    let mut layers = BTreeMap::new();
    for package in packages {
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .ok_or(TaskError::MetadataInvalid)?;
        let layer = package
            .get("metadata")
            .and_then(|value| value.get("clipcraft"))
            .and_then(|value| value.get("layer"))
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .ok_or(TaskError::MissingLayer)?;
        layers.insert(name.to_owned(), layer);
    }

    let mut violations = Vec::new();
    for package in packages {
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .ok_or(TaskError::MetadataInvalid)?;
        let layer = *layers.get(name).ok_or(TaskError::MissingLayer)?;
        let dependencies = package
            .get("dependencies")
            .and_then(Value::as_array)
            .ok_or(TaskError::MetadataInvalid)?;

        for dependency in dependencies {
            if !dependency.get("kind").is_some_and(Value::is_null) {
                continue;
            }
            let dependency_name = dependency
                .get("name")
                .and_then(Value::as_str)
                .ok_or(TaskError::MetadataInvalid)?;
            if !dependency_name.starts_with("clipcraft") {
                continue;
            }
            let Some(dependency_layer) = layers.get(dependency_name) else {
                violations.push(format!(
                    "{name} depends on unregistered workspace package {dependency_name}"
                ));
                continue;
            };
            if layer < *dependency_layer {
                violations.push(format!(
                    "{name} (layer {layer}) depends on higher layer {dependency_name} (layer {dependency_layer})"
                ));
            }
        }
    }
    Ok(violations)
}

fn check_assets() -> TaskResult<()> {
    let workspace = workspace_root()?;
    let attribution = fs::read_to_string(workspace.join("ATTRIBUTION.md"))
        .map_err(|_| TaskError::AttributionMissing)?;
    let assets = workspace.join("assets");
    if !assets.exists() {
        println!("Asset attribution check passed; no shipped assets directory exists.");
        return Ok(());
    }

    let mut files = Vec::new();
    collect_assets(&assets, &mut files)?;
    let mut missing = Vec::new();
    for path in files {
        if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
            continue;
        }
        let relative = path
            .strip_prefix(&workspace)
            .map_err(|_| TaskError::AssetPath)?;
        let sidecar = PathBuf::from(format!("{}.license.md", path.to_string_lossy()));
        // ATTRIBUTION.md writes paths with `/`, whatever the operating system uses.
        let relative = relative.to_string_lossy().replace('\\', "/");
        if !sidecar.is_file() || !attribution.contains(&relative) {
            missing.push(relative);
        }
    }
    if missing.is_empty() {
        println!("Asset attribution check passed.");
        Ok(())
    } else {
        for path in missing {
            eprintln!("asset is missing an attribution entry or .license.md sidecar: {path}");
        }
        Err(TaskError::AssetAttributionMissing)
    }
}

fn collect_assets(directory: &Path, output: &mut Vec<PathBuf>) -> TaskResult<()> {
    let mut inspected = 0usize;
    let entries = fs::read_dir(directory).map_err(|_| TaskError::AssetDirectoryUnreadable)?;
    for entry in entries {
        inspected = inspected.checked_add(1).ok_or(TaskError::AssetLimit)?;
        if inspected > 10_000 || output.len() > 10_000 {
            return Err(TaskError::AssetLimit);
        }
        let entry = entry.map_err(|_| TaskError::AssetDirectoryUnreadable)?;
        let file_type = entry
            .file_type()
            .map_err(|_| TaskError::AssetDirectoryUnreadable)?;
        let path = entry.path();
        if file_type.is_symlink() {
            return Err(TaskError::AssetSymlink);
        }
        if file_type.is_dir() {
            collect_assets(&path, output)?;
        } else if file_type.is_file() {
            output.push(path);
        }
    }
    Ok(())
}

fn check_licenses() -> TaskResult<()> {
    let workspace = workspace_root()?;
    let status = Command::new("cargo")
        .current_dir(workspace)
        .args(["deny", "check", "licenses", "bans", "sources"])
        .status()
        .map_err(|_| TaskError::CommandUnavailable)?;
    if status.success() {
        Ok(())
    } else {
        Err(TaskError::LicenseAuditFailed)
    }
}

/// Builds the release binaries and packs them, with the licences and a desktop entry, into
/// `target/dist/clipcraft-<version>-<os>-<arch>.tar.gz` plus a `SHA256SUMS` file.
///
/// The package is not signed: signing needs a certificate that only the maintainers hold.
fn package() -> TaskResult<()> {
    run_cargo(&[
        "build",
        "--release",
        "-p",
        "clipcraft",
        "-p",
        "clipcraft-cli",
    ])?;
    let workspace = workspace_root()?;
    let name = format!(
        "clipcraft-{}-{}-{}",
        env!("CARGO_PKG_VERSION"),
        env::consts::OS,
        env::consts::ARCH
    );
    let dist = workspace.join("target").join("dist");
    let folder = dist.join(&name);
    let _ = fs::remove_dir_all(&folder);
    let fail = |_| TaskError::PackageFailed;
    let windows = cfg!(windows);
    fs::create_dir_all(folder.join("bin")).map_err(fail)?;
    let release = workspace.join("target").join("release");
    // The command line tool is a separate download: most people only want the app.
    let app = format!("clipcraft{}", env::consts::EXE_SUFFIX);
    fs::copy(release.join(&app), folder.join("bin").join(&app)).map_err(fail)?;
    if !windows {
        fs::create_dir_all(folder.join("share").join("applications")).map_err(fail)?;
        fs::write(
            folder.join("share").join("applications").join("clipcraft.desktop"),
            "[Desktop Entry]\nType=Application\nName=ClipCraft\nComment=Local clip recorder and editor\nExec=clipcraft\nTerminal=false\nCategories=AudioVideo;Video;Recorder;\n",
        )
        .map_err(fail)?;
    }
    for document in ["README.md", "ATTRIBUTION.md", "LICENSE-MIT"] {
        fs::copy(workspace.join(document), folder.join(document)).map_err(fail)?;
    }
    // The embedded RNNoise model's BSD notice must accompany binary distributions (ADR 0028).
    let notices = folder.join("docs").join("licenses");
    fs::create_dir_all(&notices).map_err(fail)?;
    fs::copy(
        workspace.join("docs/licenses/nnnoiseless-BSD-3-Clause.txt"),
        notices.join("nnnoiseless-BSD-3-Clause.txt"),
    )
    .map_err(fail)?;
    let install = if windows {
        "Run the ClipCraft setup program to install it. Everything ClipCraft saves stays on this computer.\n\
This download is not signed, so Windows SmartScreen may say the publisher is unknown: choose\n\
More info, then Run anyway. Check the file against SHA256SUMS.txt from the release page first.\n"
    } else {
        "ClipCraft runs from any folder: start bin/clipcraft.\n\
To install for your user, copy bin/clipcraft to ~/.local/bin and share/applications/clipcraft.desktop to\n\
~/.local/share/applications. Everything ClipCraft saves stays on this computer.\n\
This package is not signed. Check it against SHA256SUMS before running it.\n"
    };
    fs::write(folder.join("INSTALL.txt"), install).map_err(fail)?;
    // Every dependency's licence, as the audit sees it.
    let licenses = Command::new("cargo")
        .current_dir(&workspace)
        .args(["deny", "list"])
        .output()
        .map_err(|_| TaskError::CommandUnavailable)?;
    fs::write(folder.join("THIRD-PARTY-LICENSES.txt"), licenses.stdout).map_err(fail)?;
    if windows {
        run_cargo(&["build", "--release", "-p", "clipcraft-installer"])?;
        let setup = dist.join(format!("ClipCraft-Setup-{}.exe", env!("CARGO_PKG_VERSION")));
        if setup.exists() {
            fs::remove_file(&setup).map_err(fail)?;
        }
        clipcraft_engine::installation::bundle(
            &release.join("clipcraft-installer.exe"),
            &folder,
            &setup,
        )
        .map_err(|_| TaskError::PackageFailed)?;
        println!("built offline Windows setup executable");
    }
    // The command line tool, on its own.
    let cli_name = format!(
        "clipcraft-cli-{}-{}-{}",
        env!("CARGO_PKG_VERSION"),
        env::consts::OS,
        env::consts::ARCH
    );
    let cli_folder = dist.join(&cli_name);
    let _ = fs::remove_dir_all(&cli_folder);
    fs::create_dir_all(cli_folder.join("docs").join("licenses")).map_err(fail)?;
    let cli = format!("clipcraft-cli{}", env::consts::EXE_SUFFIX);
    fs::copy(release.join(&cli), cli_folder.join(&cli)).map_err(fail)?;
    for document in ["ATTRIBUTION.md", "LICENSE-MIT", "THIRD-PARTY-LICENSES.txt"] {
        let from = if document == "THIRD-PARTY-LICENSES.txt" {
            folder.join(document)
        } else {
            workspace.join(document)
        };
        fs::copy(from, cli_folder.join(document)).map_err(fail)?;
    }
    fs::copy(
        workspace.join("docs/licenses/nnnoiseless-BSD-3-Clause.txt"),
        cli_folder.join("docs/licenses/nnnoiseless-BSD-3-Clause.txt"),
    )
    .map_err(fail)?;
    fs::write(
        cli_folder.join("README.txt"),
        "clipcraft-cli is the command line version of ClipCraft: list capture targets, record, trim, export\n\
and inspect your library from a terminal. Run it with `help` for the commands, and `diagnose` if\n\
recording does not work. It is separate from the app, which has its own download.\n",
    )
    .map_err(fail)?;
    // Windows 10 and later ship a `tar` that writes zip files.
    let pack = |folder_name: &str| -> TaskResult<String> {
        let archive = if windows {
            format!("{folder_name}.zip")
        } else {
            format!("{folder_name}.tar.gz")
        };
        let flags: &[&str] = if windows { &["-a", "-cf"] } else { &["-czf"] };
        let status = Command::new("tar")
            .current_dir(&dist)
            .args(flags)
            .args([&archive, folder_name])
            .status()
            .map_err(|_| TaskError::CommandUnavailable)?;
        if !status.success() {
            return Err(TaskError::PackageFailed);
        }
        Ok(archive)
    };
    // On Windows the app is downloaded as the setup program; Linux gets a tarball.
    let mut archives = vec![pack(&cli_name)?];
    if !windows {
        archives.push(pack(&name)?);
    }
    // On Windows the release workflow writes the checksums for every file together.
    if !windows {
        let sums = Command::new("sha256sum")
            .current_dir(&dist)
            .args(&archives)
            .output()
            .map_err(|_| TaskError::CommandUnavailable)?;
        fs::write(dist.join("SHA256SUMS"), sums.stdout).map_err(fail)?;
    }
    for archive in archives {
        println!("packed {}", dist.join(archive).display());
    }
    Ok(())
}

fn workspace_root() -> TaskResult<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or(TaskError::WorkspacePath)
}

#[derive(Clone, Copy, Debug)]
enum TaskError {
    Usage,
    DocsCheckUsage,
    CommandUnavailable,
    CommandFailed,
    GitDiffFailed,
    DocumentationMissing,
    MetadataUnavailable,
    MetadataInvalid,
    MissingLayer,
    LayerViolation,
    AttributionMissing,
    AssetDirectoryUnreadable,
    AssetLimit,
    AssetSymlink,
    AssetPath,
    AssetAttributionMissing,
    LicenseAuditFailed,
    WorkspacePath,
    PackageFailed,
}

impl fmt::Display for TaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::PackageFailed => "the release package could not be built",
            Self::Usage => {
                "usage: cargo xtask <ci|docs-check <base-ref>|layers|assets|licenses|package>"
            }
            Self::DocsCheckUsage => "usage: cargo xtask docs-check <base-ref>",
            Self::CommandUnavailable => "required Rust command could not be started",
            Self::CommandFailed => "a quality-gate command failed",
            Self::GitDiffFailed => "git could not compare the pull request with its base ref",
            Self::DocumentationMissing => {
                "implementation changes are missing required documentation updates"
            }
            Self::MetadataUnavailable => "cargo metadata failed",
            Self::MetadataInvalid => "cargo metadata was incomplete or invalid",
            Self::MissingLayer => {
                "every workspace crate must declare package.metadata.clipcraft.layer"
            }
            Self::LayerViolation => "workspace crate dependency layering is invalid",
            Self::AttributionMissing => "ATTRIBUTION.md is missing or unreadable",
            Self::AssetDirectoryUnreadable => "the assets directory could not be inspected safely",
            Self::AssetLimit => "the asset tree exceeds the inspection limit",
            Self::AssetSymlink => "asset symlinks are not accepted; store licensed files directly",
            Self::AssetPath => "an asset path escaped the workspace",
            Self::AssetAttributionMissing => {
                "one or more assets lack attribution or a license sidecar"
            }
            Self::LicenseAuditFailed => {
                "cargo-deny licenses, bans, or sources audit failed; see the output above"
            }
            Self::WorkspacePath => "could not locate the workspace root",
        };
        formatter.write_str(message)
    }
}

impl Error for TaskError {}

#[cfg(test)]
mod tests {
    use super::{find_layer_violations, missing_documentation};
    use serde_json::json;

    #[test]
    fn layer_checker_accepts_dependencies_that_flow_downward() {
        let packages = vec![
            json!({
                "name": "clipcraft-time",
                "metadata": {"clipcraft": {"layer": 0}},
                "dependencies": []
            }),
            json!({
                "name": "clipcraft-engine",
                "metadata": {"clipcraft": {"layer": 4}},
                "dependencies": [{"name": "clipcraft-time", "kind": null}]
            }),
        ];

        assert!(
            find_layer_violations(&packages)
                .expect("fixture metadata is valid")
                .is_empty()
        );
    }

    #[test]
    fn layer_checker_rejects_a_higher_layer_dependency_fixture() {
        let packages = vec![
            json!({
                "name": "clipcraft-time",
                "metadata": {"clipcraft": {"layer": 0}},
                "dependencies": [{"name": "clipcraft-engine", "kind": null}]
            }),
            json!({
                "name": "clipcraft-engine",
                "metadata": {"clipcraft": {"layer": 4}},
                "dependencies": []
            }),
        ];

        let violations = find_layer_violations(&packages).expect("fixture metadata is valid");
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("clipcraft-time (layer 0)"));
        assert!(violations[0].contains("higher layer clipcraft-engine (layer 4)"));
    }

    #[test]
    fn documentation_checker_requires_readme_and_gaps_for_implementation_changes() {
        let changed = vec!["crates/clipcraft-engine/src/lib.rs".to_owned()];

        assert_eq!(
            missing_documentation(&changed),
            vec!["README.md", "docs/gaps.md"]
        );
    }

    #[test]
    fn documentation_checker_accepts_complete_or_docs_only_changes() {
        let implementation_and_docs = vec![
            "apps/clipcraft/src/main.rs".to_owned(),
            "README.md".to_owned(),
            "docs/gaps.md".to_owned(),
        ];
        let docs_only = vec!["docs/architecture.md".to_owned()];

        assert!(missing_documentation(&implementation_and_docs).is_empty());
        assert!(missing_documentation(&docs_only).is_empty());
    }
}

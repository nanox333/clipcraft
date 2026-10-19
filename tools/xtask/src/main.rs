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
        "layers" => check_layers(),
        "assets" => check_assets(),
        "licenses" => check_licenses(),
        "help" | "--help" | "-h" => {
            println!("ClipCraft Rust tasks: ci, layers, assets, licenses");
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
        if !sidecar.is_file() || !attribution.contains(&relative.to_string_lossy().to_string()) {
            missing.push(relative.to_string_lossy().into_owned());
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
    CommandUnavailable,
    CommandFailed,
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
}

impl fmt::Display for TaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Usage => "usage: cargo xtask <ci|layers|assets|licenses>",
            Self::CommandUnavailable => "required Rust command could not be started",
            Self::CommandFailed => "a quality-gate command failed",
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
    use super::find_layer_violations;
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
}

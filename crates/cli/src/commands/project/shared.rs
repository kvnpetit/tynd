use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::ExitStatus,
};

use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;

use crate::{
    config::{self, LoadedConfig, TyndConfig},
    process::{self, CommandSpec},
};

#[derive(Clone, Debug)]
pub(super) struct BunRuntime {
    pub(super) path: PathBuf,
    pub(super) version: String,
}

pub(super) fn resolve_bun_runtime(root: &Path) -> Result<BunRuntime> {
    let launcher = process::find_executable("bun").context("Bun executable not found")?;
    let output = process::run_spec(
        CommandSpec::new(launcher.as_os_str())
            .args([
                OsString::from("--eval"),
                OsString::from(
                    "process.stdout.write(JSON.stringify({execPath:process.execPath,version:Bun.version}))",
                ),
            ])
            .cwd(root),
    )?;
    let details: Value = serde_json::from_str(output.stdout_trimmed())
        .context("Bun did not report its runtime identity")?;
    let reported_path = details
        .get("execPath")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .context("Bun did not report process.execPath")?;
    let path = fs::canonicalize(reported_path)
        .with_context(|| format!("resolve Bun runtime {reported_path}"))?;
    ensure!(
        path.is_file(),
        "Bun runtime is not a file: {}",
        path.display()
    );
    let version = details
        .get("version")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .context("Bun did not report Bun.version")?
        .trim_start_matches('v')
        .to_owned();
    semver::Version::parse(&version).with_context(|| format!("invalid Bun version {version:?}"))?;

    let pin = root.join(".bun-version");
    if pin.is_file() {
        let source = fs::read_to_string(&pin)
            .with_context(|| format!("read Bun version pin {}", pin.display()))?;
        let expected = source
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .trim_start_matches('v');
        ensure!(!expected.is_empty(), "{} is empty", pin.display());
        semver::Version::parse(expected)
            .with_context(|| format!("invalid Bun version pin {expected:?}"))?;
        ensure!(
            expected == version,
            "Bun {version} does not match {} ({expected}); install the pinned runtime or update the pin intentionally",
            pin.display()
        );
    }
    Ok(BunRuntime { path, version })
}

pub(super) fn with_project_environment(
    mut spec: CommandSpec,
    loaded: &LoadedConfig,
) -> CommandSpec {
    for (key, value) in &loaded.environment {
        if !process::is_sensitive_env_key(key) {
            spec = spec.env(key, value);
        }
    }
    spec
}

pub(super) fn ensure_success(status: ExitStatus, label: &str) -> Result<()> {
    ensure!(
        status.success(),
        "{label} exited with code {}",
        status.code().unwrap_or(-1)
    );
    Ok(())
}

pub(super) fn app_id_from_config(config: &TyndConfig) -> Option<String> {
    config
        .bundle
        .as_ref()
        .map(|bundle| bundle.identifier.clone())
        .filter(|value| !value.is_empty())
}

pub(super) fn valid_project_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .next()
            .is_some_and(|value| value.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '.' | '_' | '-'))
        && name != "."
        && name != ".."
}

pub(super) fn project_name(root: &Path) -> String {
    root.file_name()
        .and_then(OsStr::to_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("tynd-app")
        .to_owned()
}

/// Host API named by `input`: a known API, or a `tynd add` id/alias (from
/// plugins/catalog.json) of a host-backed plugin that fronts one.
pub(super) fn normalize_capability(input: &str) -> Option<&'static str> {
    let normalized = input.trim().to_ascii_lowercase();
    config::KNOWN_APIS
        .iter()
        .copied()
        .find(|value| *value == normalized)
        .or_else(|| catalog_add_name(&normalized).and_then(|(_, api)| api))
}

/// `(plugin package, host API)` of a `tynd add` id or alias from plugins/catalog.json.
pub(super) fn catalog_add_name(input: &str) -> Option<(&'static str, Option<&'static str>)> {
    let normalized = input.trim().to_ascii_lowercase();
    config::PLUGIN_ADD_NAMES
        .iter()
        .find(|(name, _, _)| *name == normalized)
        .map(|(_, package, api)| (*package, *api))
}

pub(super) fn directory_size(path: &Path) -> Result<u64> {
    let mut bytes = 0_u64;
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            bail!("refusing symlink while sizing {}", entry.path().display());
        }
        if entry.file_type().is_file() {
            bytes = bytes.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(bytes)
}

pub(super) fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub(super) fn generated_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let root = fs::canonicalize(root)
        .with_context(|| format!("resolve project root {}", root.display()))?;
    let relative = Path::new(relative);
    ensure!(
        !relative.as_os_str().is_empty()
            && !relative.is_absolute()
            && relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "generated path must be a safe project-relative path"
    );
    let normalized = relative.to_string_lossy().replace('\\', "/");
    ensure!(
        normalized == ".tynd/cache"
            || normalized.starts_with(".tynd/cache/")
            || normalized == ".tynd/build"
            || normalized.starts_with(".tynd/build/")
            || normalized == "release"
            || normalized.starts_with("release/"),
        "refusing unknown generated path {normalized}"
    );
    let target = root.join(relative);
    ensure_existing_path_is_unlinked(&root, &target)?;
    Ok(target)
}

pub(super) fn ensure_existing_path_is_unlinked(root: &Path, target: &Path) -> Result<()> {
    ensure!(
        target.starts_with(root) && target != root,
        "refusing unsafe generated target {}",
        target.display()
    );
    let mut current = root.to_path_buf();
    for component in target.strip_prefix(root)?.components() {
        ensure!(
            matches!(component, std::path::Component::Normal(_)),
            "generated target contains an unsafe component"
        );
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) => ensure!(
                !is_link_or_reparse(&metadata),
                "refusing linked generated path {}",
                current.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(error).with_context(|| format!("inspect {}", current.display()))
            },
        }
    }
    Ok(())
}

pub(super) fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub(super) fn replace_generated_directory(root: &Path, target: &Path) -> Result<()> {
    let root = fs::canonicalize(root)?;
    ensure_existing_path_is_unlinked(&root, target)?;
    if target.exists() {
        ensure!(
            target.is_dir(),
            "generated output is not a directory: {}",
            target.display()
        );
        fs::remove_dir_all(target).with_context(|| format!("remove {}", target.display()))?;
    }
    fs::create_dir_all(target).with_context(|| format!("create {}", target.display()))
}

pub(super) fn copy_tree_strict(source: &Path, destination: &Path) -> Result<()> {
    ensure!(
        source.is_dir(),
        "source directory not found: {}",
        source.display()
    );
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(
            !is_link_or_reparse(&metadata),
            "refusing linked source path {}",
            entry.path().display()
        );
        let relative = entry.path().strip_prefix(source)?;
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target).with_context(|| {
                format!("copy {} to {}", entry.path().display(), target.display())
            })?;
        }
    }
    Ok(())
}

pub(super) fn ensure_generated_target(root: &Path, target: &Path) -> Result<()> {
    let root = fs::canonicalize(root)?;
    let relative = target
        .strip_prefix(&root)
        .with_context(|| format!("{} is outside project root", target.display()))?;
    let expected = generated_path(&root, &relative.to_string_lossy())?;
    ensure!(
        expected == target,
        "generated target did not normalize safely"
    );
    Ok(())
}

pub(in crate::commands) fn app_id(cwd: &Path) -> Result<Option<String>> {
    Ok(app_id_from_config(&config::load_typed(cwd, None)?.config))
}

/// App id and the configured default updater channel, from a single config load
/// (loading a script config spawns Bun).
pub(in crate::commands) fn publish_identity(
    cwd: &Path,
) -> Result<(Option<String>, Option<String>)> {
    let config = config::load_typed(cwd, None)?.config;
    let channel = config
        .updater
        .as_ref()
        .and_then(|updater| updater.channel.clone());
    Ok((app_id_from_config(&config), channel))
}
pub(super) fn ensure_line_block(path: &Path, lines: &[&str]) -> Result<()> {
    let mut source = fs::read_to_string(path).unwrap_or_default();
    if !source.is_empty() && !source.ends_with('\n') {
        source.push('\n');
    }
    let mut changed = false;
    for line in lines {
        if !source.lines().any(|existing| existing.trim() == *line) {
            source.push_str(line);
            source.push('\n');
            changed = true;
        }
    }
    if changed || !path.exists() {
        crate::fsx::atomic_write(path, source)?;
    }
    Ok(())
}

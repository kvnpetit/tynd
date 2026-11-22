//! `tynd updater publish`: signs the release artifacts into a manifest and
//! uploads them.

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::{json, Map, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use super::{epoch_seconds, read_private_key};
use crate::args::{PatchStrategy, PublishArgs, PublishTarget};
use crate::commands::project;
use crate::config::find_project_root;
use crate::fsx::atomic_write;
use crate::output::{display_path, success, warn};

pub(in crate::commands) fn publish(args: PublishArgs) -> Result<PathBuf> {
    let cwd = find_project_root(&args.cwd)?;
    let (app_id, config_channel) = project::publish_identity(&cwd)?;
    let app_id = app_id.context("config has no bundle.identifier")?;
    let key = args.key.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| cwd.clone())
            .join(".tynd")
            .join("keys")
            .join(format!("{app_id}.key"))
    });
    publish_with_options(
        &PublishRequest {
            cwd,
            version: args.version,
            target: args.target,
            key,
            app_id,
            public_base_url: args.public_base_url,
            bucket: args.bucket,
            region: args.region,
            endpoint: args.endpoint,
            patch_strategy: args.patch_strategy,
            max_diffs: args.max_diffs,
            max_patches: args.max_patches,
            notes_file: args.notes_file,
            skip_diffs: args.no_diffs,
        },
        &PublishOptions {
            channel: args.channel,
            config_channel,
            rollout: args.rollout,
        },
    )
}

#[derive(Debug, Clone)]
struct PublishRequest {
    cwd: PathBuf,
    version: String,
    target: PublishTarget,
    key: PathBuf,
    app_id: String,
    public_base_url: Option<String>,
    bucket: Option<String>,
    region: Option<String>,
    endpoint: Option<String>,
    patch_strategy: PatchStrategy,
    max_diffs: usize,
    max_patches: usize,
    notes_file: Option<PathBuf>,
    skip_diffs: bool,
}

/// Optional distribution metadata written into the signed manifest. Kept
/// apart from [`PublishRequest`] so existing callers keep compiling.
#[derive(Debug, Clone, Default)]
struct PublishOptions {
    /// Release channel (`stable`, `beta`, ...). Written as the manifest's
    /// `channel`; clients that follow a channel reject manifests of another one.
    channel: Option<String>,
    /// `updater.channel` of the project config: the channel when neither the
    /// option nor `TYND_PUBLISH_CHANNEL` sets one (`stable` after that).
    config_channel: Option<String>,
    /// Staged-rollout percentage 0..=100. Written as the manifest's `rollout`;
    /// the client keeps the update from installs outside that share.
    rollout: Option<u8>,
}

/// Channel for a publish: the explicit option, else `TYND_PUBLISH_CHANNEL`,
/// else the configured `updater.channel`, else `stable`. A manifest always
/// carries its channel, so a client following another channel rejects it.
fn resolve_publish_options(options: &PublishOptions) -> PublishOptions {
    resolve_publish_options_with(
        options,
        std::env::var("TYND_PUBLISH_CHANNEL")
            .ok()
            .filter(|value| !value.trim().is_empty()),
    )
}

fn resolve_publish_options_with(
    options: &PublishOptions,
    env_channel: Option<String>,
) -> PublishOptions {
    let channel = options
        .channel
        .clone()
        .or(env_channel)
        .or_else(|| options.config_channel.clone())
        .unwrap_or_else(|| crate::config::DEFAULT_UPDATER_CHANNEL.to_owned());
    PublishOptions {
        channel: Some(channel),
        config_channel: options.config_channel.clone(),
        rollout: options.rollout,
    }
}

fn validate_channel_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        bail!("invalid channel '{name}': must be 1-64 characters");
    }
    if name.starts_with('.')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        bail!("invalid channel '{name}': use letters, digits, '.', '_' or '-' (not starting with '.')");
    }
    Ok(())
}

/// Add `channel` / `rollout` to a manifest after validating them.
fn apply_distribution(manifest: &mut Value, options: &PublishOptions) -> Result<()> {
    if let Some(channel) = &options.channel {
        validate_channel_name(channel)?;
        manifest["channel"] = Value::String(channel.clone());
    }
    if let Some(rollout) = options.rollout {
        if rollout > 100 {
            bail!("invalid rollout {rollout}: must be between 0 and 100");
        }
        manifest["rollout"] = Value::from(rollout);
    }
    Ok(())
}

fn publish_with_options(request: &PublishRequest, options: &PublishOptions) -> Result<PathBuf> {
    let options = resolve_publish_options(options);
    semver::Version::parse(&request.version)
        .with_context(|| format!("invalid release version {}", request.version))?;
    let signing = read_private_key(&request.key)?;
    let release = request.cwd.join("release");
    if !release.is_dir() {
        bail!("release directory not found; run `tynd build` first");
    }
    let platforms = signed_platform_entries(request, &signing, &release)?;
    if platforms.is_empty() {
        bail!("no platform artifacts found under {}", release.display());
    }
    let notes = request
        .notes_file
        .as_ref()
        .map(fs::read_to_string)
        .transpose()?;
    let mut manifest = json!({
        "version": request.version,
        "pub_date": rfc3339_now(),
        "platforms": platforms,
    });
    if let Some(notes) = notes {
        manifest["notes"] = Value::String(notes);
    }
    apply_distribution(&mut manifest, &options)?;
    let publish_dir = release.join("publish");
    fs::create_dir_all(&publish_dir)?;
    let manifest_path = publish_dir.join("manifest.json");
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    atomic_write(&manifest_path, &manifest_bytes)?;
    let manifest_signature = BASE64.encode(signing.sign(&manifest_bytes).to_bytes());
    atomic_write(
        &publish_dir.join("manifest.json.sig"),
        format!("{manifest_signature}\n"),
    )?;
    if !request.skip_diffs {
        generate_available_patches(request, &signing, &release, &mut manifest)?;
    }
    match request.target {
        PublishTarget::ManifestOnly => {},
        PublishTarget::GithubReleases => upload_github(request, &release)?,
        PublishTarget::S3 | PublishTarget::R2 => upload_s3(request, &release)?,
    }
    archive_release(request, &release)?;
    success(format!("manifest -> {}", display_path(&manifest_path)));
    Ok(manifest_path)
}

/// One signed manifest entry per `release/<platform>` directory, keyed by
/// updater platform key; unknown directories are skipped.
fn signed_platform_entries(
    request: &PublishRequest,
    signing: &SigningKey,
    release: &Path,
) -> Result<Map<String, Value>> {
    let mut platforms = Map::new();
    for entry in fs::read_dir(release)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let Some(platform_key) = platform_key(&dir_name) else {
            continue;
        };
        let artifact = updater_release_artifact(&entry.path(), &dir_name)?;
        let bytes = fs::read(&artifact)?;
        let signature = BASE64.encode(signing.sign(&bytes).to_bytes());
        let file_name = artifact
            .file_name()
            .and_then(|name| name.to_str())
            .context("release artifact has a non-UTF-8 filename")?;
        let url = artifact_url(request, &dir_name, file_name);
        let format = if dir_name.starts_with("darwin-") || dir_name.starts_with("macos-") {
            "app-archive"
        } else {
            "binary"
        };
        platforms.insert(
            platform_key.into(),
            json!({
                "url": url,
                "signature": signature,
                "size": bytes.len(),
                "format": format,
                "blake3": blake3::hash(&bytes).to_hex().to_string(),
            }),
        );
    }
    Ok(platforms)
}

struct PatchGeneration<'a> {
    request: &'a PublishRequest,
    signing: &'a SigningKey,
    bundler: &'a Path,
    archive: &'a Path,
    output: &'a Path,
    previous_versions: &'a [String],
}

impl PatchGeneration<'_> {
    fn for_platform(&self, platform: &str, new_artifact: &Path) -> Result<Vec<Value>> {
        let mut patches = Vec::new();
        for previous in self.previous_versions {
            let old_dir = self.archive.join(previous).join(platform);
            if !old_dir.is_dir() {
                continue;
            }
            let old_artifact = match updater_release_artifact(&old_dir, platform) {
                Ok(artifact) => artifact,
                Err(error) => {
                    warn(format!(
                        "cannot use archived {platform} release {previous} for a patch: {error}"
                    ));
                    continue;
                },
            };
            let output = self
                .output
                .join(format!("{previous}-{platform}.bsdiff.zst"));
            let status = Command::new(self.bundler)
                .args(["patch", "--old"])
                .arg(&old_artifact)
                .arg("--new")
                .arg(new_artifact)
                .arg("--output")
                .arg(&output)
                .status()?;
            if !status.success() {
                warn(format!(
                    "patch failed for {platform} from {previous}; full binary remains available"
                ));
                continue;
            }
            let bytes = fs::read(&output)?;
            let file_name = output
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("patch.bsdiff.zst");
            patches.push(json!({
                "from": previous,
                "url": artifact_url(self.request, "patches", file_name),
                "signature": BASE64.encode(self.signing.sign(&bytes).to_bytes()),
                "size": bytes.len(),
                "blake3": blake3::hash(&bytes).to_hex().to_string(),
                "compression": "zstd",
            }));
        }
        Ok(patches)
    }
}

fn generate_available_patches(
    request: &PublishRequest,
    signing: &SigningKey,
    release: &Path,
    manifest: &mut Value,
) -> Result<()> {
    let patches_dir = release.join("patches");
    if patches_dir.exists() {
        fs::remove_dir_all(&patches_dir)?;
    }
    let archive = releases_root(&request.cwd).join(&request.app_id);
    if !archive.is_dir() {
        return Ok(());
    }
    let previous_versions = selected_previous_versions(
        &archive,
        &request.version,
        request.patch_strategy,
        request.max_diffs,
        request.max_patches,
    )?;
    if previous_versions.is_empty() {
        return Ok(());
    }
    let bundler = crate::project::resolve_binary(crate::project::TyndBinary::Bundler)?;
    fs::create_dir_all(&patches_dir)?;
    let generation = PatchGeneration {
        request,
        signing,
        bundler: &bundler,
        archive: &archive,
        output: &patches_dir,
        previous_versions: &previous_versions,
    };
    for entry in fs::read_dir(release)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let platform = entry.file_name().to_string_lossy().to_string();
        if platform == "patches"
            || platform == "publish"
            || platform.starts_with("darwin-")
            || platform.starts_with("macos-")
        {
            continue;
        }
        let Some(key) = platform_key(&platform) else {
            continue;
        };
        let new_artifact = updater_release_artifact(&entry.path(), &platform)?;
        let patches = generation.for_platform(&platform, &new_artifact)?;
        if !patches.is_empty() {
            if let Some(entry) = manifest["platforms"].get_mut(key) {
                entry["patches"] = Value::Array(patches);
            }
        }
    }
    write_signed_manifest(release, manifest, signing)
}

fn write_signed_manifest(release: &Path, manifest: &Value, signing: &SigningKey) -> Result<()> {
    let publish_dir = release.join("publish");
    let manifest_bytes = serde_json::to_vec_pretty(manifest)?;
    atomic_write(&publish_dir.join("manifest.json"), &manifest_bytes)?;
    atomic_write(
        &publish_dir.join("manifest.json.sig"),
        format!(
            "{}\n",
            BASE64.encode(signing.sign(&manifest_bytes).to_bytes())
        ),
    )
}

fn selected_previous_versions(
    root: &Path,
    current: &str,
    strategy: PatchStrategy,
    max_diffs: usize,
    max_patches: usize,
) -> Result<Vec<String>> {
    let current = semver::Version::parse(current)?;
    let mut versions = fs::read_dir(root)?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(ToOwned::to_owned))
        .filter_map(|raw| {
            semver::Version::parse(&raw)
                .ok()
                .map(|parsed| (parsed, raw))
        })
        .filter(|(parsed, _)| parsed < &current)
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| right.0.cmp(&left.0));
    match strategy {
        PatchStrategy::SameMajor => versions.retain(|(version, _)| version.major == current.major),
        PatchStrategy::LastN => versions.truncate(max_diffs),
        PatchStrategy::All => {},
    }
    if versions.len() > max_patches {
        let dropped = versions
            .drain(max_patches..)
            .map(|(_, raw)| raw)
            .collect::<Vec<_>>();
        warn(format!(
            "patch cap {max_patches} dropped archived versions: {}",
            dropped.join(", ")
        ));
    }
    Ok(versions.into_iter().map(|(_, raw)| raw).collect())
}

fn path_has_extension(path: &Path, expected: &[&str]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|actual| {
            expected
                .iter()
                .any(|candidate| actual.eq_ignore_ascii_case(candidate))
        })
}

fn updater_release_artifact(directory: &Path, platform: &str) -> Result<PathBuf> {
    if !directory.is_dir() {
        bail!(
            "release platform directory not found: {}",
            directory.display()
        );
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!(
                "release platform directory contains a symbolic link: {}",
                entry.path().display()
            );
        }
        if file_type.is_file() {
            files.push(entry.path());
        }
    }
    files.sort();

    if platform.starts_with("darwin-") || platform.starts_with("macos-") {
        let candidates = files
            .into_iter()
            .filter(|path| {
                let name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                path_has_extension(path, &["zip"])
                    && name.contains("-macos")
                    && !name.ends_with("-artifacts.zip")
            })
            .collect::<Vec<_>>();
        let archive = require_single_updater_artifact(directory, platform, candidates)?;
        validate_macos_app_archive(&archive)?;
        return Ok(archive);
    }

    let mut candidates = Vec::new();
    for path in files {
        if has_tyndpkg_trailer(&path)? {
            candidates.push(path);
        }
    }
    require_single_updater_artifact(directory, platform, candidates)
}

fn require_single_updater_artifact(
    directory: &Path,
    platform: &str,
    candidates: Vec<PathBuf>,
) -> Result<PathBuf> {
    match candidates.as_slice() {
        [artifact] => Ok(artifact.clone()),
        [] if platform.starts_with("darwin-") || platform.starts_with("macos-") => bail!(
            "{} has no macOS app update archive; build with `tynd build --bundle macos` before publishing",
            directory.display()
        ),
        [] => bail!(
            "{} has no packed TYNDPKG update binary; run `tynd build` before publishing",
            directory.display()
        ),
        _ => bail!(
            "{} contains multiple updater artifacts for {platform}: {}",
            directory.display(),
            candidates
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Whether `path` carries a readable TYNDPKG section. Delegates to the pack
/// reader so signed artifacts (the trailer sits before the Authenticode /
/// code-signature blob, not at EOF) and the magic stay in one place.
fn has_tyndpkg_trailer(path: &Path) -> Result<bool> {
    fs::metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    match tynd_pack_format::PackReader::open_path_checked(path) {
        Ok(found) => Ok(found.is_some()),
        Err(error) => bail!("{} has a damaged TYNDPKG section: {error}", path.display()),
    }
}

fn validate_macos_app_archive(path: &Path) -> Result<()> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("read macOS app archive {}", path.display()))?;
    let mut has_info_plist = false;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let name = entry.name()?.replace('\\', "/");
        if name.contains(".app/Contents/") && name.ends_with("Info.plist") {
            has_info_plist = true;
            break;
        }
    }
    if !has_info_plist {
        bail!(
            "{} is not a macOS .app archive (Contents/Info.plist is missing)",
            path.display()
        );
    }
    Ok(())
}

fn platform_key(directory: &str) -> Option<&'static str> {
    match directory {
        "win32-x64" | "windows-x64" => Some("windows-x86_64"),
        "win32-arm64" | "windows-arm64" => Some("windows-aarch64"),
        "linux-x64" => Some("linux-x86_64"),
        "linux-arm64" => Some("linux-aarch64"),
        "darwin-x64" | "macos-x64" => Some("darwin-x86_64"),
        "darwin-arm64" | "macos-arm64" => Some("darwin-aarch64"),
        _ => None,
    }
}

fn artifact_url(request: &PublishRequest, platform: &str, file: &str) -> String {
    if let Some(base) = &request.public_base_url {
        return format!(
            "{}/{}/{}/{}/{}",
            base.trim_end_matches('/'),
            request.app_id,
            request.version,
            platform,
            file
        );
    }
    format!(
        "{}/{}/{}/{}",
        request.app_id, request.version, platform, file
    )
}

fn upload_github(request: &PublishRequest, release: &Path) -> Result<()> {
    let tag = format!("v{}", request.version);
    let mut artifacts = Vec::new();
    for entry in walkdir::WalkDir::new(release).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file() {
            artifacts.push(entry.path().to_path_buf());
        }
    }
    let mut command = Command::new("gh");
    command.args(["release", "upload", &tag]);
    command.args(&artifacts).arg("--clobber");
    let status = command.status().context("start gh release upload")?;
    if !status.success() {
        bail!("gh release upload failed with {status}");
    }
    Ok(())
}

fn upload_s3(request: &PublishRequest, release: &Path) -> Result<()> {
    let bucket = request.bucket.as_deref().context("--bucket is required")?;
    let destination = format!(
        "s3://{}/{}/{}",
        bucket.trim_matches('/'),
        request.app_id,
        request.version
    );
    let mut command = Command::new("aws");
    command
        .args(["s3", "cp"])
        .arg(release)
        .arg(destination)
        .arg("--recursive");
    if let Some(region) = &request.region {
        command.args(["--region", region]);
    }
    if let Some(endpoint) = &request.endpoint {
        command.args(["--endpoint-url", endpoint]);
    }
    apply_s3_credentials(&mut command, |name| std::env::var(name).ok())?;
    let status = command.status().context("start aws s3 upload")?;
    if !status.success() {
        bail!("aws s3 upload failed with {status}");
    }
    Ok(())
}

/// Credentials come from the environment only, never from the command line
/// (visible in process lists and shell history). With the R2 pair selected, an
/// inherited `AWS_SESSION_TOKEN` belongs to other AWS credentials and would
/// make the request fail, so it is removed.
fn apply_s3_credentials(command: &mut Command, var: impl Fn(&str) -> Option<String>) -> Result<()> {
    if let Some(credentials) = s3_credentials(var)? {
        command
            .env("AWS_ACCESS_KEY_ID", credentials.access_key)
            .env("AWS_SECRET_ACCESS_KEY", credentials.secret_key);
        if credentials.access_name != "AWS_ACCESS_KEY_ID" {
            command.env_remove("AWS_SESSION_TOKEN");
        }
    }
    Ok(())
}

/// One resolved credential pair and the variable it was read from.
#[derive(Debug, PartialEq, Eq)]
struct S3Credentials {
    access_name: &'static str,
    access_key: String,
    secret_key: String,
}

/// Environment credential pairs, tried in order; with none set, the AWS CLI
/// falls back to its configured profile.
const S3_CREDENTIAL_PAIRS: [(&str, &str); 2] = [
    ("R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY"),
    ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"),
];

/// Resolves S3 credentials as whole pairs: the R2 pair, else the AWS pair,
/// else `None` (AWS CLI profile). A half-set pair is an error naming the
/// missing variable, never a value. Empty values count as unset.
fn s3_credentials(var: impl Fn(&str) -> Option<String>) -> Result<Option<S3Credentials>> {
    let var = |name: &str| var(name).filter(|value| !value.is_empty());
    for (access_name, secret_name) in S3_CREDENTIAL_PAIRS {
        match (var(access_name), var(secret_name)) {
            (Some(access_key), Some(secret_key)) => {
                return Ok(Some(S3Credentials {
                    access_name,
                    access_key,
                    secret_key,
                }));
            },
            (None, None) => {},
            (Some(_), None) => {
                bail!("{access_name} is set but {secret_name} is missing; set both or neither")
            },
            (None, Some(_)) => {
                bail!("{secret_name} is set but {access_name} is missing; set both or neither")
            },
        }
    }
    Ok(None)
}

fn releases_root(cwd: &Path) -> PathBuf {
    std::env::var_os("TYND_RELEASES_ROOT").map_or_else(
        || {
            dirs::home_dir()
                .unwrap_or_else(|| cwd.to_path_buf())
                .join(".tynd")
                .join("releases")
        },
        PathBuf::from,
    )
}

fn archive_release(request: &PublishRequest, release: &Path) -> Result<()> {
    let root = releases_root(&request.cwd)
        .join(&request.app_id)
        .join(&request.version);
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    crate::fsx::copy_tree(release, &root)
}

fn rfc3339_now() -> String {
    // UTC timestamp without adding a date/time dependency to the distributed CLI.
    // Consumers accept RFC3339; PowerShell/Unix date are not used, keeping this deterministic.
    const SECONDS_PER_DAY: i64 = 86_400;
    let seconds = i64::try_from(epoch_seconds()).unwrap_or(i64::MAX);
    let days = seconds.div_euclid(SECONDS_PER_DAY);
    let day_seconds = seconds.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_credentials_resolve_as_whole_pairs() {
        let lookup = |vars: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        assert_eq!(s3_credentials(lookup(&[])).expect("profile"), None);
        assert_eq!(
            s3_credentials(lookup(&[
                ("R2_ACCESS_KEY_ID", "r2-id"),
                ("R2_SECRET_ACCESS_KEY", "r2-secret"),
                ("AWS_ACCESS_KEY_ID", "aws-id"),
                ("AWS_SECRET_ACCESS_KEY", "aws-secret"),
            ]))
            .expect("r2 pair"),
            Some(S3Credentials {
                access_name: "R2_ACCESS_KEY_ID",
                access_key: "r2-id".to_owned(),
                secret_key: "r2-secret".to_owned(),
            })
        );
        assert_eq!(
            s3_credentials(lookup(&[
                ("AWS_ACCESS_KEY_ID", "aws-id"),
                ("AWS_SECRET_ACCESS_KEY", "aws-secret"),
            ]))
            .expect("aws pair"),
            Some(S3Credentials {
                access_name: "AWS_ACCESS_KEY_ID",
                access_key: "aws-id".to_owned(),
                secret_key: "aws-secret".to_owned(),
            })
        );
        // A half R2 pair never mixes with the AWS secret.
        let error = s3_credentials(lookup(&[
            ("R2_ACCESS_KEY_ID", "r2-id"),
            ("AWS_ACCESS_KEY_ID", "aws-id"),
            ("AWS_SECRET_ACCESS_KEY", "aws-secret"),
        ]))
        .expect_err("half pair")
        .to_string();
        assert!(error.contains("R2_SECRET_ACCESS_KEY"), "{error}");
        assert!(
            !error.contains("r2-id") && !error.contains("aws-secret"),
            "{error}"
        );
        let error = s3_credentials(lookup(&[("AWS_SECRET_ACCESS_KEY", "aws-secret")]))
            .expect_err("half pair")
            .to_string();
        assert!(error.contains("AWS_ACCESS_KEY_ID"), "{error}");
        assert!(!error.contains("aws-secret"), "{error}");
    }

    #[test]
    fn r2_credentials_drop_the_inherited_session_token() {
        use std::ffi::OsStr;
        let envs = |vars: &'static [(&'static str, &'static str)]| {
            let mut command = Command::new("aws");
            apply_s3_credentials(&mut command, |name| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            })
            .expect("credentials");
            command
                .get_envs()
                .map(|(key, value)| (key.to_owned(), value.map(OsStr::to_owned)))
                .collect::<Vec<_>>()
        };
        let session = |envs: &[(std::ffi::OsString, Option<std::ffi::OsString>)]| {
            envs.iter()
                .find(|(key, _)| key == "AWS_SESSION_TOKEN")
                .map(|(_, value)| value.clone())
        };
        let r2 = envs(&[
            ("R2_ACCESS_KEY_ID", "r2-id"),
            ("R2_SECRET_ACCESS_KEY", "r2-secret"),
        ]);
        assert_eq!(session(&r2), Some(None), "removed for the R2 pair");
        let aws = envs(&[
            ("AWS_ACCESS_KEY_ID", "aws-id"),
            ("AWS_SECRET_ACCESS_KEY", "aws-secret"),
        ]);
        assert_eq!(session(&aws), None, "kept for the AWS pair");
        assert!(envs(&[]).is_empty(), "profile leaves the environment alone");
    }

    #[test]
    fn writes_channel_and_rollout_into_the_manifest() {
        let mut manifest = json!({ "version": "1.2.3", "platforms": {} });
        let options = PublishOptions {
            channel: Some("beta".into()),
            rollout: Some(25),
            ..PublishOptions::default()
        };
        apply_distribution(&mut manifest, &options).unwrap();
        assert_eq!(manifest["channel"], "beta");
        assert_eq!(manifest["rollout"], 25);
    }

    #[test]
    fn leaves_manifest_unchanged_without_options() {
        let mut manifest = json!({ "version": "1.2.3" });
        apply_distribution(&mut manifest, &PublishOptions::default()).unwrap();
        assert!(manifest.get("channel").is_none());
        assert!(manifest.get("rollout").is_none());
    }

    #[test]
    fn rejects_bad_channel_and_rollout() {
        let mut manifest = json!({});
        for channel in ["", "a/b", ".hidden", "sp ace"] {
            let options = PublishOptions {
                channel: Some(channel.into()),
                ..PublishOptions::default()
            };
            assert!(
                apply_distribution(&mut manifest, &options).is_err(),
                "{channel:?}"
            );
        }
        let options = PublishOptions {
            rollout: Some(101),
            ..PublishOptions::default()
        };
        assert!(apply_distribution(&mut manifest, &options).is_err());
    }

    #[test]
    fn publish_channel_precedence_is_option_env_config_then_stable() {
        let options = |channel: Option<&str>, config_channel: Option<&str>| PublishOptions {
            channel: channel.map(Into::into),
            config_channel: config_channel.map(Into::into),
            rollout: None,
        };
        let resolved = |options: &PublishOptions, env: Option<&str>| {
            resolve_publish_options_with(options, env.map(Into::into)).channel
        };
        assert_eq!(
            resolved(&options(Some("cli"), Some("cfg")), Some("env")).as_deref(),
            Some("cli")
        );
        assert_eq!(
            resolved(&options(None, Some("cfg")), Some("env")).as_deref(),
            Some("env")
        );
        assert_eq!(
            resolved(&options(None, Some("cfg")), None).as_deref(),
            Some("cfg")
        );
        assert_eq!(
            resolved(&options(None, None), None).as_deref(),
            Some("stable")
        );
    }

    #[test]
    fn default_publish_writes_the_stable_channel_into_the_manifest() {
        let resolved = resolve_publish_options_with(&PublishOptions::default(), None);
        let mut manifest = json!({ "version": "1.2.3", "platforms": {} });
        apply_distribution(&mut manifest, &resolved).unwrap();
        assert_eq!(manifest["channel"], "stable");
    }
}

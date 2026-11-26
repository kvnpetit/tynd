//! `tynd updater keygen`.

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::SigningKey;
use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{epoch_seconds, PKCS8_PREFIX};
use crate::args::KeygenArgs;
use crate::commands::project;
use crate::config::find_project_root;
use crate::fsx::{absolute, atomic_write};
use crate::output::{display_path, step, success, warn};

/// Default private key path of a generic updater keypair; the public key is
/// the same path with a `.pub` extension. Matches `tynd updater sign --key`.
const GENERIC_PRIVATE_KEY: &str = "tynd-updater.key";

/// `tynd updater keygen`: a generic keypair, or with `--app-id` the app-scoped
/// key whose id falls back to `bundle.identifier`.
pub(in crate::commands) fn keygen(args: KeygenArgs) -> Result<()> {
    let Some(app_id) = args.app_id else {
        return generic_keygen(&KeygenRequest {
            out: args
                .out
                .unwrap_or_else(|| PathBuf::from(GENERIC_PRIVATE_KEY)),
            force: args.force,
        })
        .map(|_| ());
    };
    let cwd = find_project_root(&args.cwd)?;
    let app_id = app_id
        .or_else(|| project::app_id(&cwd).ok().flatten())
        .context("--app-id has no value and config has no bundle.identifier")?;
    app_keygen(&UpdaterKeygenRequest {
        cwd,
        app_id,
        out: args.out,
        force: args.force,
        edit_config: !args.no_config_edit,
    })
    .map(|_| ())
}

#[derive(Debug)]
struct KeygenRequest {
    out: PathBuf,
    force: bool,
}

#[derive(Debug)]
struct UpdaterKeygenRequest {
    cwd: PathBuf,
    app_id: String,
    out: Option<PathBuf>,
    force: bool,
    edit_config: bool,
}

fn generic_keygen(request: &KeygenRequest) -> Result<(PathBuf, PathBuf)> {
    let private_path = request.out.clone();
    let public_path = private_path.with_extension("pub");
    if public_path == private_path {
        bail!(
            "{} is the public key path; pass the private key path to -o",
            private_path.display()
        );
    }
    if !request.force {
        for path in [&private_path, &public_path] {
            if path.exists() {
                bail!(
                    "{} already exists (use --force to overwrite)",
                    path.display()
                );
            }
        }
    }
    let signing = generate_signing_key()?;
    backup_existing_key(&private_path)?;
    write_private_key(&private_path, &signing)?;
    atomic_write(
        &public_path,
        format!("{}\n", BASE64.encode(signing.verifying_key().as_bytes())),
    )?;
    success(format!("Private key -> {}", display_path(&private_path)));
    success(format!("Public key  -> {}", display_path(&public_path)));
    Ok((private_path, public_path))
}

fn app_keygen(request: &UpdaterKeygenRequest) -> Result<String> {
    validate_app_id(&request.app_id)?;
    let private_path = request.out.clone().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| request.cwd.clone())
            .join(".tynd")
            .join("keys")
            .join(format!("{}.key", request.app_id))
    });
    if private_path.exists() && !request.force {
        bail!(
            "key already exists: {} (use --force only if key rotation is intentional)",
            private_path.display()
        );
    }
    let signing = generate_signing_key()?;
    let rotated = backup_existing_key(&private_path)?.is_some();
    write_private_key(&private_path, &signing)?;
    let public = BASE64.encode(signing.verifying_key().as_bytes());
    success(format!("Generated Ed25519 keypair for {}", request.app_id));
    step(format!("private -> {}", display_path(&private_path)));
    step(format!("public  -> {public}"));
    if request.edit_config {
        if let Some(path) = crate::config::find_config_in_directory(&request.cwd) {
            let mutation = if rotated {
                crate::config::ConfigMutation::AppendUpdaterPublicKey(&public)
            } else {
                crate::config::ConfigMutation::SetUpdaterPublicKey(&public)
            };
            match patch_updater_public_key(&path, mutation) {
                Ok(true) if rotated => {
                    success(format!(
                        "added the new public key to updater.publicKeys in {}",
                        display_path(&path)
                    ));
                    step(
                        "the previous public key stays listed so already-installed apps still \
                         verify updates; remove it once every install has updated",
                    );
                },
                Ok(true) => {
                    success(format!("updated {}", display_path(&path)));
                    if let Some(hint) = endpoints_hint(&path) {
                        step(hint);
                    }
                },
                Ok(false) if rotated => {
                    step(format!("{} already lists this public key", path.display()));
                },
                Ok(false) => warn(format!(
                    "{} already has updater.publicKeys; left unchanged",
                    path.display()
                )),
                Err(error) => warn(format!(
                    "could not edit {}: {error}; set updater.publicKeys manually",
                    path.display()
                )),
            }
        }
    }
    warn("Back up this private key. Losing it breaks future signed updates.");
    Ok(public)
}

fn generate_signing_key() -> Result<SigningKey> {
    let mut secret = [0_u8; 32];
    getrandom::fill(&mut secret)
        .map_err(|error| anyhow::anyhow!("generate Ed25519 key material: {error}"))?;
    Ok(SigningKey::from_bytes(&secret))
}

fn write_private_key(path: &Path, signing: &SigningKey) -> Result<()> {
    let mut der = Vec::with_capacity(48);
    der.extend_from_slice(&PKCS8_PREFIX);
    der.extend_from_slice(&signing.to_bytes());
    // `atomic_write` goes through a `tempfile` created with mode 0600 on Unix,
    // so the key is never readable by others, even before the chmod below.
    atomic_write(path, der)?;
    warn_if_not_gitignored(path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Renames an existing private key to `<stem>.bak-<epoch seconds>.<ext>` next
/// to it (so a `*.key` ignore rule still covers it), so `--force` never
/// destroys the only copy of a signing key.
fn backup_existing_key(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let backup = backup_path(path, epoch_seconds())?;
    if backup.exists() {
        bail!("backup {} already exists; retry", backup.display());
    }
    fs::rename(path, &backup)
        .with_context(|| format!("back up {} to {}", path.display(), backup.display()))?;
    warn(format!(
        "previous private key backed up to {}",
        display_path(&backup)
    ));
    warn_if_not_gitignored(&backup);
    Ok(Some(backup))
}

/// `dir/<stem>.key` -> `dir/<stem>.bak-<epoch>.key`; without an extension,
/// `dir/<name>` -> `dir/<name>.bak-<epoch>`.
fn backup_path(path: &Path, epoch: u64) -> Result<PathBuf> {
    let stem = path
        .file_stem()
        .context("private key path has no file name")?
        .to_string_lossy();
    let name = match path.extension() {
        Some(extension) => format!("{stem}.bak-{epoch}.{}", extension.to_string_lossy()),
        None => format!("{stem}.bak-{epoch}"),
    };
    Ok(path.with_file_name(name))
}

/// Warns when `path` sits inside a git work tree and no `.gitignore` between
/// the key and the work tree root covers it. A cheap line match, not git.
fn warn_if_not_gitignored(path: &Path) {
    let Ok(path) = absolute(path) else {
        return;
    };
    if git_root_ignoring(&path) == Some(false) {
        warn(format!(
            "{} is inside a git work tree and no .gitignore covers it; add `*.key` to \
             .gitignore so the private key is never committed",
            display_path(&path)
        ));
    }
}

/// `None` outside a git work tree; otherwise whether a `.gitignore` from the
/// key's directory up to the work tree root has a line covering `path`.
fn git_root_ignoring(path: &Path) -> Option<bool> {
    let mut ignored = false;
    for directory in path.ancestors().skip(1) {
        if !ignored {
            if let (Ok(source), Ok(relative)) = (
                fs::read_to_string(directory.join(".gitignore")),
                path.strip_prefix(directory),
            ) {
                let relative = relative.to_string_lossy().replace('\\', "/");
                ignored = source
                    .lines()
                    .any(|line| gitignore_line_covers(line, &relative));
            }
        }
        if directory.join(".git").exists() {
            return Some(ignored);
        }
    }
    None
}

/// Whether one `.gitignore` line covers `relative` (slash-separated, from the
/// `.gitignore` directory). Supports `*`/`?` wildcards, anchored patterns and
/// directory patterns; negations and comments never cover.
fn gitignore_line_covers(line: &str, relative: &str) -> bool {
    let pattern = line.trim();
    if pattern.is_empty() || pattern.starts_with('#') || pattern.starts_with('!') {
        return false;
    }
    let pattern = pattern.trim_end_matches('/');
    let components: Vec<&str> = relative.split('/').collect();
    // A leading `**/` matches in all directories: the rest is unanchored.
    if let Some(rest) = pattern.strip_prefix("**/") {
        let depth = rest.split('/').count();
        return (0..components.len())
            .filter(|start| start + depth <= components.len())
            .any(|start| wildcard_match(rest, &components[start..start + depth].join("/")));
    }
    if let Some(anchored) = pattern
        .strip_prefix('/')
        .or_else(|| pattern.contains('/').then_some(pattern))
    {
        let depth = anchored.split('/').count();
        return depth <= components.len()
            && wildcard_match(anchored, &components[..depth].join("/"));
    }
    components
        .iter()
        .any(|component| wildcard_match(pattern, component))
}

/// Glob match with `*` (any run) and `?` (one byte).
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let (pattern, text) = (pattern.as_bytes(), text.as_bytes());
    let (mut p, mut t, mut star, mut mark) = (0, 0, None, 0);
    while t < text.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if let Some(position) = star {
            p = position + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&byte| byte == b'*')
}

fn validate_app_id(app_id: &str) -> Result<()> {
    let pattern = regex::Regex::new(r"^[A-Za-z][A-Za-z0-9-]*(?:\.[A-Za-z0-9-]+){2,}$")?;
    if !pattern.is_match(app_id) {
        bail!("app id must be reverse-DNS, for example com.example.myapp");
    }
    Ok(())
}

/// Hint to set `updater.endpoints` when the config at `path` names none:
/// keygen writes only the keys, the manifest URL is the user's to choose.
fn endpoints_hint(path: &Path) -> Option<String> {
    let source = fs::read_to_string(path).ok()?;
    (!source.contains("endpoints")).then(|| {
        format!(
            "set updater.endpoints in {} to the URL of your update manifest; the config does not validate without it",
            display_path(path)
        )
    })
}

fn patch_updater_public_key(
    path: &Path,
    mutation: crate::config::ConfigMutation<'_>,
) -> Result<bool> {
    let source = fs::read_to_string(path)?;
    let next = crate::config::mutate_config_source(path, &source, mutation)?;
    if next == source {
        return Ok(false);
    }
    atomic_write(path, next)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_out_is_the_private_key_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("release.key");
        let (private, public) = generic_keygen(&KeygenRequest {
            out: out.clone(),
            force: false,
        })
        .expect("keygen");
        assert_eq!(private, out);
        assert_eq!(public, dir.path().join("release.pub"));
        assert!(super::super::read_private_key(&private).is_ok());
        assert!(generic_keygen(&KeygenRequest { out, force: false }).is_err());
    }

    #[test]
    fn generic_default_matches_the_sign_default() {
        use clap::Parser as _;
        let cli =
            crate::args::Cli::try_parse_from(["tynd", "updater", "sign", "file"]).expect("parse");
        let rendered = format!("{cli:?}");
        assert!(rendered.contains(GENERIC_PRIVATE_KEY), "{rendered}");
    }

    #[test]
    fn rotation_backs_up_the_key_and_appends_the_public_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("tynd.config.ts");
        fs::write(&config, "export default {};\n").expect("write config");
        let key = dir.path().join("app.key");
        let request = UpdaterKeygenRequest {
            cwd: dir.path().to_path_buf(),
            app_id: "com.example.app".to_owned(),
            out: Some(key.clone()),
            force: false,
            edit_config: true,
        };
        let first = app_keygen(&request).expect("first keygen");
        let old_secret = fs::read(&key).expect("read key");
        assert!(
            app_keygen(&request).is_err(),
            "no overwrite without --force"
        );
        let second = app_keygen(&UpdaterKeygenRequest {
            force: true,
            ..request
        })
        .expect("rotation");
        assert_ne!(first, second);
        let source = fs::read_to_string(&config).expect("read config");
        assert!(
            source.contains(&format!("publicKeys: [\"{first}\", \"{second}\"]")),
            "{source}"
        );
        let backups: Vec<_> = fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("app.bak-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read(backups[0].path()).expect("read backup"),
            old_secret
        );
    }

    #[test]
    fn app_keygen_writes_only_the_public_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("tynd.config.ts");
        fs::write(&config, "export default {};\n").expect("write config");
        let public = app_keygen(&UpdaterKeygenRequest {
            cwd: dir.path().to_path_buf(),
            app_id: "com.example.app".to_owned(),
            out: Some(dir.path().join("app.key")),
            force: false,
            edit_config: true,
        })
        .expect("keygen");
        let source = fs::read_to_string(&config).expect("read config");
        assert_eq!(
            source,
            format!("export default {{\n  updater: {{ publicKeys: [\"{public}\"] }}}};\n")
        );
        assert!(endpoints_hint(&config).is_some());
        fs::write(
            &config,
            "export default { updater: { endpoints: [\"https://u.test/m.json\"] } };\n",
        )
        .expect("write config");
        assert!(endpoints_hint(&config).is_none());
    }

    #[test]
    fn backup_keeps_the_key_extension() {
        let dir = Path::new("keys");
        assert_eq!(
            backup_path(&dir.join("tynd-updater.key"), 1_763_700_000).expect("backup path"),
            dir.join("tynd-updater.bak-1763700000.key")
        );
        assert_eq!(
            backup_path(&dir.join("signing"), 7).expect("backup path"),
            dir.join("signing.bak-7")
        );
    }

    #[test]
    fn gitignore_lines_cover_keys() {
        assert!(gitignore_line_covers("*.key", "tynd-updater.key"));
        assert!(gitignore_line_covers("*.key", "keys/app.key"));
        assert!(gitignore_line_covers("keys/", "keys/app.key"));
        assert!(gitignore_line_covers(
            "/tynd-updater.key",
            "tynd-updater.key"
        ));
        assert!(!gitignore_line_covers("/keys", "nested/keys/app.key"));
        assert!(!gitignore_line_covers("# *.key", "app.key"));
        assert!(!gitignore_line_covers("!*.key", "app.key"));
        assert!(!gitignore_line_covers("*.pub", "app.key"));
        assert!(gitignore_line_covers("**/*.key", "tynd-updater.key"));
        assert!(gitignore_line_covers("**/*.key", "keys/app.key"));
        assert!(gitignore_line_covers("**/keys/", "nested/keys/app.key"));
        assert!(gitignore_line_covers("**/keys/*.key", "a/keys/app.key"));
        assert!(!gitignore_line_covers("**/*.pub", "keys/app.key"));
        assert!(gitignore_line_covers(
            "*.key",
            "tynd-updater.bak-1763700000.key"
        ));
    }

    #[test]
    fn git_work_tree_detection_reads_gitignore() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = dir.path().join("sub").join("app.key");
        assert_eq!(git_root_ignoring(&key), None);
        fs::create_dir_all(dir.path().join(".git")).expect("git dir");
        assert_eq!(git_root_ignoring(&key), Some(false));
        fs::write(dir.path().join(".gitignore"), "node_modules/\n*.key\n").expect("gitignore");
        assert_eq!(git_root_ignoring(&key), Some(true));
    }
}

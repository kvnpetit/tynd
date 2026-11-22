use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context, Result};
use tynd_bundle_contract::{CodesignAction, CodesignConfig, CodesignIdentity, Notarization};

use crate::{
    config::{self, LoadedConfig},
    fsx::write_json_file,
    output::write_stdout,
    process::{self, CommandSpec},
};

use super::{
    packaging::platform_app_binary,
    shared::{ensure_existing_path_is_unlinked, ensure_success, generated_path},
};

fn resolve_signing_value(value: &str, loaded: &LoadedConfig, label: &str) -> Result<String> {
    if let Some(name) = value.strip_prefix("env:") {
        ensure!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|character| character == '_' || character.is_ascii_alphanumeric()),
            "{label} uses an invalid environment variable name"
        );
        return std::env::var(name)
            .ok()
            .or_else(|| loaded.environment.get(name).cloned())
            .with_context(|| format!("{label} environment variable {name} is not set"));
    }
    Ok(value.to_owned())
}

pub(super) fn sign_windows_artifacts(loaded: &LoadedConfig, artifacts: &[PathBuf]) -> Result<()> {
    let Some(signing) = loaded
        .config
        .bundle
        .as_ref()
        .and_then(|bundle| bundle.sign.as_ref())
        .and_then(|signing| signing.windows.as_ref())
    else {
        return Ok(());
    };
    if !cfg!(windows) {
        return Ok(());
    }
    let targets = artifacts
        .iter()
        .filter(|path| path.is_file() && has_extension(path, &["exe", "msi", "msix"]))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Ok(());
    }
    let signtool = process::find_executable("signtool")
        .context("Windows signing is configured but signtool was not found")?;
    let password = signing
        .password
        .as_deref()
        .map(|value| resolve_signing_value(value, loaded, "Windows certificate password"))
        .transpose()?;
    for target in targets {
        let (arguments, secret_index) =
            windows_signing_arguments(signing, password.as_deref(), target)?;
        let mut spec = CommandSpec::new(signtool.as_os_str())
            .args(arguments)
            .cwd(&loaded.root);
        if let Some(index) = secret_index {
            spec = spec.secret_arg(index);
        }
        ensure_success(process::run_streaming(spec)?, "signtool")?;
    }
    Ok(())
}

fn windows_signing_arguments(
    signing: &config::WindowsSigningConfig,
    password: Option<&str>,
    target: &Path,
) -> Result<(Vec<OsString>, Option<usize>)> {
    let mut arguments = vec![
        OsString::from("sign"),
        OsString::from("/fd"),
        OsString::from("SHA256"),
    ];
    let certificate = signing.certificate.trim();
    if let Some(subject) = certificate.strip_prefix("cert:subject=").or_else(|| {
        certificate
            .strip_prefix("cert:")
            .filter(|value| !value.starts_with("sha1="))
    }) {
        ensure!(
            !subject.trim().is_empty(),
            "Windows certificate subject is empty"
        );
        arguments.extend([OsString::from("/n"), OsString::from(subject)]);
    } else if let Some(thumbprint) = certificate
        .strip_prefix("cert:sha1=")
        .or_else(|| is_sha1_thumbprint(certificate).then_some(certificate))
    {
        ensure!(
            is_sha1_thumbprint(thumbprint),
            "Windows certificate SHA-1 thumbprint must contain exactly 40 hexadecimal characters"
        );
        arguments.extend([OsString::from("/sha1"), OsString::from(thumbprint)]);
    } else {
        let certificate = Path::new(certificate);
        ensure!(
            certificate.is_file(),
            "Windows certificate file not found: {}",
            certificate.display()
        );
        arguments.extend([OsString::from("/f"), certificate.as_os_str().to_os_string()]);
    }
    let secret_index = password.map(|password| {
        arguments.push(OsString::from("/p"));
        arguments.push(OsString::from(password));
        arguments.len() - 1
    });
    let timestamp = signing
        .timestamp_url
        .as_deref()
        .unwrap_or("http://timestamp.digicert.com");
    arguments.extend([
        OsString::from("/tr"),
        OsString::from(timestamp),
        OsString::from("/td"),
        OsString::from("SHA256"),
    ]);
    arguments.push(target.as_os_str().to_os_string());
    Ok((arguments, secret_index))
}

fn is_sha1_thumbprint(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

pub(super) fn sign_macos_artifacts(
    bundler: &Path,
    loaded: &LoadedConfig,
    artifacts: &[PathBuf],
) -> Result<()> {
    let Some(signing) = loaded
        .config
        .bundle
        .as_ref()
        .and_then(|bundle| bundle.sign.as_ref())
        .and_then(|signing| signing.macos.as_ref())
    else {
        return Ok(());
    };
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    ensure!(
        signing.identity.is_file(),
        "macOS PKCS#12 identity not found: {}",
        signing.identity.display()
    );
    let targets = artifacts
        .iter()
        .filter(|path| path.exists() && macos_signable(path))
        .cloned()
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Ok(());
    }
    // The pack trailer must stay discoverable once the signature exists:
    // `tynd_pack_format::PackReader` finds it just before `LC_CODE_SIGNATURE`.
    // If codesign drops or relocates the appended pack, fail the build here
    // instead of shipping an app that reports "--backend-entry is required".
    let packed = targets
        .iter()
        .flat_map(|target| packed_executables(target))
        .collect::<Vec<_>>();
    invoke_macos_codesign(
        bundler,
        loaded,
        signing,
        CodesignAction::Sign,
        &targets,
        None,
    )?;
    ensure_packs_survive_signing(&packed)?;
    let Some(notarization) = &signing.notarize else {
        return Ok(());
    };
    let notarize_targets = targets
        .iter()
        .filter(|path| has_extension(path, &["app", "dmg", "pkg"]))
        .cloned()
        .collect::<Vec<_>>();
    if notarize_targets.is_empty() {
        return Ok(());
    }
    let api_key = notarization_api_key(loaded, notarization)?;
    invoke_macos_codesign(
        bundler,
        loaded,
        signing,
        CodesignAction::Notarize,
        &notarize_targets,
        Some(&api_key),
    )
}

/// Resolves the notarization API key path (relative paths are rooted at the
/// project) and requires the file to exist.
fn notarization_api_key(
    loaded: &LoadedConfig,
    notarization: &config::MacosNotarizeConfig,
) -> Result<PathBuf> {
    let api_key = resolve_signing_value(
        &notarization.api_key,
        loaded,
        "macOS notarization API key path",
    )?;
    let api_key = PathBuf::from(api_key);
    let api_key = if api_key.is_absolute() {
        api_key
    } else {
        loaded.root.join(api_key)
    };
    ensure!(
        api_key.is_file(),
        "macOS notarization API key file not found: {}",
        api_key.display()
    );
    Ok(api_key)
}

/// Executables under `target` that currently carry a readable TYNDPKG section:
/// the file itself, or the files of an `.app` bundle's `Contents/MacOS`.
fn packed_executables(target: &Path) -> Vec<PathBuf> {
    let candidates = if target.is_dir() {
        fs::read_dir(target.join("Contents").join("MacOS"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        vec![target.to_path_buf()]
    };
    candidates
        .into_iter()
        .filter(|path| {
            path.is_file()
                && matches!(
                    tynd_pack_format::PackReader::open_path_checked(path),
                    Ok(Some(_))
                )
        })
        .collect()
}

/// Every executable in `packed` must still expose its pack after signing.
fn ensure_packs_survive_signing(packed: &[PathBuf]) -> Result<()> {
    for path in packed {
        match tynd_pack_format::PackReader::open_path_checked(path) {
            Ok(Some(_)) => {},
            Ok(None) => anyhow::bail!(
                "{} lost its packed app payload during macOS code signing; the TYNDPKG \
                 trailer must stay before the code signature (LC_CODE_SIGNATURE)",
                path.display()
            ),
            Err(error) => anyhow::bail!(
                "{} has a damaged TYNDPKG section after macOS code signing: {error}",
                path.display()
            ),
        }
    }
    Ok(())
}

fn has_extension(path: &Path, expected: &[&str]) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|actual| {
            expected
                .iter()
                .any(|candidate| actual.eq_ignore_ascii_case(candidate))
        })
}

fn macos_signable(path: &Path) -> bool {
    if path.is_dir() {
        return has_extension(path, &["app"]);
    }
    path.is_file()
        && path
            .extension()
            .and_then(OsStr::to_str)
            .is_none_or(|extension| {
                ["dmg", "pkg", "app"]
                    .iter()
                    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
            })
}

fn invoke_macos_codesign(
    bundler: &Path,
    loaded: &LoadedConfig,
    signing: &config::MacosSigningConfig,
    action: CodesignAction,
    targets: &[PathBuf],
    api_key: Option<&Path>,
) -> Result<()> {
    let password = signing
        .password
        .as_deref()
        .map(|value| resolve_signing_value(value, loaded, "macOS PKCS#12 password"))
        .transpose()?
        .unwrap_or_default();
    let value = CodesignConfig {
        action,
        targets: targets.to_vec(),
        identity: CodesignIdentity::P12 {
            path: signing.identity.clone(),
            password_env: "TYND_MACOS_P12_PASSWORD".to_owned(),
        },
        notarization: api_key.map(|path| Notarization {
            api_key_path: path.to_path_buf(),
        }),
        entitlements: signing.entitlements.clone(),
    };
    let action = action.as_str();
    let cache = generated_path(&loaded.root, ".tynd/cache")?;
    fs::create_dir_all(&cache)?;
    let path = cache.join(format!("codesign-{action}.json"));
    write_json_file(&path, &serde_json::to_value(&value)?)?;
    let status = process::run_streaming(
        CommandSpec::new(bundler.as_os_str())
            .args([OsString::from("codesign"), path.as_os_str().to_os_string()])
            .cwd(&loaded.root)
            .env("TYND_MACOS_P12_PASSWORD", password),
    )?;
    ensure_success(status, &format!("tynd-bundler codesign {action}"))
}

pub(super) fn archive_macos_apps(
    bundler: &Path,
    loaded: &LoadedConfig,
    app_name: &str,
    version: &str,
    artifacts: &[PathBuf],
) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let apps = app_bundles(artifacts);
    for (index, app) in apps.iter().enumerate() {
        let suffix = artifact_index_suffix(index, apps.len());
        let output = app.parent().unwrap_or(&loaded.root).join(format!(
            "{}-{version}-macos{suffix}.zip",
            platform_app_binary(app_name)
        ));
        let result = process::run_spec(
            CommandSpec::new(bundler.as_os_str())
                .args([
                    OsString::from("archive-app"),
                    OsString::from("--app"),
                    app.as_os_str().to_os_string(),
                    OsString::from("--output"),
                    output.as_os_str().to_os_string(),
                ])
                .cwd(&loaded.root),
        )?;
        echo_output(&result)?;
    }
    Ok(())
}

/// The `.app` bundle directories among `artifacts`.
fn app_bundles(artifacts: &[PathBuf]) -> Vec<&PathBuf> {
    artifacts
        .iter()
        .filter(|path| path.is_dir() && has_extension(path, &["app"]))
        .collect()
}

/// `-<n>` (1-based) when several artifacts share a name, empty otherwise.
fn artifact_index_suffix(index: usize, count: usize) -> String {
    if count == 1 {
        String::new()
    } else {
        format!("-{}", index + 1)
    }
}

/// Replays a captured child process's output on our own stdout/stderr.
fn echo_output(result: &process::ProcessOutput) -> Result<()> {
    if !result.stdout.is_empty() {
        write_stdout(result.stdout.as_bytes())?;
    }
    if !result.stderr.is_empty() {
        eprint!("{}", result.stderr);
    }
    Ok(())
}

pub(super) fn create_macos_dmgs(
    bundler: &Path,
    loaded: &LoadedConfig,
    app_name: &str,
    version: &str,
    artifacts: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    if !cfg!(target_os = "macos") {
        return Ok(Vec::new());
    }
    let apps = app_bundles(artifacts);
    ensure!(
        !apps.is_empty(),
        "signed DMG creation requires a generated .app bundle"
    );

    let architecture = match crate::project::current_architecture()? {
        crate::project::Architecture::X64 => "x64",
        crate::project::Architecture::Arm64 => "aarch64",
    };
    let root = fs::canonicalize(&loaded.root)?;
    let version = safe_artifact_component(version);
    let mut dmgs = Vec::with_capacity(apps.len());
    for (index, app) in apps.iter().enumerate() {
        let app = fs::canonicalize(app)
            .with_context(|| format!("resolve signed application {}", app.display()))?;
        ensure!(
            app.starts_with(&root),
            "signed application escapes project root: {}",
            app.display()
        );
        let output_dir = dmg_output_dir(&app)?;
        ensure_existing_path_is_unlinked(&root, &output_dir)?;
        fs::create_dir_all(&output_dir)
            .with_context(|| format!("create DMG output directory {}", output_dir.display()))?;

        let product_name = app
            .file_stem()
            .and_then(OsStr::to_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(app_name);
        let file_name = safe_artifact_component(product_name);
        let suffix = artifact_index_suffix(index, apps.len());
        let output = output_dir.join(format!("{file_name}_{version}_{architecture}{suffix}.dmg"));
        ensure_existing_path_is_unlinked(&root, &output)?;
        let result = process::run_spec(
            CommandSpec::new(bundler.as_os_str())
                .args([
                    OsString::from("dmg-from-app"),
                    OsString::from("--app"),
                    app.as_os_str().to_os_string(),
                    OsString::from("--output"),
                    output.as_os_str().to_os_string(),
                    OsString::from("--volume-name"),
                    OsString::from(file_name.clone()),
                ])
                .cwd(&loaded.root),
        )?;
        echo_output(&result)?;
        ensure!(
            output.is_file(),
            "tynd-bundler did not create expected DMG {}",
            output.display()
        );
        dmgs.push(output);
    }
    Ok(dmgs)
}

/// `<bundle root>/dmg`, where the bundle root is the `.app`'s parent, or its
/// grandparent when the `.app` sits in a `macos` directory.
fn dmg_output_dir(app: &Path) -> Result<PathBuf> {
    let app_parent = app.parent().context("signed application has no parent")?;
    let bundle_root = if app_parent
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.eq_ignore_ascii_case("macos"))
    {
        app_parent
            .parent()
            .context("macOS bundle directory has no parent")?
    } else {
        app_parent
    };
    Ok(bundle_root.join("dmg"))
}

fn safe_artifact_component(value: &str) -> String {
    let value = value
        .chars()
        .take(80)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let value = value.trim_matches('.');
    if value.is_empty() {
        "app".to_owned()
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use tynd_pack_format::{write_section_and_trailer, PackEntry};

    use super::*;

    fn pack_body() -> Vec<u8> {
        let mut body = Vec::new();
        write_section_and_trailer(&mut body, [PackEntry::new("bundle.js", b"x")])
            .expect("write pack");
        body
    }

    /// Thin 64-bit Mach-O header with no load commands, then `body`.
    fn unsigned_macho(body: &[u8]) -> Vec<u8> {
        let mut image = vec![0u8; 32];
        image[..4].copy_from_slice(&0xfeed_facfu32.to_le_bytes());
        image.extend_from_slice(body);
        image
    }

    /// What codesign is expected to produce: header with one
    /// `LC_CODE_SIGNATURE`, the pack padded to 16 bytes, then the signature.
    fn signed_macho(body: &[u8], signature: &[u8]) -> Vec<u8> {
        let mut image = vec![0u8; 32 + 16];
        image[..4].copy_from_slice(&0xfeed_facfu32.to_le_bytes());
        image[16..20].copy_from_slice(&1u32.to_le_bytes());
        image[20..24].copy_from_slice(&16u32.to_le_bytes());
        image.extend_from_slice(body);
        while !image.len().is_multiple_of(16) {
            image.push(0);
        }
        let offset = u32::try_from(image.len()).expect("small image");
        image[32..36].copy_from_slice(&0x1du32.to_le_bytes());
        image[36..40].copy_from_slice(&16u32.to_le_bytes());
        image[40..44].copy_from_slice(&offset.to_le_bytes());
        image[44..48].copy_from_slice(&(signature.len() as u32).to_le_bytes());
        image.extend_from_slice(signature);
        image
    }

    #[test]
    fn pack_survives_signing_when_it_precedes_the_signature() {
        let dir = tempfile::tempdir().expect("temp dir");
        let exe = dir.path().join("app");
        fs::write(&exe, unsigned_macho(&pack_body())).expect("write");
        let packed = packed_executables(&exe);
        assert_eq!(packed, vec![exe.clone()]);
        // Signing appends the blob after the pack.
        fs::write(&exe, signed_macho(&pack_body(), &[0xEE; 300])).expect("sign");
        ensure_packs_survive_signing(&packed).expect("pack still readable");
    }

    #[test]
    fn signing_that_drops_or_buries_the_pack_is_an_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let exe = dir.path().join("app");
        fs::write(&exe, unsigned_macho(&pack_body())).expect("write");
        let packed = packed_executables(&exe);
        // Signer rebuilt the image from its segments: pack gone.
        fs::write(&exe, signed_macho(&[], &[0xEE; 300])).expect("sign");
        let error = ensure_packs_survive_signing(&packed).expect_err("pack lost");
        assert!(error.to_string().contains("lost its packed app payload"));
        // Signature placed before the pack: trailer is no longer adjacent.
        let mut buried = signed_macho(&[], &[0xEE; 300]);
        buried.extend_from_slice(&pack_body());
        buried.extend_from_slice(&[0xAA; 40]);
        fs::write(&exe, buried).expect("sign");
        assert!(ensure_packs_survive_signing(&packed).is_err());
    }

    #[test]
    fn only_packed_executables_are_tracked_inside_an_app_bundle() {
        let dir = tempfile::tempdir().expect("temp dir");
        let macos = dir.path().join("App.app/Contents/MacOS");
        fs::create_dir_all(&macos).expect("mkdir");
        fs::write(macos.join("app"), unsigned_macho(&pack_body())).expect("write");
        fs::write(macos.join("helper"), unsigned_macho(&[1, 2, 3])).expect("write");
        let packed = packed_executables(&dir.path().join("App.app"));
        assert_eq!(packed, vec![macos.join("app")]);
        // A bundle without Contents/MacOS or an unpacked file tracks nothing.
        assert_eq!(
            packed_executables(&dir.path().join("Empty.app")),
            [] as [PathBuf; 0]
        );
        assert_eq!(
            packed_executables(&macos.join("helper")),
            [] as [PathBuf; 0]
        );
    }
}

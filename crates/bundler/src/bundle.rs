//! Translates `BundleConfig` (JSON contract with the native `tynd` CLI) into
//! `tauri_bundler::Settings` and runs `bundle_project`. Windows Authenticode
//! signing is invoked inline by tauri-bundler; macOS signing is delegated to
//! the `codesign` subcommand for cross-host (Linux CI) support.

use std::{
    fmt,
    io::Write as _,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use error_stack::{Report, ResultExt as _};
use handlebars::Handlebars;
use miette::Diagnostic;
use tauri_bundler::Settings;
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::{info, warn};
use tynd_bundle_contract::{
    AppImageConfig, BundleConfig, DebConfig, RpmConfig, WebviewInstallMode,
};

/// `error-stack` context markers for the bundle pipeline. Rendered into the
/// `cause` field of `BundleError::SettingsBuild` / `::BundleProject` at the
/// boundary so the user-facing diagnostic still carries the bundle metadata
/// (name, version, target list, binary path) rather than only the upstream
/// error's `Display`.
#[derive(Debug)]
pub(crate) enum BundleContext {
    BuildSettings,
    InvokeBundleProject,
}

impl fmt::Display for BundleContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuildSettings => f.write_str("assemble tauri-bundler Settings"),
            Self::InvokeBundleProject => f.write_str("run tauri-bundler bundle_project"),
        }
    }
}

impl std::error::Error for BundleContext {}

use crate::util;

mod settings;

#[derive(Debug)]
struct RemoveFileOnDrop(PathBuf);

impl Drop for RemoveFileOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `Clone` is required so `main.rs` can hand a value to `miette::Report::new`
/// after a downcast. Sources are rendered to `String` because `anyhow::Error`
/// is not `Clone`.
#[derive(Debug, Clone, Error, Diagnostic)]
pub(crate) enum BundleError {
    #[error("unknown bundle target: {target}")]
    #[diagnostic(
        code(tynd::bundler::unknown_target),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/guides/bundling"),
        help(
            "valid targets: msi, nsis, msix, dmg, app, deb, rpm, appimage, flatpak, snap, \
             pkgbuild. Check `targets` in your tynd.config.ts."
        )
    )]
    UnknownTarget { target: String },

    #[error("icon not found: {path}")]
    #[diagnostic(
        code(tynd::bundler::missing_icon),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/guides/bundling"),
        help(
            "every entry in `icons` must point to an existing .svg/.png/.ico file. Paths are \
             resolved relative to the project root."
        )
    )]
    MissingIcon { path: String },

    #[error("binary not found: {path}")]
    #[diagnostic(
        code(tynd::bundler::missing_binary),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/guides/bundling"),
        help(
            "the `binary_path` in BundleConfig must point to a built executable. Run your build \
             step before `tynd-bundler bundle`."
        )
    )]
    MissingBinary { path: String },

    #[error("project_root not found: {path}")]
    #[diagnostic(
        code(tynd::bundler::missing_project_root),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/guides/bundling"),
        help(
            "`project_root` must be an existing directory; the bundler writes intermediate \
             artifacts under it."
        )
    )]
    MissingProjectRoot { path: String },

    #[error("WebView2 fixed runtime directory not found: {path}")]
    #[diagnostic(
        code(tynd::bundler::missing_webview2_fixed_runtime),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/guides/bundling"),
        help(
            "`bundle.windows.webviewInstallMode.path` must name an existing project-relative \
             directory containing an extracted Microsoft WebView2 fixed runtime."
        )
    )]
    MissingWebView2FixedRuntime { path: String },

    #[error("WebView2 fixed runtime path must be project-relative: {path}")]
    #[diagnostic(
        code(tynd::bundler::invalid_webview2_fixed_runtime),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/guides/bundling"),
        help(
            "`bundle.windows.webviewInstallMode.path` cannot be absolute or contain empty, dot, \
             or parent segments."
        )
    )]
    InvalidWebView2FixedRuntime { path: String },

    #[error("failed to assemble bundler settings: {cause}")]
    #[diagnostic(
        code(tynd::bundler::settings_build),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/guides/bundling"),
        help(
            "tauri-bundler refused the BundleConfig. Common culprits: an unsupported `category`, \
             an empty `targets` list, or a binary path with the wrong arch."
        )
    )]
    SettingsBuild { cause: String },

    #[error("tauri-bundler failed: {cause}")]
    #[diagnostic(
        code(tynd::bundler::bundle_project),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/guides/bundling"),
        help(
            "the underlying tauri-bundler call failed. Check the lines above for stderr from \
             external tools (WiX, signtool, fpm, makensis, ...)."
        )
    )]
    BundleProject { cause: String },
}

pub(crate) fn run(cfg: &BundleConfig) -> Result<()> {
    validate_paths(cfg)?;
    log_binary_arch(cfg)?;

    let (icons, _generated_bundle_icon) = prepare_configured_icons(cfg)?;
    let template_data = bundle_template_data(cfg);
    let nsis_tmp_path = render_template_file(
        cfg.custom_nsis_template.as_deref(),
        &template_data,
        &cfg.name,
        "nsi",
        "NSIS",
    )?;
    let _nsis_defer = nsis_tmp_path.as_ref().map(|p| RemoveFileOnDrop(p.clone()));
    let wix_tmp_path = render_template_file(
        cfg.custom_wix_template.as_deref(),
        &template_data,
        &cfg.name,
        "wxs",
        "WiX",
    )?;
    let _wix_defer = wix_tmp_path.as_ref().map(|p| RemoveFileOnDrop(p.clone()));

    let (extra, tauri_targets) = split_targets(&cfg.targets);
    let mut artifacts = Vec::new();
    if !tauri_targets.is_empty() {
        let tauri_cfg = BundleConfig {
            targets: tauri_targets,
            ..cfg.clone()
        };
        let (settings, _generated_windows_icon) = settings::build(&tauri_cfg, &icons)?;
        artifacts = bundle_project(&tauri_cfg, &settings)?;
    }
    if extra.msix {
        let path = crate::msix::build(cfg, &cfg.icons)?;
        println!("{}", path.display());
        artifacts.push(path);
    }
    for path in crate::linux_packages::build(cfg, &extra.linux, &cfg.icons)? {
        println!("{}", path.display());
        artifacts.push(path);
    }
    archive_artifacts(cfg, &artifacts)
}

/// Targets Tynd builds itself rather than through tauri-bundler.
#[derive(Debug, Default, PartialEq, Eq)]
struct ExtraTargets {
    msix: bool,
    linux: Vec<crate::linux_packages::LinuxPackage>,
}

fn split_targets(targets: &[String]) -> (ExtraTargets, Vec<String>) {
    let mut extra = ExtraTargets::default();
    let mut tauri = Vec::new();
    for target in targets {
        if target == "msix" {
            extra.msix = true;
        } else if let Some(package) = crate::linux_packages::LinuxPackage::parse(target) {
            extra.linux.push(package);
        } else {
            tauri.push(target.clone());
        }
    }
    (extra, tauri)
}

fn log_binary_arch(cfg: &BundleConfig) -> Result<()> {
    let binary_utf8 = util::to_utf8_path(&cfg.binary_path)?;
    let arch = util::detect_binary_arch(&binary_utf8)?;
    info!("binary arch: {arch}");
    if arch == "unknown" {
        warn!(
            "could not detect arch for '{}' — ensure it is a valid ELF / PE / Mach-O binary",
            cfg.binary_path.display()
        );
    }
    Ok(())
}

fn prepare_configured_icons(cfg: &BundleConfig) -> Result<(Vec<PathBuf>, Option<NamedTempFile>)> {
    if cfg.icons.is_empty() {
        // A fresh scaffold has no icon; the host default is a valid result,
        // not a problem to warn about on every build.
        info!(
            "no icon configured, using the default host icon — set `icon` in tynd.config.ts to \
             brand the app and its installers"
        );
    }
    prepare_bundle_icons(&cfg.icons)
}

fn bundle_template_data(cfg: &BundleConfig) -> serde_json::Value {
    serde_json::json!({
        "name": cfg.name,
        "version": cfg.version,
        "identifier": cfg.identifier,
        "description": cfg.description,
        "homepage": cfg.homepage,
        "authors": cfg.authors,
        "license": cfg.license,
        "copyright": cfg.copyright,
    })
}

fn render_template_file(
    template: Option<&str>,
    data: &serde_json::Value,
    name: &str,
    extension: &str,
    label: &str,
) -> Result<Option<PathBuf>> {
    let path = template
        .map(|template| -> Result<PathBuf> {
            let rendered = render_custom_template(template, data)?;
            let path = std::env::temp_dir().join(format!("tynd-bundler-{name}.{extension}"));
            std::fs::write(&path, rendered.as_bytes())
                .with_context(|| format!("write {} temp file", label.to_ascii_lowercase()))?;
            Ok(path)
        })
        .transpose()?;
    if let Some(path) = &path {
        info!("rendered {label} template: {}", path.display());
    }
    Ok(path)
}

fn bundle_project(cfg: &BundleConfig, settings: &Settings) -> Result<Vec<PathBuf>> {
    info!("bundling {} v{}", cfg.name, cfg.version);
    let bundles = tauri_bundler::bundle_project(settings)
        .map_err(|e| {
            Report::new(BundleContext::InvokeBundleProject).attach(format!("source = {e:#}"))
        })
        .attach_with({
            let n = cfg.name.clone();
            move || format!("name = {n}")
        })
        .attach_with({
            let v = cfg.version.clone();
            move || format!("version = {v}")
        })
        .attach_with({
            let t = cfg.targets.join(",");
            move || format!("targets = [{t}]")
        })
        .map_err(|report| {
            anyhow::Error::new(BundleError::BundleProject {
                cause: format!("{report:?}"),
            })
        })?;

    let mut artifact_paths: Vec<PathBuf> = Vec::new();
    for bundle in &bundles {
        for path in &bundle.bundle_paths {
            println!("{}", path.display());
            artifact_paths.push(path.clone());
        }
    }
    Ok(artifact_paths)
}

fn archive_artifacts(cfg: &BundleConfig, artifact_paths: &[PathBuf]) -> Result<()> {
    if !artifact_paths.is_empty() {
        let out_utf8 = util::to_utf8_path(&cfg.out_dir)?;
        let zip_path = out_utf8.join(format!("{}-{}-artifacts.zip", cfg.name, cfg.version));
        match util::zip_artifacts(artifact_paths, &zip_path) {
            Ok(()) => info!(
                "artifact archive: {}",
                util::display_path(zip_path.as_std_path())
            ),
            Err(e) => warn!("could not create artifact zip: {e:#}"),
        }
    }

    Ok(())
}
fn render_custom_template(template: &str, data: &serde_json::Value) -> Result<String> {
    let mut hbs = Handlebars::new();
    hbs.register_template_string("t", template)
        .context("invalid template")?;
    hbs.render("t", data).context("render template")
}

fn prepare_bundle_icons(
    configured_icons: &[PathBuf],
) -> Result<(Vec<PathBuf>, Option<NamedTempFile>)> {
    if !configured_icons.is_empty() {
        return Ok((configured_icons.to_vec(), None));
    }

    let mut temporary = tempfile::Builder::new()
        .prefix("tynd-bundle-icon-")
        .suffix(".png")
        .tempfile()
        .context("create temporary bundle icon")?;
    temporary
        .write_all(&crate::pack::icons::default_png_bytes()?)
        .context("write temporary bundle icon")?;
    temporary.flush().context("flush temporary bundle icon")?;
    Ok((vec![temporary.path().to_path_buf()], Some(temporary)))
}

fn validate_paths(cfg: &BundleConfig) -> Result<()> {
    if !cfg.project_root.exists() {
        return Err(anyhow::Error::new(BundleError::MissingProjectRoot {
            path: cfg.project_root.display().to_string(),
        }));
    }
    if !cfg.binary_path.exists() {
        return Err(anyhow::Error::new(BundleError::MissingBinary {
            path: cfg.binary_path.display().to_string(),
        }));
    }
    for icon in &cfg.icons {
        if !exists_relaxed(icon, &cfg.project_root) {
            return Err(anyhow::Error::new(BundleError::MissingIcon {
                path: icon.display().to_string(),
            }));
        }
    }
    if let Some(WebviewInstallMode::FixedRuntime { path }) = &cfg.windows.webview_install_mode {
        let path_text = path.to_string_lossy();
        if tynd_pack_format::validate_subpath(&path_text).is_none() {
            return Err(anyhow::Error::new(
                BundleError::InvalidWebView2FixedRuntime {
                    path: path_text.into_owned(),
                },
            ));
        }
        let resolved = cfg.project_root.join(path);
        if !resolved.is_dir() || !resolved.join("msedgewebview2.exe").is_file() {
            return Err(anyhow::Error::new(
                BundleError::MissingWebView2FixedRuntime {
                    path: path.display().to_string(),
                },
            ));
        }
    }
    Ok(())
}

/// Accept absolute or root-relative paths — mirrors tauri-bundler's logic.
fn exists_relaxed(p: &Path, root: &Path) -> bool {
    if p.is_absolute() {
        p.exists()
    } else {
        p.exists() || root.join(p).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux_packages::LinuxPackage;

    #[test]
    fn tynd_built_targets_are_split_from_tauri_targets() {
        let targets = ["nsis", "msix", "deb", "flatpak", "snap", "pkgbuild"].map(String::from);
        let (extra, tauri) = split_targets(&targets);
        assert_eq!(tauri, ["nsis", "deb"]);
        assert_eq!(
            extra,
            ExtraTargets {
                msix: true,
                linux: vec![
                    LinuxPackage::Flatpak,
                    LinuxPackage::Snap,
                    LinuxPackage::Pkgbuild
                ],
            }
        );
    }
}

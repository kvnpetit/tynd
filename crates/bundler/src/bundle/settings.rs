use std::{
    io::Write as _,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use error_stack::{Report, ResultExt as _};
use rayon::prelude::*;
use tauri_bundler::{
    AppCategory, AppImageSettings, BundleBinary, BundleSettings, DebianSettings, DmgSettings,
    Entitlements, MacOsSettings, NsisSettings, PackageSettings, PackageType, PlistKind,
    RpmSettings, Settings, SettingsBuilder, WindowsSettings,
};
use tauri_utils::config::{
    AssociationExt, BundleTypeRole, DeepLinkProtocol, ExportedFileAssociation, FileAssociation,
    HandlerRank, NSISInstallerMode, WebviewInstallMode,
};
use tempfile::NamedTempFile;
use tracing::info;
use tynd_bundle_contract as contract;

use super::{AppImageConfig, BundleConfig, BundleContext, BundleError, DebConfig, RpmConfig};

pub(super) fn build(
    cfg: &BundleConfig,
    icons: &[PathBuf],
) -> Result<(Settings, Option<NamedTempFile>)> {
    let package = package_settings(cfg);
    let (bundle, generated_windows_icon) = bundle_settings(cfg, icons)?;
    let settings = build_settings(cfg, package, bundle)?;
    Ok((settings, generated_windows_icon))
}

/// Contract → tauri-utils. Field-by-field so a change on either side is a
/// compile error here instead of a silently dropped JSON key.
fn file_association(association: &contract::FileAssociation) -> FileAssociation {
    FileAssociation {
        ext: association
            .ext
            .iter()
            .map(|ext| AssociationExt(ext.trim_start_matches('.').to_owned()))
            .collect(),
        content_types: association.content_types.clone(),
        name: association.name.clone(),
        description: association.description.clone(),
        role: match association.role {
            contract::FileAssociationRole::Editor => BundleTypeRole::Editor,
            contract::FileAssociationRole::Viewer => BundleTypeRole::Viewer,
            contract::FileAssociationRole::Shell => BundleTypeRole::Shell,
            contract::FileAssociationRole::QlGenerator => BundleTypeRole::QLGenerator,
            contract::FileAssociationRole::None => BundleTypeRole::None,
        },
        mime_type: association.mime_type.clone(),
        rank: match association.rank {
            contract::FileAssociationRank::Default => HandlerRank::Default,
            contract::FileAssociationRank::Owner => HandlerRank::Owner,
            contract::FileAssociationRank::Alternate => HandlerRank::Alternate,
            contract::FileAssociationRank::None => HandlerRank::None,
        },
        exported_type: association
            .exported_type
            .as_ref()
            .map(|exported| ExportedFileAssociation {
                identifier: exported.identifier.clone(),
                conforms_to: exported.conforms_to.clone(),
            }),
        android_intent_action_filters: None,
    }
}

fn webview_install_mode(mode: &contract::WebviewInstallMode) -> WebviewInstallMode {
    match mode {
        contract::WebviewInstallMode::Skip => WebviewInstallMode::Skip,
        contract::WebviewInstallMode::DownloadBootstrapper { silent } => {
            WebviewInstallMode::DownloadBootstrapper { silent: *silent }
        },
        contract::WebviewInstallMode::EmbedBootstrapper { silent } => {
            WebviewInstallMode::EmbedBootstrapper { silent: *silent }
        },
        contract::WebviewInstallMode::OfflineInstaller { silent } => {
            WebviewInstallMode::OfflineInstaller { silent: *silent }
        },
        contract::WebviewInstallMode::FixedRuntime { path } => {
            WebviewInstallMode::FixedRuntime { path: path.clone() }
        },
    }
}

fn package_settings(cfg: &BundleConfig) -> PackageSettings {
    PackageSettings {
        product_name: cfg.name.clone(),
        version: cfg.version.clone(),
        description: cfg.description.clone().unwrap_or_default(),
        homepage: cfg.homepage.clone(),
        authors: cfg.authors.clone(),
        default_run: None,
    }
}

fn bundle_settings(
    cfg: &BundleConfig,
    icons: &[PathBuf],
) -> Result<(BundleSettings, Option<NamedTempFile>)> {
    let (windows, generated_windows_icon) = prepare_windows_settings(cfg, icons)?;
    let bundle_icons = icons
        .iter()
        .map(PathBuf::as_path)
        .chain(generated_windows_icon.as_ref().map(NamedTempFile::path))
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let settings = BundleSettings {
        identifier: Some(cfg.identifier.clone()),
        publisher: cfg.authors.as_ref().and_then(|a| a.first().cloned()),
        homepage: cfg.homepage.clone(),
        icon: Some(bundle_icons),
        resources: Some(
            cfg.resources
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        ),
        file_associations: if cfg.file_associations.is_empty() {
            None
        } else {
            Some(cfg.file_associations.iter().map(file_association).collect())
        },
        copyright: cfg.copyright.clone(),
        license: cfg.license.clone(),
        license_file: cfg.license_file.clone(),
        category: cfg.category.as_deref().and_then(parse_category),
        short_description: cfg.description.clone(),
        long_description: cfg.long_description.clone(),
        external_bin: None,
        deep_link_protocols: if cfg.macos.url_schemes.is_empty() {
            None
        } else {
            Some(vec![DeepLinkProtocol {
                schemes: cfg.macos.url_schemes.clone(),
                domains: Vec::new(),
                name: None,
                role: BundleTypeRole::Editor,
            }])
        },
        deb: linux_deb(&cfg.linux.deb),
        rpm: linux_rpm(&cfg.linux.rpm),
        appimage: linux_appimage(&cfg.linux.appimage),
        dmg: DmgSettings::default(),
        macos: macos_settings(&cfg.macos)?,
        updater: None,
        windows,
        ..Default::default()
    };
    Ok((settings, generated_windows_icon))
}

fn macos_settings(cfg: &contract::MacOsConfig) -> Result<MacOsSettings> {
    let minimum = match &cfg.minimum_system_version {
        Some(version) => {
            anyhow::ensure!(
                version_at_least(version, contract::MACOS_MINIMUM_SYSTEM_VERSION),
                "macos.minimum_system_version {version:?} is below the minimum Tynd supports ({}); raise it or leave it unset",
                contract::MACOS_MINIMUM_SYSTEM_VERSION
            );
            version.clone()
        },
        None => contract::MACOS_MINIMUM_SYSTEM_VERSION.to_owned(),
    };
    Ok(MacOsSettings {
        minimum_system_version: Some(minimum),
        entitlements: cfg.entitlements.clone().map(Entitlements::Path),
        info_plist: cfg.info_plist.clone().map(PlistKind::Path),
        ..MacOsSettings::default()
    })
}

/// Dotted numeric comparison; a malformed `version` never passes.
fn version_at_least(version: &str, minimum: &str) -> bool {
    let parse = |text: &str| {
        text.split('.')
            .map(|part| part.parse::<u32>().ok())
            .collect::<Option<Vec<_>>>()
    };
    let (Some(mut version), Some(mut minimum)) = (parse(version), parse(minimum)) else {
        return false;
    };
    let len = version.len().max(minimum.len());
    version.resize(len, 0);
    minimum.resize(len, 0);
    version >= minimum
}

fn build_settings(
    cfg: &BundleConfig,
    package: PackageSettings,
    bundle: BundleSettings,
) -> Result<Settings> {
    let (binary_name, binary_directory) = binary_layout(&cfg.binary_path)?;
    let binary = BundleBinary::new(binary_name, true);
    let package_types: Vec<PackageType> = cfg
        .targets
        .par_iter()
        .map(String::as_str)
        .map(parse_target)
        .collect::<Result<Vec<_>>>()?;

    let pkg_count = package_types.len();
    SettingsBuilder::new()
        .package_settings(package)
        .bundle_settings(bundle)
        .binaries(vec![binary])
        .project_out_directory(binary_directory)
        .package_types(package_types)
        .build()
        .map_err(|e| Report::new(BundleContext::BuildSettings).attach(format!("source = {e:#}")))
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
        .attach_with({
            let p = cfg.binary_path.display().to_string();
            move || format!("binary_path = {p}")
        })
        .attach_with(move || format!("package_type_count = {pkg_count}"))
        .map_err(|report| {
            anyhow::Error::new(BundleError::SettingsBuild {
                cause: format!("{report:?}"),
            })
        })
}

/// Unknown values fail loudly so a typo doesn't silently skip a format.
fn parse_target(t: &str) -> Result<PackageType> {
    Ok(match t {
        "msi" | "wix" => PackageType::WindowsMsi,
        "nsis" => PackageType::Nsis,
        "dmg" => PackageType::Dmg,
        "app" | "macos" => PackageType::MacOsBundle,
        "deb" => PackageType::Deb,
        "rpm" => PackageType::Rpm,
        "appimage" => PackageType::AppImage,
        other => {
            return Err(anyhow::Error::new(BundleError::UnknownTarget {
                target: other.to_string(),
            }));
        },
    })
}

fn binary_layout(binary_path: &Path) -> Result<(String, PathBuf)> {
    let name = binary_path
        .file_stem()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| {
            anyhow::Error::new(BundleError::MissingBinary {
                path: binary_path.display().to_string(),
            })
        })?;
    let directory = binary_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    Ok((name, directory))
}

fn prepare_windows_settings(
    cfg: &BundleConfig,
    icons: &[PathBuf],
) -> Result<(WindowsSettings, Option<NamedTempFile>)> {
    let mut settings = WindowsSettings {
        webview_install_mode: cfg
            .windows
            .webview_install_mode
            .as_ref()
            .map_or(WebviewInstallMode::Skip, webview_install_mode),
        ..WindowsSettings::default()
    };
    if let Some(version) = &cfg.windows.minimum_webview2_version {
        settings.minimum_webview2_version = Some(version.clone());
    }
    if let Some(allow) = cfg.windows.allow_downgrades {
        settings.allow_downgrades = allow;
    }
    if let Some(mode) = cfg.windows.install_mode {
        settings.nsis = Some(NsisSettings {
            install_mode: match mode {
                contract::NsisInstallMode::CurrentUser => NSISInstallerMode::CurrentUser,
                contract::NsisInstallMode::PerMachine => NSISInstallerMode::PerMachine,
                contract::NsisInstallMode::Both => NSISInstallerMode::Both,
            },
            ..NsisSettings::default()
        });
    }
    if !cfg
        .targets
        .iter()
        .any(|target| target == "msi" || target == "wix")
    {
        return Ok((settings, None));
    }

    if let Some(icon) = windows_icon_path(icons, None) {
        set_windows_icon_path(&mut settings, icon);
        return Ok((settings, None));
    }

    let bytes = if let Some(source) = icons.iter().find(|icon| {
        icon.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("png") || extension.eq_ignore_ascii_case("svg")
            })
    }) {
        info!(
            "converting '{}' to ICO for the MSI installer",
            source.display()
        );
        crate::pack::icons::build_ico_bytes(source)?
    } else {
        info!("using generated fallback ICO for the MSI installer");
        crate::pack::icons::default_ico_bytes()?
    };

    let mut temporary = tempfile::Builder::new()
        .prefix("tynd-installer-")
        .suffix(".ico")
        .tempfile()
        .context("create temporary MSI icon")?;
    temporary
        .write_all(&bytes)
        .context("write temporary MSI icon")?;
    temporary.flush().context("flush temporary MSI icon")?;
    if let Some(icon) = windows_icon_path(icons, Some(temporary.path())) {
        set_windows_icon_path(&mut settings, icon);
    }
    Ok((settings, Some(temporary)))
}

/// The ICO WiX embeds in the MSI: the user's own `.ico` when one is configured,
/// else the ICO generated for this build. Without it tauri falls back to its
/// default `icons/icon.ico`, which a Tynd project does not have.
fn windows_icon_path(icons: &[PathBuf], generated: Option<&Path>) -> Option<PathBuf> {
    icons
        .iter()
        .find(|icon| {
            icon.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("ico"))
        })
        .cloned()
        .or_else(|| generated.map(Path::to_path_buf))
}

// `icon_path` is deprecated upstream in favour of an `.ico` in
// `BundleSettings::icon`, but the MSI bundler still reads it first and only
// falls back to the icon list when it is empty.
#[allow(deprecated)]
fn set_windows_icon_path(settings: &mut WindowsSettings, icon: PathBuf) {
    settings.icon_path = icon;
}

fn parse_category(s: &str) -> Option<AppCategory> {
    use std::str::FromStr;
    AppCategory::from_str(s).ok()
}

fn linux_deb(cfg: &DebConfig) -> DebianSettings {
    DebianSettings {
        depends: option_vec(&cfg.depends),
        recommends: option_vec(&cfg.recommends),
        provides: option_vec(&cfg.provides),
        conflicts: option_vec(&cfg.conflicts),
        replaces: option_vec(&cfg.replaces),
        priority: cfg.priority.clone(),
        section: cfg.section.clone(),
        desktop_template: cfg.desktop_template.clone(),
        ..Default::default()
    }
}

fn linux_rpm(cfg: &RpmConfig) -> RpmSettings {
    RpmSettings {
        depends: option_vec(&cfg.depends),
        release: cfg.release.clone().unwrap_or_else(|| "1".into()),
        epoch: cfg.epoch.unwrap_or(0),
        desktop_template: cfg.desktop_template.clone(),
        ..Default::default()
    }
}

// `bundle_xdg_open` is deprecated upstream (Bundling xdg-open in an AppImage does
// not work and is ignored) but stays a documented config field, so it is still
// forwarded.
#[allow(deprecated)]
fn linux_appimage(cfg: &AppImageConfig) -> AppImageSettings {
    AppImageSettings {
        bundle_xdg_open: cfg.bundle_xdg_open,
        ..Default::default()
    }
}

fn option_vec(v: &[String]) -> Option<Vec<String>> {
    if v.is_empty() {
        None
    } else {
        Some(v.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_icon_path_prefers_configured_ico() {
        let icons = vec![
            PathBuf::from("assets/logo.png"),
            PathBuf::from("assets/app.ICO"),
        ];
        assert_eq!(
            windows_icon_path(&icons, Some(Path::new("tmp/generated.ico"))),
            Some(PathBuf::from("assets/app.ICO"))
        );
    }

    #[test]
    fn windows_icon_path_falls_back_to_generated_ico() {
        let icons = vec![PathBuf::from("assets/logo.png")];
        assert_eq!(
            windows_icon_path(&icons, Some(Path::new("tmp/generated.ico"))),
            Some(PathBuf::from("tmp/generated.ico"))
        );
        assert_eq!(
            windows_icon_path(&[], Some(Path::new("tmp/generated.ico"))),
            Some(PathBuf::from("tmp/generated.ico"))
        );
    }

    #[test]
    fn windows_icon_path_is_none_without_any_ico() {
        let icons = vec![PathBuf::from("assets/logo.png")];
        assert_eq!(windows_icon_path(&icons, None), None);
    }

    #[test]
    fn macos_bundle_declares_the_minimum_system_version() {
        let cfg: BundleConfig = serde_json::from_value(serde_json::json!({
            "name": "Demo",
            "version": "1.0.0",
            "identifier": "dev.tynd.demo",
            "targets": ["app"],
            "project_root": "/project",
            "out_dir": "/project/release",
            "binary_path": "/project/release/demo",
            "resources": [],
            "icons": [],
        }))
        .expect("bundle config");
        let (bundle, _) = bundle_settings(&cfg, &[]).expect("bundle settings");
        assert_eq!(
            bundle.macos.minimum_system_version.as_deref(),
            Some(contract::MACOS_MINIMUM_SYSTEM_VERSION)
        );
    }

    fn minimal_config(extra: serde_json::Value) -> BundleConfig {
        let mut value = serde_json::json!({
            "name": "Demo",
            "version": "1.0.0",
            "identifier": "dev.tynd.demo",
            "targets": ["nsis"],
            "project_root": "/project",
            "out_dir": "/project/release",
            "binary_path": "/project/release/demo",
            "resources": [],
            "icons": [],
        });
        if let (Some(target), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
            target.extend(extra.clone());
        }
        serde_json::from_value(value).expect("bundle config")
    }

    #[test]
    fn macos_metadata_is_forwarded() {
        let cfg = minimal_config(serde_json::json!({
            "macos": {
                "minimum_system_version": "15.1",
                "entitlements": "app.entitlements",
                "info_plist": "Extra.plist",
            },
            "license_file": "LICENSE",
        }));
        let (bundle, _) = bundle_settings(&cfg, &[]).expect("bundle settings");
        assert_eq!(bundle.macos.minimum_system_version.as_deref(), Some("15.1"));
        assert!(matches!(
            bundle.macos.entitlements,
            Some(Entitlements::Path(ref path)) if path == Path::new("app.entitlements")
        ));
        assert!(matches!(
            bundle.macos.info_plist,
            Some(PlistKind::Path(ref path)) if path == Path::new("Extra.plist")
        ));
        assert_eq!(bundle.license_file, Some(PathBuf::from("LICENSE")));
    }

    #[test]
    fn macos_minimum_below_the_supported_floor_is_rejected() {
        let cfg = minimal_config(serde_json::json!({
            "macos": { "minimum_system_version": "13.6" },
        }));
        assert!(bundle_settings(&cfg, &[]).is_err());
        assert!(version_at_least("14", "14.0"));
        assert!(version_at_least("14.0.1", "14.0"));
        assert!(!version_at_least("fourteen", "14.0"));
    }

    #[test]
    fn nsis_install_mode_and_desktop_templates_are_forwarded() {
        let cfg = minimal_config(serde_json::json!({
            "windows": { "install_mode": "perMachine" },
            "linux": {
                "deb": { "desktop_template": "deb.desktop" },
                "rpm": { "desktop_template": "rpm.desktop" },
            },
        }));
        let (bundle, _) = bundle_settings(&cfg, &[]).expect("bundle settings");
        let nsis = bundle.windows.nsis.expect("nsis settings");
        assert_eq!(nsis.install_mode, NSISInstallerMode::PerMachine);
        assert_eq!(
            bundle.deb.desktop_template,
            Some(PathBuf::from("deb.desktop"))
        );
        assert_eq!(
            bundle.rpm.desktop_template,
            Some(PathBuf::from("rpm.desktop"))
        );
    }
}

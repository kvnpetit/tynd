use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use serde_json::Value;
use tynd_bundle_contract as contract;

use crate::{
    args::BuildArgs,
    config::{
        self, BundleConfig, DebBundleConfig, ExecutionLevel, FileAssociation, FileAssociationRank,
        FileAssociationRole, LoadedConfig, NsisInstallMode, RpmBundleConfig, TyndConfig,
        WebView2InstallMode, WindowsBundleConfig,
    },
    fsx::{absolute, atomic_write, write_json_file},
    output::{display_path, success, warn, write_line},
    process::{self, CommandSpec},
};

use super::{
    lifecycle::resolve_backend_entry,
    plugin_imports::ensure_plugin_imports_declared,
    shared::{
        app_id_from_config, copy_tree_strict, ensure_existing_path_is_unlinked, ensure_success,
        generated_path, is_link_or_reparse, project_name, replace_generated_directory,
        resolve_bun_runtime, with_project_environment,
    },
    signing::{
        archive_macos_apps, create_macos_dmgs, sign_macos_artifacts, sign_windows_artifacts,
    },
    webview::{fixed_webview_runtime, stage_fixed_webview_runtime},
};

pub(in crate::commands) fn build(args: &BuildArgs) -> Result<()> {
    let requested_targets = args.bundle.as_deref().map(bundle_targets).transpose()?;
    let loaded = config::load_typed(&absolute(&args.cwd)?, None)?;
    if requested_targets.is_some() {
        ensure!(
            loaded.config.bundle.is_some(),
            "installer bundles require a `bundle` section with `bundle.identifier` in tynd.config.ts, for example:\n\n  bundle: {{\n    identifier: \"com.example.myapp\",\n    displayName: \"My App\",\n  }},\n\nSee https://tynd-apps.github.io/tynd/docs/v1.0/guides/bundling#required-config"
        );
    }
    ensure_plugin_imports_declared(&loaded)?;
    let frontend = crate::project::detect_frontend(&loaded.root)?;
    if let Some(blocked) = &frontend.blocked_by {
        bail!("{blocked} is server-rendered; Tynd requires a static SPA")
    }
    if let Some(command) = frontend.build_command.as_ref() {
        run_plan(command, &loaded)?;
    }
    let backend = build_backend(&loaded)?;
    let frontend_dir = build_frontend_assets(&loaded, &frontend)?;
    let host = crate::project::build_custom_host(&loaded.root, &loaded.config, true)?;
    let bundler = crate::project::resolve_binary(crate::project::TyndBinary::Bundler)?;
    let package = crate::project::load_package_json(&loaded.root)?.unwrap_or_default();
    let release_root = generated_path(&loaded.root, "release")?;
    let platform_dir = release_root.join(format!(
        "{}-{}",
        crate::project::current_platform()?.npm_name(),
        crate::project::current_architecture()?.npm_name()
    ));
    ensure_existing_path_is_unlinked(&fs::canonicalize(&loaded.root)?, &platform_dir)?;
    fs::create_dir_all(&platform_dir)?;
    let app_name = loaded
        .config
        .bundle
        .as_ref()
        .and_then(|value| value.display_name.clone())
        .or(package.name.clone())
        .unwrap_or_else(|| project_name(&loaded.root));
    let version = package.version.clone().unwrap_or_else(|| "0.0.0".into());
    let output = prepare_build_output(&loaded.root, args.out.as_deref(), &platform_dir, &app_name)?;
    let bun = resolve_bun_runtime(&loaded.root)?;
    let fixed_webview = fixed_webview_runtime(&loaded)?;
    let pack_config = contract::PackConfig {
        host_binary: host,
        output: output.clone(),
        icon: loaded.config.icon.clone(),
        app_name: app_name.clone(),
        version: version.clone(),
        backend_bundle: backend,
        frontend_dir,
        sidecars: loaded
            .config
            .sidecars
            .iter()
            .map(|sidecar| contract::SidecarSpec {
                name: sidecar.name.clone(),
                path: sidecar.path.clone(),
            })
            .collect(),
        bun_runtime: Some(bun.path),
        bun_version: Some(bun.version),
        webview2_runtime_path: fixed_webview
            .as_ref()
            .map(|runtime| runtime.relative.clone()),
        observability_json: config::resolve_observability_json(&loaded.config)?,
        app_identifier: app_id_from_config(&loaded.config),
        debug: args.debug_protocol,
        windows: contract::WindowsPackOptions {
            embed_icon: true,
            patch_subsystem_gui: true,
            requested_execution_level: loaded
                .config
                .bundle
                .as_ref()
                .and_then(|bundle| bundle.windows.as_ref())
                .and_then(|windows| windows.requested_execution_level)
                .map(execution_level),
        },
    };
    if args.debug_protocol {
        warn(
            "--debug-protocol: this build lets any local process drive the app through its \
             debug socket; do not distribute it",
        );
    }
    invoke_bundler(&bundler, "pack", &pack_config, &loaded.root)?;
    if let Some(runtime) = &fixed_webview {
        stage_fixed_webview_runtime(&loaded.root, &output, runtime)?;
    }
    // Sign AFTER packing: the signature must cover the final bytes, pack
    // included. Authenticode appends its certificate table after the pack, so
    // `tynd_pack_format::PackReader` locates the trailer through the PE security
    // directory (and `LC_CODE_SIGNATURE` on macOS) instead of assuming EOF.
    // macOS follows the same order, then checks the pack is still readable.
    sign_windows_artifacts(&loaded, std::slice::from_ref(&output))?;
    sign_macos_artifacts(&bundler, &loaded, std::slice::from_ref(&output))?;
    success(format!("built {}", display_path(&output)));

    finish_release_artifacts(
        &bundler,
        &loaded,
        &package,
        &output,
        requested_targets,
        &app_name,
        &version,
    )?;
    Ok(())
}

fn finish_release_artifacts(
    bundler: &Path,
    loaded: &LoadedConfig,
    package: &crate::project::PackageJson,
    output: &Path,
    requested_targets: Option<Vec<String>>,
    app_name: &str,
    version: &str,
) -> Result<()> {
    let phased_macos_dmg = requested_targets.as_ref().is_some_and(|targets| {
        cfg!(target_os = "macos")
            && targets.iter().any(|target| target == "dmg")
            && loaded
                .config
                .bundle
                .as_ref()
                .and_then(|bundle| bundle.sign.as_ref())
                .and_then(|signing| signing.macos.as_ref())
                .is_some()
    });
    let mut artifacts = Vec::new();
    if let Some(mut targets) = requested_targets {
        if phased_macos_dmg {
            targets.retain(|target| target != "dmg");
            if !targets.iter().any(|target| target == "macos") {
                targets.push("macos".to_owned());
            }
        }
        let bundle_config = native_bundle_config(loaded, package, output, targets)?;
        artifacts = invoke_bundler(bundler, "bundle", &bundle_config, &loaded.root)?;
        sign_windows_artifacts(loaded, &artifacts)?;
    }
    // The packed executable itself was signed right after packing.
    sign_macos_artifacts(bundler, loaded, &artifacts)?;
    archive_macos_apps(bundler, loaded, app_name, version, &artifacts)?;
    if phased_macos_dmg {
        let dmgs = create_macos_dmgs(bundler, loaded, app_name, version, &artifacts)?;
        sign_macos_artifacts(bundler, loaded, &dmgs)?;
    }

    Ok(())
}

fn prepare_build_output(
    root: &Path,
    requested: Option<&Path>,
    release_dir: &Path,
    app_name: &str,
) -> Result<PathBuf> {
    let output = match requested {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => {
            ensure!(
                path.components()
                    .all(|component| matches!(component, std::path::Component::Normal(_))),
                "--out must not contain dot or parent segments"
            );
            fs::canonicalize(root)?.join(path)
        },
        None => release_dir.join(platform_app_binary(app_name)),
    };
    ensure!(output.file_name().is_some(), "--out must name a file");
    let canonical_root = fs::canonicalize(root)?;
    if output.starts_with(&canonical_root) {
        ensure_existing_path_is_unlinked(&canonical_root, &output)?;
    }
    let parent = output.parent().context("build output has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    if let Ok(metadata) = fs::symlink_metadata(&output) {
        ensure!(
            !is_link_or_reparse(&metadata) && metadata.is_file(),
            "refusing unsafe build output {}",
            output.display()
        );
    }
    Ok(output)
}

fn build_backend(loaded: &LoadedConfig) -> Result<PathBuf> {
    let entry = resolve_backend_entry(loaded)?;
    let cache = generated_path(&loaded.root, ".tynd/cache")?;
    fs::create_dir_all(&cache)?;
    let output = cache.join("bundle.dist.js");
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let bun = resolve_bun_runtime(&loaded.root)?;
    let command = CommandSpec::new(bun.path.as_os_str())
        .args([
            OsString::from("build"),
            entry.as_os_str().to_os_string(),
            OsString::from("--target=bun"),
            OsString::from("--outfile"),
            output.as_os_str().to_os_string(),
        ])
        .cwd(&loaded.root)
        .arg("--minify");

    ensure_success(
        process::run_streaming(with_project_environment(command, loaded))?,
        "backend build",
    )?;
    Ok(output)
}

fn build_frontend_assets(
    loaded: &LoadedConfig,
    frontend: &crate::project::FrontendInfo,
) -> Result<PathBuf> {
    let configured = &loaded.config.frontend_dir;
    let built = if configured.is_dir() {
        configured.clone()
    } else {
        let detected = loaded.root.join(&frontend.out_dir);
        ensure!(
            detected.is_dir(),
            "frontend build output not found: {}",
            detected.display()
        );
        detected
    };

    if frontend.build_tool == crate::project::BuildTool::None {
        let output = generated_path(&loaded.root, ".tynd/cache/frontend")?;
        replace_generated_directory(&loaded.root, &output)?;
        copy_tree_strict(&built, &output)?;
        inject_frontend_runtime_config(&output, &loaded.config)?;
        Ok(output)
    } else {
        inject_frontend_runtime_config(&built, &loaded.config)?;
        Ok(built)
    }
}

fn inject_frontend_runtime_config(frontend_dir: &Path, config: &TyndConfig) -> Result<()> {
    // Renderer-visible: only the public updater config. `updater.proxy` /
    // `updater.headers` (credentials) reach the host solely through the backend
    // bootstrap (`resolve_backend_entry`), which `dev`, `start` and `build` share.
    let updater = config::resolve_updater_config(config)?;
    let updater = updater
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?
        .unwrap_or_else(|| "null".to_owned())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let marker = "<script id=\"tynd-runtime-config\">";
    let script = format!(
        "{marker}globalThis.__TYND_RUNTIME__=\"bun\";globalThis.__TYND_UPDATER_CONFIG__={updater};</script>"
    );
    for entry in walkdir::WalkDir::new(frontend_dir).follow_links(false) {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(
            !is_link_or_reparse(&metadata),
            "refusing linked frontend asset {}",
            entry.path().display()
        );
        if !entry.file_type().is_file()
            || !entry
                .path()
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("html"))
        {
            continue;
        }
        let source = fs::read_to_string(entry.path())?;
        let next = if let Some(start) = source.find(marker) {
            let close = source[start..]
                .find("</script>")
                .map(|offset| start + offset + "</script>".len())
                .context("generated Tynd runtime config script is not closed")?;
            let mut next = source.clone();
            next.replace_range(start..close, &script);
            next
        } else if let Some(index) = source.to_ascii_lowercase().find("</head>") {
            let mut next = source.clone();
            next.insert_str(index, &script);
            next
        } else {
            format!("{script}{source}")
        };
        atomic_write(entry.path(), next)?;
    }
    Ok(())
}

fn native_bundle_config(
    loaded: &LoadedConfig,
    package: &crate::project::PackageJson,
    binary: &Path,
    targets: Vec<String>,
) -> Result<contract::BundleConfig> {
    let bundle = loaded
        .config
        .bundle
        .as_ref()
        .context("installer bundles require bundle.identifier in tynd.config")?;
    let author = crate::project::normalize_author(package.author.as_ref());
    let name = bundle
        .display_name
        .clone()
        .or(package.name.clone())
        .unwrap_or_else(|| project_name(&loaded.root));
    let version = package.version.clone().unwrap_or_else(|| "0.0.0".into());
    let webview_mode = bundle
        .windows
        .as_ref()
        .and_then(|windows| windows.webview_install_mode.as_ref());
    let resources = bundle_resources(&loaded.root, &bundle.resources, webview_mode)?;
    let file_associations = bundle
        .file_associations
        .iter()
        .map(normalize_file_association)
        .collect();
    let windows = windows_bundle_section(bundle.windows.as_ref());
    let project_file = |value: Option<&String>| {
        value
            .map(|path| resolve_bundle_resource(&loaded.root, path))
            .transpose()
    };
    let linux = linux_bundle_section(bundle, &project_file)?;
    let macos = bundle.macos.as_ref();
    let schemes = bundle
        .macos
        .as_ref()
        .and_then(|macos| macos.url_schemes.clone())
        .unwrap_or_else(|| loaded.config.protocols.clone());
    Ok(contract::BundleConfig {
        name,
        version,
        identifier: bundle.identifier.clone(),
        description: bundle.short_description.clone(),
        long_description: bundle.long_description.clone(),
        homepage: package.homepage.clone(),
        authors: author.map(|value| vec![value.name]),
        license: package
            .extra
            .get("license")
            .and_then(Value::as_str)
            .map(str::to_owned),
        copyright: bundle.copyright.clone(),
        category: bundle.categories.first().cloned(),
        license_file: project_file(bundle.license_file.as_ref())?,
        targets,
        // WiX (candle/light) cannot open verbatim `\\?\` paths: hand the
        // bundler plain paths wherever they are representable.
        project_root: dunce::simplified(&loaded.root).to_path_buf(),
        out_dir: dunce::simplified(
            binary
                .parent()
                .context("bundle binary has no output directory")?,
        )
        .to_path_buf(),
        binary_path: dunce::simplified(binary).to_path_buf(),
        resources,
        file_associations,
        icons: loaded.config.icon.iter().cloned().collect(),
        linux,
        macos: contract::MacOsConfig {
            url_schemes: schemes,
            minimum_system_version: macos.and_then(|macos| macos.minimum_system_version.clone()),
            entitlements: project_file(macos.and_then(|macos| macos.entitlements.as_ref()))?,
            info_plist: project_file(macos.and_then(|macos| macos.info_plist.as_ref()))?,
        },
        windows,
        custom_nsis_template: None,
        custom_wix_template: None,
    })
}

/// Resolves the configured bundle resources; on Windows a fixed WebView2
/// runtime directory is shipped as an extra resource.
fn bundle_resources(
    root: &Path,
    configured: &[String],
    webview_mode: Option<&WebView2InstallMode>,
) -> Result<Vec<PathBuf>> {
    let mut resources = configured
        .iter()
        .map(|path| resolve_bundle_resource(root, path))
        .collect::<Result<Vec<_>>>()?;
    if let Some(WebView2InstallMode::FixedRuntime { path }) =
        cfg!(windows).then_some(webview_mode).flatten()
    {
        let path = dunce::canonicalize(root.join(path))?;
        if !resources.contains(&path) {
            resources.push(path);
        }
    }
    Ok(resources)
}

fn windows_bundle_section(windows: Option<&WindowsBundleConfig>) -> contract::WindowsConfig {
    contract::WindowsConfig {
        webview_install_mode: windows
            .and_then(|windows| windows.webview_install_mode.as_ref())
            .map(webview_install_mode),
        minimum_webview2_version: windows
            .and_then(|windows| windows.minimum_web_view2_version.clone()),
        allow_downgrades: windows.and_then(|windows| windows.allow_downgrades),
        install_mode: windows
            .and_then(|windows| windows.install_mode)
            .map(|mode| match mode {
                NsisInstallMode::CurrentUser => contract::NsisInstallMode::CurrentUser,
                NsisInstallMode::PerMachine => contract::NsisInstallMode::PerMachine,
                NsisInstallMode::Both => contract::NsisInstallMode::Both,
            }),
        msix: windows
            .and_then(|windows| windows.msix.as_ref())
            .map(|msix| contract::MsixConfig {
                publisher: msix.publisher.clone(),
                publisher_display_name: msix.publisher_display_name.clone(),
                min_version: msix.min_version.clone(),
                version: msix.version.clone(),
            }),
        requested_execution_level: windows
            .and_then(|windows| windows.requested_execution_level)
            .map(execution_level),
    }
}

const fn execution_level(level: ExecutionLevel) -> contract::ExecutionLevel {
    match level {
        ExecutionLevel::AsInvoker => contract::ExecutionLevel::AsInvoker,
        ExecutionLevel::HighestAvailable => contract::ExecutionLevel::HighestAvailable,
        ExecutionLevel::RequireAdministrator => contract::ExecutionLevel::RequireAdministrator,
    }
}

/// An unset `silent` means a silent install, as in the config schema.
fn webview_install_mode(mode: &WebView2InstallMode) -> contract::WebviewInstallMode {
    match mode {
        WebView2InstallMode::Skip => contract::WebviewInstallMode::Skip,
        WebView2InstallMode::DownloadBootstrapper { silent } => {
            contract::WebviewInstallMode::DownloadBootstrapper {
                silent: silent.unwrap_or(true),
            }
        },
        WebView2InstallMode::EmbedBootstrapper { silent } => {
            contract::WebviewInstallMode::EmbedBootstrapper {
                silent: silent.unwrap_or(true),
            }
        },
        WebView2InstallMode::OfflineInstaller { silent } => {
            contract::WebviewInstallMode::OfflineInstaller {
                silent: silent.unwrap_or(true),
            }
        },
        WebView2InstallMode::FixedRuntime { path } => contract::WebviewInstallMode::FixedRuntime {
            path: PathBuf::from(path),
        },
    }
}

fn normalize_file_association(association: &FileAssociation) -> contract::FileAssociation {
    contract::FileAssociation {
        ext: association
            .ext
            .iter()
            .map(|extension| extension.trim_start_matches('.').to_owned())
            .collect(),
        content_types: association.content_types.clone(),
        name: association.name.clone(),
        description: association.description.clone(),
        role: association
            .role
            .map_or_else(Default::default, |role| match role {
                FileAssociationRole::Editor => contract::FileAssociationRole::Editor,
                FileAssociationRole::Viewer => contract::FileAssociationRole::Viewer,
                FileAssociationRole::Shell => contract::FileAssociationRole::Shell,
                FileAssociationRole::QlGenerator => contract::FileAssociationRole::QlGenerator,
                FileAssociationRole::None => contract::FileAssociationRole::None,
            }),
        mime_type: association.mime_type.clone(),
        rank: association
            .rank
            .map_or_else(Default::default, |rank| match rank {
                FileAssociationRank::Default => contract::FileAssociationRank::Default,
                FileAssociationRank::Owner => contract::FileAssociationRank::Owner,
                FileAssociationRank::Alternate => contract::FileAssociationRank::Alternate,
                FileAssociationRank::None => contract::FileAssociationRank::None,
            }),
        exported_type: association.exported_type.as_ref().map(|exported| {
            contract::ExportedFileType {
                identifier: exported.identifier.clone(),
                conforms_to: exported.conforms_to.clone(),
            }
        }),
    }
}

fn linux_bundle_section(
    bundle: &BundleConfig,
    project_file: &impl Fn(Option<&String>) -> Result<Option<PathBuf>>,
) -> Result<contract::LinuxConfig> {
    let mut deb = normalize_deb(bundle.deb.as_ref());
    deb.desktop_template = project_file(
        bundle
            .deb
            .as_ref()
            .and_then(|deb| deb.desktop_template.as_ref()),
    )?;
    let mut rpm = normalize_rpm(bundle.rpm.as_ref());
    rpm.desktop_template = project_file(
        bundle
            .rpm
            .as_ref()
            .and_then(|rpm| rpm.desktop_template.as_ref()),
    )?;
    Ok(contract::LinuxConfig {
        deb,
        rpm,
        appimage: contract::AppImageConfig {
            bundle_xdg_open: bundle
                .appimage
                .as_ref()
                .and_then(|appimage| appimage.bundle_xdg_open)
                .unwrap_or(false),
        },
        pkgbuild: contract::PkgbuildConfig {
            depends: bundle
                .pkgbuild
                .as_ref()
                .and_then(|pkgbuild| pkgbuild.depends.clone())
                .unwrap_or_default(),
        },
    })
}

fn normalize_deb(value: Option<&DebBundleConfig>) -> contract::DebConfig {
    contract::DebConfig {
        depends: value
            .and_then(|value| value.depends.clone())
            .unwrap_or_default(),
        priority: value.and_then(|value| value.priority.clone()),
        section: value.and_then(|value| value.section.clone()),
        ..contract::DebConfig::default()
    }
}

fn normalize_rpm(value: Option<&RpmBundleConfig>) -> contract::RpmConfig {
    contract::RpmConfig {
        depends: value
            .and_then(|value| value.requires.clone())
            .unwrap_or_default(),
        ..contract::RpmConfig::default()
    }
}

fn resolve_bundle_resource(root: &Path, configured: &str) -> Result<PathBuf> {
    let configured = Path::new(configured);
    let path = if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        ensure!(
            configured
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
            "bundle resource contains an unsafe path: {}",
            configured.display()
        );
        root.join(configured)
    };
    ensure!(
        path.exists(),
        "bundle resource not found: {}",
        path.display()
    );
    // Compare verbatim forms (dunce keeps `\\?\` for long paths, so mixing
    // could reject an in-project resource); simplify only the returned text.
    let canonical = fs::canonicalize(&path)?;
    let root = fs::canonicalize(root)?;
    ensure!(
        configured.is_absolute() || canonical.starts_with(&root),
        "bundle resource escapes project root: {}",
        path.display()
    );
    Ok(dunce::simplified(&canonical).to_path_buf())
}

fn invoke_bundler(
    bundler: &Path,
    command: &str,
    config: &impl Serialize,
    root: &Path,
) -> Result<Vec<PathBuf>> {
    let cache = generated_path(root, ".tynd/cache")?;
    fs::create_dir_all(&cache)?;
    let path = cache.join(format!("{command}.json"));
    write_json_file(&path, &serde_json::to_value(config)?)?;
    let output = process::run_spec(
        CommandSpec::new(bundler.as_os_str())
            .args([OsString::from(command), path.as_os_str().to_os_string()])
            .cwd(root),
    )?;
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    // stdout lists artifact paths, one per line: echo them readable, but keep
    // the canonical form below for the paths the next steps operate on.
    for line in output.stdout.lines() {
        write_line(format_args!("{}", display_path(Path::new(line))))?;
    }
    Ok(output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        })
        .filter(|path| path.exists())
        .collect())
}

fn run_plan(plan: &crate::project::CommandPlan, loaded: &LoadedConfig) -> Result<()> {
    ensure_success(
        process::run_streaming(with_project_environment(
            CommandSpec::new(&plan.program)
                .args(plan.args.clone())
                .cwd(&loaded.root),
            loaded,
        ))?,
        &plan.to_string(),
    )
}

fn bundle_targets(value: &str) -> Result<Vec<String>> {
    let requested = if value.eq_ignore_ascii_case("all") {
        if cfg!(windows) {
            vec!["msi".to_owned(), "nsis".to_owned()]
        } else if cfg!(target_os = "macos") {
            vec!["macos".to_owned(), "dmg".to_owned()]
        } else {
            vec!["deb".to_owned(), "rpm".to_owned(), "appimage".to_owned()]
        }
    } else {
        value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                if value.eq_ignore_ascii_case("app") {
                    "macos".to_owned()
                } else {
                    value.to_ascii_lowercase()
                }
            })
            .collect()
    };
    ensure!(
        !requested.is_empty(),
        "--bundle requires all or a comma-separated target list"
    );
    let supported: &[&str] = if cfg!(windows) {
        &["msi", "nsis", "msix"]
    } else if cfg!(target_os = "macos") {
        &["macos", "dmg"]
    } else {
        &["deb", "rpm", "appimage", "flatpak", "snap", "pkgbuild"]
    };
    let mut unique = BTreeSet::new();
    for target in requested {
        ensure!(
            supported.contains(&target.as_str()),
            "bundle target {target:?} is unavailable on {}; supported targets: {}",
            std::env::consts::OS,
            supported.join(", ")
        );
        ensure!(
            unique.insert(target.clone()),
            "duplicate bundle target {target:?}"
        );
    }
    Ok(unique.into_iter().collect())
}

pub(super) fn platform_app_binary(name: &str) -> String {
    let safe = contract::safe_file_stem(name);
    if cfg!(windows) {
        format!("{safe}.exe")
    } else {
        safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_specific_package_targets_are_accepted_only_on_their_os() {
        if cfg!(windows) {
            assert_eq!(bundle_targets("msix,nsis").unwrap(), ["msix", "nsis"]);
            assert!(bundle_targets("flatpak").is_err());
        } else if cfg!(target_os = "linux") {
            assert_eq!(
                bundle_targets("snap,flatpak,pkgbuild").unwrap(),
                ["flatpak", "pkgbuild", "snap"]
            );
            assert!(bundle_targets("msix").is_err());
        }
    }

    #[test]
    fn execution_levels_map_to_the_contract() {
        assert_eq!(
            execution_level(ExecutionLevel::RequireAdministrator),
            contract::ExecutionLevel::RequireAdministrator
        );
        assert_eq!(
            execution_level(ExecutionLevel::AsInvoker),
            contract::ExecutionLevel::AsInvoker
        );
    }
}

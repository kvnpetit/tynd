//! Flatpak, Snap and Arch Linux (PKGBUILD) targets. tauri-bundler has none of
//! them, so each target stages the packed binary, a `.desktop` entry, an
//! AppStream metainfo file and the hicolor icons, writes the build manifest,
//! then builds it when the tool (`flatpak-builder`, `snapcraft`, `makepkg`) is
//! on `PATH`. Without the tool the manifest is the artifact and a log line says
//! how to build it.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr as _,
};

use anyhow::{bail, ensure, Context, Result};
use tauri_bundler::AppCategory;
use tracing::{info, warn};
use tynd_bundle_contract::BundleConfig;

use crate::{icon_set, util};

/// Flatpak runtime providing GTK 3 and WebKitGTK 4.1 (what wry links).
const FLATPAK_RUNTIME_VERSION: &str = "48";
/// Snap base matching the `gnome` extension (GTK 3 + WebKitGTK 4.1).
const SNAP_BASE: &str = "core24";

/// A Linux package target handled here, not by tauri-bundler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxPackage {
    Flatpak,
    Snap,
    Pkgbuild,
}

impl LinuxPackage {
    pub(crate) fn parse(target: &str) -> Option<Self> {
        match target {
            "flatpak" => Some(Self::Flatpak),
            "snap" => Some(Self::Snap),
            "pkgbuild" | "arch" => Some(Self::Pkgbuild),
            _ => None,
        }
    }

    const fn dir_name(self) -> &'static str {
        match self {
            Self::Flatpak => "flatpak",
            Self::Snap => "snap",
            Self::Pkgbuild => "pkgbuild",
        }
    }
}

/// Facts shared by every manifest, derived once from the bundle request.
#[derive(Debug)]
struct App<'a> {
    cfg: &'a BundleConfig,
    /// File name of the packed binary.
    binary: String,
    /// Lowercase package name (`[a-z0-9-]`), used by Snap and Arch.
    package: String,
    /// `x86_64` / `aarch64`.
    arch: String,
}

impl<'a> App<'a> {
    fn new(cfg: &'a BundleConfig) -> Result<Self> {
        let binary = cfg
            .binary_path
            .file_name()
            .and_then(|name| name.to_str())
            .context("the binary path has no UTF-8 file name")?
            .to_owned();
        let arch = util::detect_binary_arch(&util::to_utf8_path(&cfg.binary_path)?)?;
        ensure!(
            matches!(arch.as_str(), "x86_64" | "aarch64"),
            "Linux packages need an x86_64 or aarch64 ELF binary; {} is {arch}",
            cfg.binary_path.display()
        );
        Ok(Self {
            cfg,
            binary,
            package: package_name(&cfg.name),
            arch,
        })
    }

    fn summary(&self) -> String {
        let text = self.cfg.description.as_deref().unwrap_or(&self.cfg.name);
        single_line(text).chars().take(78).collect()
    }
}

/// Lowercase `[a-z0-9-]` name, no leading/trailing/double `-`, at most 40
/// characters (the Snap Store limit; Arch accepts it too).
pub(crate) fn package_name(name: &str) -> String {
    let mut out = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    if out.is_empty() {
        return "app".to_owned();
    }
    if !out.starts_with(|c: char| c.is_ascii_lowercase()) {
        out.insert_str(0, "app-");
    }
    let mut out: String = out.chars().take(40).collect();
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `.desktop` free-text value: whitespace runs (line breaks included)
/// collapse to one space, other control characters are refused, `\` is
/// escaped as the Desktop Entry spec requires.
fn desktop_value(field: &str, text: &str) -> Result<String> {
    let text = single_line(text);
    ensure!(
        !text.chars().any(char::is_control),
        "the .desktop {field} value {text:?} contains a control character"
    );
    Ok(text.replace('\\', "\\\\"))
}

/// `.desktop` token (icon name, URL scheme, MIME type): no whitespace,
/// control character or `;` (the list separator), so it cannot start a new
/// line or list item.
fn desktop_token<'t>(field: &str, text: &'t str) -> Result<&'t str> {
    ensure!(
        !text.is_empty()
            && !text.chars().any(|character| character.is_control()
                || character.is_whitespace()
                || character == ';'),
        "the .desktop {field} value {text:?} must be non-empty, without whitespace, control \
         characters or `;`"
    );
    Ok(text)
}

/// `Exec` program: quoted per the Desktop Entry spec when it holds a reserved
/// character, `%` doubled, then string-escaped.
fn desktop_exec(program: &str) -> Result<String> {
    ensure!(
        !program.is_empty() && !program.chars().any(char::is_control),
        "the .desktop Exec program {program:?} must be non-empty without control characters"
    );
    let program = program.replace('%', "%%");
    let quoted = if program.contains(|character: char| {
        character.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(character)
    }) {
        let mut out = String::from("\"");
        for character in program.chars() {
            if matches!(character, '"' | '`' | '$' | '\\') {
                out.push('\\');
            }
            out.push(character);
        }
        out.push('"');
        out
    } else {
        program
    };
    Ok(quoted.replace('\\', "\\\\"))
}

pub(crate) fn desktop_entry(cfg: &BundleConfig, exec: &str, icon: &str) -> Result<String> {
    let categories = cfg
        .category
        .as_deref()
        .and_then(|category| AppCategory::from_str(category).ok())
        .map_or("Utility;", AppCategory::freedesktop_categories);
    let mut mime_types = cfg
        .macos
        .url_schemes
        .iter()
        .map(|scheme| {
            desktop_token("URL scheme", scheme).map(|scheme| format!("x-scheme-handler/{scheme}"))
        })
        .collect::<Result<Vec<_>>>()?;
    for association in &cfg.file_associations {
        if let Some(mime_type) = &association.mime_type {
            mime_types.push(desktop_token("MimeType", mime_type)?.to_owned());
        }
    }
    let mut entry = String::from("[Desktop Entry]\nType=Application\n");
    let _ = writeln!(entry, "Name={}", desktop_value("Name", &cfg.name)?);
    if let Some(description) = &cfg.description {
        let _ = writeln!(entry, "Comment={}", desktop_value("Comment", description)?);
    }
    let _ = writeln!(entry, "Exec={} %U", desktop_exec(exec)?);
    let _ = writeln!(entry, "Icon={}", desktop_token("Icon", icon)?);
    let _ = writeln!(entry, "Terminal=false");
    let _ = writeln!(entry, "Categories={categories}");
    let _ = writeln!(
        entry,
        "StartupWMClass={}",
        desktop_value("StartupWMClass", exec)?
    );
    if !mime_types.is_empty() {
        let _ = writeln!(entry, "MimeType={};", mime_types.join(";"));
    }
    Ok(entry)
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn metainfo(cfg: &BundleConfig, summary: &str) -> String {
    let description = cfg
        .long_description
        .as_deref()
        .or(cfg.description.as_deref())
        .unwrap_or(&cfg.name);
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<component type=\"desktop-application\">\n",
    );
    let _ = writeln!(xml, "  <id>{}</id>", xml_escape(&cfg.identifier));
    let _ = writeln!(xml, "  <name>{}</name>", xml_escape(&cfg.name));
    let _ = writeln!(xml, "  <summary>{}</summary>", xml_escape(summary));
    let _ = writeln!(xml, "  <metadata_license>CC0-1.0</metadata_license>");
    if let Some(license) = &cfg.license {
        let _ = writeln!(
            xml,
            "  <project_license>{}</project_license>",
            xml_escape(license)
        );
    }
    let _ = writeln!(
        xml,
        "  <description>\n    <p>{}</p>\n  </description>",
        xml_escape(&single_line(description))
    );
    let _ = writeln!(
        xml,
        "  <launchable type=\"desktop-id\">{}.desktop</launchable>",
        xml_escape(&cfg.identifier)
    );
    if let Some(homepage) = &cfg.homepage {
        let _ = writeln!(
            xml,
            "  <url type=\"homepage\">{}</url>",
            xml_escape(homepage)
        );
    }
    let _ = writeln!(
        xml,
        "  <releases>\n    <release version=\"{}\"/>\n  </releases>",
        xml_escape(&cfg.version)
    );
    xml.push_str("</component>\n");
    xml
}

/// YAML double-quoted scalar. JSON string syntax is valid YAML.
fn yaml(text: &str) -> String {
    serde_json::Value::String(text.to_owned()).to_string()
}

fn flatpak_manifest(app: &App<'_>) -> String {
    let id = &app.cfg.identifier;
    let mut commands = vec![
        format!("install -Dm755 {} /app/bin/{}", app.binary, app.binary),
        format!("install -Dm644 {id}.desktop /app/share/applications/{id}.desktop"),
        format!("install -Dm644 {id}.metainfo.xml /app/share/metainfo/{id}.metainfo.xml"),
    ];
    commands.extend(icon_set::HICOLOR_SIZES.iter().map(|size| {
        format!(
            "install -Dm644 icons/{size}x{size}.png /app/share/icons/hicolor/{size}x{size}/apps/{id}.png"
        )
    }));
    let mut yaml_text = String::new();
    let _ = writeln!(yaml_text, "app-id: {}", yaml(id));
    let _ = writeln!(yaml_text, "runtime: org.gnome.Platform");
    let _ = writeln!(
        yaml_text,
        "runtime-version: {}",
        yaml(FLATPAK_RUNTIME_VERSION)
    );
    let _ = writeln!(yaml_text, "sdk: org.gnome.Sdk");
    let _ = writeln!(yaml_text, "command: {}", yaml(&app.binary));
    yaml_text.push_str(
        "finish-args:\n  - --share=ipc\n  - --share=network\n  - --socket=wayland\n  - \
         --socket=fallback-x11\n  - --device=dri\n  - --talk-name=org.freedesktop.Notifications\n",
    );
    yaml_text.push_str("modules:\n");
    let _ = writeln!(yaml_text, "  - name: {}", yaml(&app.package));
    yaml_text.push_str("    buildsystem: simple\n    build-commands:\n");
    for command in commands {
        let _ = writeln!(yaml_text, "      - {}", yaml(&command));
    }
    yaml_text.push_str("    sources:\n");
    for file in [
        app.binary.clone(),
        format!("{id}.desktop"),
        format!("{id}.metainfo.xml"),
    ] {
        let _ = writeln!(
            yaml_text,
            "      - type: file\n        path: {}",
            yaml(&file)
        );
    }
    yaml_text.push_str("      - type: dir\n        path: icons\n        dest: icons\n");
    yaml_text
}

fn snapcraft_yaml(app: &App<'_>) -> String {
    let id = &app.cfg.identifier;
    let description = app
        .cfg
        .long_description
        .as_deref()
        .or(app.cfg.description.as_deref())
        .unwrap_or(&app.cfg.name);
    let mut text = String::new();
    let _ = writeln!(text, "name: {}", app.package);
    let _ = writeln!(text, "base: {SNAP_BASE}");
    let _ = writeln!(text, "version: {}", yaml(&app.cfg.version));
    let _ = writeln!(text, "summary: {}", yaml(&app.summary()));
    let _ = writeln!(text, "description: {}", yaml(&single_line(description)));
    if let Some(license) = &app.cfg.license {
        let _ = writeln!(text, "license: {}", yaml(license));
    }
    // Store listing icon: the 512 px PNG staged next to `snap/`.
    text.push_str("icon: icons/512x512.png\n");
    text.push_str("grade: stable\nconfinement: strict\n");
    let _ = writeln!(
        text,
        "platforms:\n  {}:\n",
        if app.arch == "aarch64" {
            "arm64"
        } else {
            "amd64"
        }
    );
    let _ = writeln!(text, "apps:\n  {}:", app.package);
    let _ = writeln!(text, "    command: bin/{}", app.binary);
    let _ = writeln!(text, "    desktop: share/applications/{id}.desktop");
    text.push_str(
        "    extensions: [gnome]\n    plugs: [network, home, desktop, wayland, x11, opengl]\n",
    );
    let _ = writeln!(text, "parts:\n  {}:", app.package);
    text.push_str("    plugin: dump\n    source: .\n    organize:\n");
    let _ = writeln!(text, "      {}: bin/{}", yaml(&app.binary), app.binary);
    let _ = writeln!(
        text,
        "      {}: share/applications/{id}.desktop",
        yaml(&format!("{id}.desktop"))
    );
    let _ = writeln!(
        text,
        "      {}: share/metainfo/{id}.metainfo.xml",
        yaml(&format!("{id}.metainfo.xml"))
    );
    for size in icon_set::HICOLOR_SIZES {
        let _ = writeln!(
            text,
            "      icons/{size}x{size}.png: share/icons/hicolor/{size}x{size}/apps/{id}.png"
        );
    }
    text.push_str("    stage:\n      - bin\n      - share\n");
    text
}

/// Bash double-quoted string body: `\`, `"`, `$` and backtick escaped.
fn bash_quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in single_line(text).chars() {
        if matches!(character, '\\' | '"' | '$' | '`') {
            out.push('\\');
        }
        out.push(character);
    }
    out.push('"');
    out
}

/// Arch `pkgver` may not contain `-`, `:` or whitespace.
fn arch_pkgver(version: &str) -> String {
    version
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '+') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn pkgbuild(app: &App<'_>) -> String {
    let id = &app.cfg.identifier;
    let mut sources = vec![
        app.binary.clone(),
        format!("{id}.desktop"),
        format!("{id}.metainfo.xml"),
    ];
    sources.extend(
        icon_set::HICOLOR_SIZES
            .iter()
            .map(|size| format!("icons-{size}x{size}.png")),
    );
    let mut text = String::from("# Generated by tynd build --bundle pkgbuild.\n");
    let _ = writeln!(text, "pkgname={}", app.package);
    let _ = writeln!(text, "pkgver={}", arch_pkgver(&app.cfg.version));
    text.push_str("pkgrel=1\n");
    let _ = writeln!(text, "pkgdesc={}", bash_quoted(&app.summary()));
    let _ = writeln!(text, "arch=('{}')", app.arch);
    if let Some(homepage) = &app.cfg.homepage {
        let _ = writeln!(text, "url={}", bash_quoted(homepage));
    }
    let _ = writeln!(
        text,
        "license=({})",
        bash_quoted(app.cfg.license.as_deref().unwrap_or("custom"))
    );
    // Arch package names, not the Fedora ones of `bundle.rpm.requires`.
    let mut depends = vec!["gtk3".to_owned(), "webkit2gtk-4.1".to_owned()];
    for depend in &app.cfg.linux.pkgbuild.depends {
        if !depends.contains(depend) {
            depends.push(depend.clone());
        }
    }
    let depends: Vec<String> = depends.iter().map(|value| bash_quoted(value)).collect();
    let _ = writeln!(text, "depends=({})", depends.join(" "));
    // `tray-icon` loads the AppIndicator library at runtime.
    text.push_str("optdepends=('libayatana-appindicator: system tray icon')\n");
    text.push_str("options=('!strip' '!debug')\n");
    let quoted: Vec<String> = sources.iter().map(|source| bash_quoted(source)).collect();
    let _ = writeln!(text, "source=({})", quoted.join(" "));
    let skips = vec!["'SKIP'"; sources.len()].join(" ");
    let _ = writeln!(text, "sha256sums=({skips})");
    text.push_str("\npackage() {\n");
    let _ = writeln!(
        text,
        "  install -Dm755 \"$srcdir/{0}\" \"$pkgdir/usr/bin/{0}\"",
        app.binary
    );
    let _ = writeln!(
        text,
        "  install -Dm644 \"$srcdir/{id}.desktop\" \"$pkgdir/usr/share/applications/{id}.desktop\""
    );
    let _ = writeln!(
        text,
        "  install -Dm644 \"$srcdir/{id}.metainfo.xml\" \"$pkgdir/usr/share/metainfo/{id}.metainfo.xml\""
    );
    for size in icon_set::HICOLOR_SIZES {
        let _ = writeln!(
            text,
            "  install -Dm644 \"$srcdir/icons-{size}x{size}.png\" \"$pkgdir/usr/share/icons/hicolor/{size}x{size}/apps/{id}.png\""
        );
    }
    if app.cfg.license_file.is_some() {
        let _ = writeln!(
            text,
            "  install -Dm644 \"$srcdir/LICENSE\" \"$pkgdir/usr/share/licenses/{}/LICENSE\"",
            app.package
        );
    }
    text.push_str("}\n");
    if app.cfg.license_file.is_some() {
        text = text
            .replacen("source=(", "source=(\"LICENSE\" ", 1)
            .replacen("sha256sums=(", "sha256sums=('SKIP' ", 1);
    }
    text
}

/// Copy the binary, desktop entry, metainfo and icons into `dir`.
fn stage(app: &App<'_>, dir: &Path, icons: &Path, exec: &str) -> Result<()> {
    fs::create_dir_all(dir.join("icons")).with_context(|| format!("create {}", dir.display()))?;
    let id = &app.cfg.identifier;
    fs::copy(&app.cfg.binary_path, dir.join(&app.binary))
        .with_context(|| format!("copy {}", app.cfg.binary_path.display()))?;
    fs::write(
        dir.join(format!("{id}.desktop")),
        desktop_entry(app.cfg, exec, id)?,
    )?;
    fs::write(
        dir.join(format!("{id}.metainfo.xml")),
        metainfo(app.cfg, &app.summary()),
    )?;
    for size in icon_set::HICOLOR_SIZES {
        let name = format!("{size}x{size}.png");
        fs::copy(icons.join("png").join(&name), dir.join("icons").join(&name))
            .with_context(|| format!("stage icon {name}"))?;
    }
    if let Some(license) = &app.cfg.license_file {
        fs::copy(license, dir.join("LICENSE"))
            .with_context(|| format!("copy license file {}", license.display()))?;
    }
    Ok(())
}

/// Render the icon set once for every Linux package target.
fn render_icons(cfg: &BundleConfig, icons: &[PathBuf], work: &Path) -> Result<PathBuf> {
    let out = work.join("icon-set");
    let source = icons
        .iter()
        .find(|icon| {
            icon.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("svg") || extension.eq_ignore_ascii_case("png")
                })
        })
        .map(|icon| {
            if icon.is_absolute() {
                icon.clone()
            } else {
                cfg.project_root.join(icon)
            }
        })
        .context("Flatpak, Snap and PKGBUILD packages need a PNG or SVG `icon`")?;
    icon_set::generate(&source, &out)?;
    Ok(out)
}

fn find_tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn run_tool(tool: &Path, args: &[&str], cwd: &Path) -> Result<()> {
    info!("running {} {}", tool.display(), args.join(" "));
    let status = Command::new(tool)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("start {}", tool.display()))?;
    if !status.success() {
        bail!("{} {} failed with {status}", tool.display(), args.join(" "));
    }
    Ok(())
}

/// Build the requested Linux package targets; returns the artifact paths
/// (packages, or manifests when the build tool is missing).
pub(crate) fn build(
    cfg: &BundleConfig,
    targets: &[LinuxPackage],
    icons: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    ensure!(
        cfg!(target_os = "linux"),
        "the flatpak, snap and pkgbuild targets build on Linux only"
    );
    let app = App::new(cfg)?;
    let work = cfg.out_dir.join("linux-packages");
    let icon_dir = render_icons(cfg, icons, &work)?;
    let mut artifacts = Vec::new();
    for &target in targets {
        let dir = work.join(target.dir_name());
        if dir.exists() {
            fs::remove_dir_all(&dir).with_context(|| format!("clean {}", dir.display()))?;
        }
        artifacts.push(match target {
            LinuxPackage::Flatpak => build_flatpak(&app, &dir, &icon_dir)?,
            LinuxPackage::Snap => build_snap(&app, &dir, &icon_dir)?,
            LinuxPackage::Pkgbuild => build_pkgbuild(&app, &dir, &icon_dir)?,
        });
    }
    Ok(artifacts)
}

fn build_flatpak(app: &App<'_>, dir: &Path, icons: &Path) -> Result<PathBuf> {
    stage(app, dir, icons, &app.binary)?;
    let id = &app.cfg.identifier;
    let manifest = dir.join(format!("{id}.yml"));
    fs::write(&manifest, flatpak_manifest(app))?;
    let (Some(builder), Some(flatpak)) = (find_tool("flatpak-builder"), find_tool("flatpak"))
    else {
        warn!(
            "flatpak-builder not found: wrote {}. Install flatpak-builder and the GNOME {FLATPAK_RUNTIME_VERSION} \
             runtime (flatpak install flathub org.gnome.Platform//{FLATPAK_RUNTIME_VERSION} \
             org.gnome.Sdk//{FLATPAK_RUNTIME_VERSION}), then run: flatpak-builder --force-clean \
             --repo=repo build {id}.yml && flatpak build-bundle repo {id}.flatpak {id}",
            manifest.display()
        );
        return Ok(manifest);
    };
    let manifest_name = format!("{id}.yml");
    run_tool(
        &builder,
        &["--force-clean", "--repo=repo", "build", &manifest_name],
        dir,
    )?;
    let bundle_name = format!("{}-{}-{}.flatpak", app.package, app.cfg.version, app.arch);
    run_tool(&flatpak, &["build-bundle", "repo", &bundle_name, id], dir)?;
    let output = app.cfg.out_dir.join(&bundle_name);
    fs::rename(dir.join(&bundle_name), &output)?;
    Ok(output)
}

fn build_snap(app: &App<'_>, dir: &Path, icons: &Path) -> Result<PathBuf> {
    // Inside the snap the binary lives under `$SNAP/bin`, on the app's PATH.
    stage(app, dir, icons, &app.binary)?;
    let snap_dir = dir.join("snap");
    fs::create_dir_all(&snap_dir)?;
    let manifest = snap_dir.join("snapcraft.yaml");
    fs::write(&manifest, snapcraft_yaml(app))?;
    let Some(snapcraft) = find_tool("snapcraft") else {
        warn!(
            "snapcraft not found: wrote {}. Install it (sudo snap install snapcraft --classic), then \
             run `snapcraft pack` in {}",
            manifest.display(),
            dir.display()
        );
        return Ok(manifest);
    };
    run_tool(&snapcraft, &["pack"], dir)?;
    let snap = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "snap")
        })
        .context("snapcraft finished without producing a .snap")?;
    let output = app
        .cfg
        .out_dir
        .join(snap.file_name().context("snap file name")?);
    fs::rename(&snap, &output)?;
    Ok(output)
}

fn build_pkgbuild(app: &App<'_>, dir: &Path, icons: &Path) -> Result<PathBuf> {
    stage(app, dir, icons, &app.binary)?;
    // `makepkg` takes local `source=` entries from the PKGBUILD directory
    // itself, without subdirectories: lay the icons out flat.
    for size in icon_set::HICOLOR_SIZES {
        let name = format!("{size}x{size}.png");
        fs::copy(
            dir.join("icons").join(&name),
            dir.join(format!("icons-{name}")),
        )?;
    }
    let manifest = dir.join("PKGBUILD");
    fs::write(&manifest, pkgbuild(app))?;
    let Some(makepkg) = find_tool("makepkg") else {
        warn!(
            "makepkg not found: wrote {}. On Arch Linux run `makepkg -f` in {}",
            manifest.display(),
            dir.display()
        );
        return Ok(manifest);
    };
    run_tool(&makepkg, &["-f", "--nodeps"], dir)?;
    let package = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".pkg.tar"))
        })
        .context("makepkg finished without producing a package")?;
    let output = app
        .cfg
        .out_dir
        .join(package.file_name().context("package file name")?);
    fs::rename(&package, &output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BundleConfig {
        serde_json::from_value(serde_json::json!({
            "name": "My App",
            "version": "1.2.0-beta.1",
            "identifier": "com.example.myapp",
            "description": "Says \"hi\" for $5",
            "license": "MIT",
            "category": "DeveloperTool",
            "targets": ["flatpak"],
            "project_root": "/project",
            "out_dir": "/project/release",
            "binary_path": "/project/release/my-app",
            "resources": [],
            "icons": [],
            "macos": { "url_schemes": ["myapp"] },
            "file_associations": [{ "ext": ["myd"], "mimeType": "application/x-myd" }],
        }))
        .expect("bundle config")
    }

    fn app(cfg: &BundleConfig) -> App<'_> {
        App {
            cfg,
            binary: "my-app".into(),
            package: package_name(&cfg.name),
            arch: "x86_64".into(),
        }
    }

    #[test]
    fn package_names_are_store_safe() {
        assert_eq!(package_name("My App"), "my-app");
        assert_eq!(package_name("  Ünïcode -- App!! "), "n-code-app");
        assert_eq!(package_name("42 Things"), "app-42-things");
        assert_eq!(package_name("!!!"), "app");
        assert_eq!(package_name("-"), "app");
        assert_eq!(package_name("4"), "app-4");
        assert_eq!(
            package_name(&format!("{}-x", "a".repeat(39))),
            "a".repeat(39)
        );
        let digits = package_name(&format!("1{}", "-b".repeat(30)));
        assert!(!digits.ends_with('-') && digits.len() <= 40, "{digits}");
        assert!(package_name(&"x".repeat(80)).len() <= 40);
    }

    #[test]
    fn desktop_entry_declares_schemes_mime_types_and_categories() {
        let entry = desktop_entry(&config(), "my-app", "com.example.myapp").unwrap();
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\nName=My App\n"));
        assert!(entry.contains("Exec=my-app %U\n"));
        assert!(entry.contains("Categories=Development;\n"));
        assert!(entry.contains("MimeType=x-scheme-handler/myapp;application/x-myd;\n"));
    }

    #[test]
    fn desktop_entry_refuses_line_injection() {
        let mut cfg = config();
        cfg.macos.url_schemes = vec!["myapp\nExec=evil".into()];
        assert!(desktop_entry(&cfg, "my-app", "com.example.myapp").is_err());

        let mut cfg = config();
        cfg.file_associations[0].mime_type = Some("text/x-a;\rExec=evil".into());
        assert!(desktop_entry(&cfg, "my-app", "com.example.myapp").is_err());

        let cfg = config();
        assert!(desktop_entry(&cfg, "my-app\nExec=evil", "com.example.myapp").is_err());
        assert!(desktop_entry(&cfg, "my-app", "icon\nExec=evil").is_err());

        let mut cfg = config();
        cfg.name = "My\nExec=evil".into();
        cfg.description = Some("a\u{7}b".into());
        assert!(desktop_entry(&cfg, "my-app", "com.example.myapp").is_err());
        cfg.description = None;
        let entry = desktop_entry(&cfg, "my-app", "com.example.myapp").unwrap();
        assert!(entry.contains("Name=My Exec=evil\n"));
        assert_eq!(entry.matches("\nExec=").count(), 1);
    }

    #[test]
    fn desktop_exec_quotes_reserved_characters() {
        assert_eq!(desktop_exec("my-app").unwrap(), "my-app");
        assert_eq!(desktop_exec("50%").unwrap(), "50%%");
        assert_eq!(desktop_exec("my app").unwrap(), "\"my app\"");
        // `$` gets a quoting backslash, which the string escape doubles.
        assert_eq!(desktop_exec("a$b").unwrap(), "\"a\\\\$b\"");
    }

    #[test]
    fn flatpak_manifest_installs_every_staged_file() {
        let cfg = config();
        let manifest = flatpak_manifest(&app(&cfg));
        assert!(manifest.starts_with("app-id: \"com.example.myapp\"\n"));
        assert!(manifest.contains("command: \"my-app\""));
        assert!(manifest.contains("/app/share/icons/hicolor/512x512/apps/com.example.myapp.png"));
        assert!(manifest.contains("path: \"com.example.myapp.metainfo.xml\""));
    }

    #[test]
    fn snapcraft_yaml_uses_the_gnome_extension() {
        let cfg = config();
        let yaml = snapcraft_yaml(&app(&cfg));
        assert!(yaml.starts_with("name: my-app\nbase: core24\nversion: \"1.2.0-beta.1\"\n"));
        assert!(yaml.contains("summary: \"Says \\\"hi\\\" for $5\""));
        assert!(yaml.contains("extensions: [gnome]"));
        assert!(yaml.contains("platforms:\n  amd64:"));
        assert!(yaml.contains("\nicon: icons/512x512.png\n"));
    }

    #[test]
    fn pkgbuild_escapes_shell_metacharacters() {
        let mut cfg = config();
        cfg.license_file = Some(PathBuf::from("/project/LICENSE"));
        let text = pkgbuild(&app(&cfg));
        assert!(text.contains("pkgver=1.2.0_beta.1\n"));
        assert!(text.contains("pkgdesc=\"Says \\\"hi\\\" for \\$5\"\n"));
        assert!(text.contains("depends=(\"gtk3\" \"webkit2gtk-4.1\")\n"));
        assert!(text.contains("optdepends=('libayatana-appindicator: system tray icon')"));
        assert!(text.contains("source=(\"LICENSE\" \"my-app\""));
        assert!(text.contains(" \"icons-16x16.png\" "));
        let sources = text
            .lines()
            .find(|line| line.starts_with("source="))
            .unwrap();
        let sums = text
            .lines()
            .find(|line| line.starts_with("sha256sums="))
            .unwrap();
        assert_eq!(
            sources.matches("\" \"").count() + 1,
            sums.matches("'SKIP'").count()
        );
    }

    #[test]
    fn pkgbuild_uses_arch_dependencies_not_rpm_ones() {
        let mut cfg = config();
        cfg.linux.rpm.depends = vec!["webkit2gtk4.1".into()];
        cfg.linux.pkgbuild.depends = vec!["gtk3".into(), "libnotify".into()];
        let text = pkgbuild(&app(&cfg));
        assert!(text.contains("depends=(\"gtk3\" \"webkit2gtk-4.1\" \"libnotify\")\n"));
        assert!(!text.contains("webkit2gtk4.1"));
    }

    #[test]
    fn metainfo_escapes_xml() {
        let mut cfg = config();
        cfg.name = "A <b> & C".into();
        let xml = metainfo(&cfg, "s");
        assert!(xml.contains("<name>A &lt;b&gt; &amp; C</name>"));
        assert!(xml.contains("<project_license>MIT</project_license>"));
    }
}

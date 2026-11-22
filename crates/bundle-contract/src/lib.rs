//! JSON contract between the `tynd` CLI and the `tynd-bundler` executable.
//!
//! The CLI writes one of these requests to a JSON file and runs
//! `tynd-bundler <subcommand> <file>`; the bundler deserializes the same type.
//! Both sides compile against this crate, so a field cannot be renamed, added
//! or dropped on one side only. Every type rejects unknown fields.
//!
//! | Subcommand | Request |
//! | --- | --- |
//! | `bundle` | [`BundleConfig`] |
//! | `codesign` | [`CodesignConfig`] |
//! | `pack` | [`PackConfig`] |
//!
//! `patch`, `archive-app` and `dmg-from-app` take plain command-line flags.
//! The bundler answers on stdout with one artifact path per line.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Oldest macOS release Tynd supports: the host links the WKWebView proxy
/// symbols of Network.framework, which macOS 14 introduced. The bundler writes
/// it as `LSMinimumSystemVersion`; the CLI builds the host with it as
/// `MACOSX_DEPLOYMENT_TARGET`.
pub const MACOS_MINIMUM_SYSTEM_VERSION: &str = "14.0";

const fn default_true() -> bool {
    true
}

// `skip_serializing_if` requires a `&T` predicate.
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}

/// File-name-safe form of an app name: `[A-Za-z0-9._-]` kept, anything else
/// becomes `-`. Names the packed binary and the MSIX package file.
pub fn safe_file_stem(name: &str) -> String {
    name.chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '.') {
                value
            } else {
                '-'
            }
        })
        .collect()
}

/// Request of `tynd-bundler bundle`: build installer artifacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleConfig {
    pub name: String,
    pub version: String,
    pub identifier: String,
    pub description: Option<String>,
    pub long_description: Option<String>,
    pub homepage: Option<String>,
    pub authors: Option<Vec<String>>,
    pub license: Option<String>,
    pub copyright: Option<String>,
    pub category: Option<String>,
    /// License text shown by the NSIS/WiX installers and shipped by the Linux
    /// packages.
    #[serde(default)]
    pub license_file: Option<PathBuf>,
    /// `msi`, `nsis`, `msix`, `macos`, `dmg`, `deb`, `rpm`, `appimage`,
    /// `flatpak`, `snap`, `pkgbuild`.
    pub targets: Vec<String>,
    pub project_root: PathBuf,
    /// Where installers and the artifact zip archive land.
    pub out_dir: PathBuf,
    pub binary_path: PathBuf,
    pub resources: Vec<PathBuf>,
    #[serde(default)]
    pub file_associations: Vec<FileAssociation>,
    /// `.svg`, `.png`, or `.ico` — bundler picks the right output per platform.
    pub icons: Vec<PathBuf>,
    #[serde(default)]
    pub linux: LinuxConfig,
    #[serde(default)]
    pub macos: MacOsConfig,
    #[serde(default)]
    pub windows: WindowsConfig,
    /// Handlebars template for the NSIS installer script. Rendered with the
    /// bundle metadata then handed to tauri-bundler via a temp file.
    #[serde(default)]
    pub custom_nsis_template: Option<String>,
    /// Handlebars template for the WiX `.wxs` installer script.
    #[serde(default)]
    pub custom_wix_template: Option<String>,
}

/// A file type the installed app registers for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileAssociation {
    /// Extensions without the leading dot, e.g. `png`.
    pub ext: Vec<String>,
    /// `LSItemContentTypes` on macOS.
    #[serde(default)]
    pub content_types: Option<Vec<String>>,
    /// `CFBundleTypeName` on macOS; defaults to `ext[0]`.
    #[serde(default)]
    pub name: Option<String>,
    /// Shown in the `Type` column of Windows Explorer.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub role: FileAssociationRole,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub rank: FileAssociationRank,
    /// `UTExportedTypeDeclarations` entry on macOS.
    #[serde(default)]
    pub exported_type: Option<ExportedFileType>,
}

/// `CFBundleTypeRole` on macOS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileAssociationRole {
    #[default]
    Editor,
    Viewer,
    Shell,
    #[serde(rename = "QLGenerator")]
    QlGenerator,
    None,
}

/// `LSHandlerRank` on macOS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileAssociationRank {
    #[default]
    Default,
    Owner,
    Alternate,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportedFileType {
    /// `UTTypeIdentifier`.
    pub identifier: String,
    /// `UTTypeConformsTo`.
    #[serde(default)]
    pub conforms_to: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxConfig {
    #[serde(default)]
    pub deb: DebConfig,
    #[serde(default)]
    pub rpm: RpmConfig,
    #[serde(default)]
    pub appimage: AppImageConfig,
    #[serde(default)]
    pub pkgbuild: PkgbuildConfig,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DebConfig {
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub recommends: Vec<String>,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub replaces: Vec<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
    /// Handlebars template replacing the generated `.desktop` entry (also used
    /// by the AppImage).
    #[serde(default)]
    pub desktop_template: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpmConfig {
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub epoch: Option<u32>,
    /// Handlebars template replacing the generated `.desktop` entry.
    #[serde(default)]
    pub desktop_template: Option<PathBuf>,
}

/// Arch Linux `PKGBUILD`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PkgbuildConfig {
    /// Arch package names added to the default `gtk3` and `webkit2gtk-4.1`.
    #[serde(default)]
    pub depends: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppImageConfig {
    #[serde(default)]
    pub bundle_xdg_open: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacOsConfig {
    /// URL schemes declared under `CFBundleURLTypes`. Required at build time
    /// for Launch Services to know the app accepts `<scheme>://...`.
    #[serde(default)]
    pub url_schemes: Vec<String>,
    /// `LSMinimumSystemVersion`; never below [`MACOS_MINIMUM_SYSTEM_VERSION`].
    #[serde(default)]
    pub minimum_system_version: Option<String>,
    /// Entitlements plist embedded when tauri-bundler signs the `.app`.
    #[serde(default)]
    pub entitlements: Option<PathBuf>,
    /// Plist merged into the generated `Info.plist`.
    #[serde(default)]
    pub info_plist: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsConfig {
    #[serde(default)]
    pub webview_install_mode: Option<WebviewInstallMode>,
    #[serde(default)]
    pub minimum_webview2_version: Option<String>,
    #[serde(default)]
    pub allow_downgrades: Option<bool>,
    /// NSIS install scope.
    #[serde(default)]
    pub install_mode: Option<NsisInstallMode>,
    /// MSIX package identity; required by the `msix` target.
    #[serde(default)]
    pub msix: Option<MsixConfig>,
    /// `requestedExecutionLevel` of the app executable; an elevating level
    /// adds the `allowElevation` capability to the MSIX manifest.
    #[serde(default)]
    pub requested_execution_level: Option<ExecutionLevel>,
}

/// Who the NSIS installer installs for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NsisInstallMode {
    /// `%LOCALAPPDATA%`, no elevation.
    CurrentUser,
    /// `Program Files`, elevates.
    PerMachine,
    /// Lets the user pick; elevates for a per-machine install.
    Both,
}

/// Identity of the MSIX package. `publisher` must equal the subject of the
/// signing certificate, or Windows refuses to install the package.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MsixConfig {
    /// Distinguished name, for example `CN=Example Corp, O=Example Corp, C=US`.
    pub publisher: String,
    /// Shown by the App Installer; defaults to the first author or the app name.
    #[serde(default)]
    pub publisher_display_name: Option<String>,
    /// `MinVersion` of `Windows.Desktop`; defaults to Windows 10 1809.
    #[serde(default)]
    pub min_version: Option<String>,
    /// Four-part package version replacing the one derived from the app
    /// version (whose prerelease and build suffixes are dropped).
    #[serde(default)]
    pub version: Option<String>,
}

/// `requestedExecutionLevel` of the embedded application manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionLevel {
    AsInvoker,
    HighestAvailable,
    RequireAdministrator,
}

impl ExecutionLevel {
    /// The manifest attribute value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AsInvoker => "asInvoker",
            Self::HighestAvailable => "highestAvailable",
            Self::RequireAdministrator => "requireAdministrator",
        }
    }
}

/// How the Windows installer provides the WebView2 runtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum WebviewInstallMode {
    Skip,
    DownloadBootstrapper {
        #[serde(default = "default_true")]
        silent: bool,
    },
    EmbedBootstrapper {
        #[serde(default = "default_true")]
        silent: bool,
    },
    OfflineInstaller {
        #[serde(default = "default_true")]
        silent: bool,
    },
    FixedRuntime {
        /// Project-relative directory of the extracted fixed runtime.
        path: PathBuf,
    },
}

/// Request of `tynd-bundler codesign`: sign, and optionally notarize, macOS
/// artifacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodesignConfig {
    pub action: CodesignAction,
    /// `.app` / `.dmg` / `.pkg` / Mach-O — dispatched per extension.
    pub targets: Vec<PathBuf>,
    pub identity: CodesignIdentity,
    /// Required when `action = notarize`.
    #[serde(default)]
    pub notarization: Option<Notarization>,
    /// Required for notarization, optional otherwise.
    #[serde(default)]
    pub entitlements: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodesignAction {
    /// Embed a signature.
    Sign,
    /// Sign, submit to the Apple notary service and staple.
    Notarize,
}

impl CodesignAction {
    /// The wire name, also used in log lines and cache file names.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sign => "sign",
            Self::Notarize => "notarize",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub enum CodesignIdentity {
    /// Loads a PKCS#12 (.p12) keystore from `path` and reads its
    /// passphrase from the env var named by `password_env`. Works on any
    /// host OS — the keystore is portable so signing runs from CI without
    /// a macOS keychain.
    P12 {
        path: PathBuf,
        /// Env var holding the P12 password — never the value itself.
        password_env: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notarization {
    /// JSON file produced by `rcodesign encode-app-store-connect-api-key`
    /// (bundles key_id, issuer_id, and the .p8 private key).
    pub api_key_path: PathBuf,
}

/// Request of `tynd-bundler pack`: append a `TYNDPKG` section to a host binary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackConfig {
    /// Host executable to wrap (for example `tynd-runtime.exe`).
    pub host_binary: PathBuf,
    /// Destination path for the packed binary; parent must exist.
    pub output: PathBuf,
    /// `.svg`, `.png`, or `.ico`. When absent the default host icon is kept.
    #[serde(default)]
    pub icon: Option<PathBuf>,
    /// Display name (Task Manager / shortcuts) — feeds PE VERSIONINFO.
    pub app_name: String,
    /// Dotted version (`1.2.3.4`) — first four components feed VS_FIXEDFILEINFO.
    pub version: String,
    /// Bun-bundled backend JS.
    pub backend_bundle: PathBuf,
    /// Frontend asset directory; files packed under `frontend/<rel-path>`.
    pub frontend_dir: PathBuf,
    /// Native sidecars packed under `sidecar/<name>`.
    #[serde(default)]
    pub sidecars: Vec<SidecarSpec>,
    /// `tynd-runtime` payload; zstd-compressed at level 19 and packed as
    /// `bun.zst`. Requires `bun_version`.
    #[serde(default)]
    pub bun_runtime: Option<PathBuf>,
    /// Bun release tag (e.g. `1.3.14`) — packed as `bun.version`.
    #[serde(default)]
    pub bun_version: Option<String>,
    /// Project-relative WebView2 fixed-runtime directory. Stored as boot
    /// metadata; installer resources carry the directory itself.
    #[serde(default)]
    pub webview2_runtime_path: Option<String>,
    /// Serialized `observability` config (camelCase JSON object, at most 4096
    /// bytes), packed verbatim as the `observability.json` entry. Absent when
    /// the app has no active telemetry pipeline.
    #[serde(default)]
    pub observability_json: Option<String>,
    /// App identifier (`bundle.identifier`), packed as the `app.identifier`
    /// entry. The packaged host names the app's own directories after it;
    /// absent, it falls back to the executable name.
    #[serde(default)]
    pub app_identifier: Option<String>,
    /// Allow the debug protocol in the packaged app (`tynd build --debug-protocol`):
    /// packs a `debug.flag` entry. Off by default; a packaged host refuses to start
    /// its debug server without it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub debug: bool,
    /// Ignored on non-Windows hosts.
    #[serde(default)]
    pub windows: WindowsPackOptions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SidecarSpec {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsPackOptions {
    /// Embed the rendered ICO into the `.rsrc` section of the output PE.
    #[serde(default = "default_true")]
    pub embed_icon: bool,
    /// Flip subsystem to IMAGE_SUBSYSTEM_WINDOWS_GUI=2 (no console window).
    #[serde(default = "default_true")]
    pub patch_subsystem_gui: bool,
    /// Embed an application manifest requesting this execution level. Absent
    /// means no manifest (Windows runs the app `asInvoker`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_execution_level: Option<ExecutionLevel>,
}

#[cfg(test)]
mod tests;

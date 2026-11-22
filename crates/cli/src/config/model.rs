use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const CONFIG_FILES: &[&str] = &[
    "tynd.config.ts",
    "tynd.config.mts",
    "tynd.config.cts",
    "tynd.config.js",
    "tynd.config.mjs",
    "tynd.config.cjs",
    "tynd.config.json",
    "tynd.config.jsonc",
    "tynd.config.toml",
    "tynd.config.yaml",
    "tynd.config.yml",
];

pub(crate) const CONFIG_EXTENSIONS: &[&str] = &[
    "ts", "mts", "cts", "js", "mjs", "cjs", "json", "jsonc", "toml", "yaml", "yml",
];

// KNOWN_APIS, CORE_HOST_APIS, PLUGIN_FIRST_APIS, API_PLUGIN_PACKAGES and PLUGIN_ADD_NAMES.
include!("api_catalog.generated.rs");

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ResolvedConfig {
    pub value: Value,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedConfig {
    pub config: TyndConfig,
    pub path: Option<PathBuf>,
    pub root: PathBuf,
    /// Config files merged into config, ordered from lowest to highest precedence.
    pub sources: Vec<PathBuf>,
    /// Values loaded from project dotenv files. Existing process variables win.
    pub environment: BTreeMap<String, String>,
}

// This model is the single source of the published JSON schema
// (packages/cli/schemas/tynd.config.json) and of the TypeScript config types
// (packages/cli/src/lib/config/types.generated.ts): doc comments become their
// descriptions and `#[schemars]` attributes their editor-side constraints. Run
// `bun scripts/gen-api-catalog.ts` after changing it.

/// Tynd application configuration, as written in
/// `tynd.config.{ts,js,json,jsonc,yaml,toml}`. Full reference:
/// <https://tynd-apps.github.io/tynd/docs/v1.0/cli/config>
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[schemars(title = "Tynd configuration", transform = super::schema::config_file_keys)]
pub(crate) struct TyndConfig {
    #[schemars(with = "Vec<super::schema::ApiName>")]
    pub apis: Vec<String>,
    pub plugins: Vec<PluginEntry>,
    #[schemars(length(min = 1))]
    pub backend: PathBuf,
    #[schemars(length(min = 1))]
    pub frontend_dir: PathBuf,
    #[schemars(url, regex(pattern = r"^https?://"))]
    pub dev_url: Option<String>,
    #[schemars(length(min = 1))]
    pub dev_command: Option<String>,
    #[schemars(inner(length(min = 1)))]
    pub dev_command_args: Option<Vec<String>>,
    #[schemars(length(min = 1))]
    pub icon: Option<PathBuf>,
    pub binary_args: Vec<String>,
    #[schemars(inner(regex(pattern = r"^[a-zA-Z][a-zA-Z0-9+\-.]*$")))]
    pub protocols: Vec<String>,
    pub bundle: Option<BundleConfig>,
    pub sidecars: Vec<SidecarConfig>,
    pub updater: Option<UpdaterConfig>,
    pub observability: Option<ObservabilityConfig>,
}

impl Default for TyndConfig {
    fn default() -> Self {
        Self {
            apis: Vec::new(),
            plugins: Vec::new(),
            backend: PathBuf::from("backend/main.ts"),
            frontend_dir: PathBuf::from("frontend"),
            dev_url: None,
            dev_command: None,
            dev_command_args: None,
            icon: None,
            binary_args: Vec::new(),
            protocols: Vec::new(),
            bundle: None,
            sidecars: Vec::new(),
            updater: None,
            observability: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum PluginEntry {
    Package(#[schemars(length(min = 1))] String),
    Detailed(PluginConfig),
}

impl PluginEntry {
    pub(crate) fn package(&self) -> &str {
        match self {
            Self::Package(package) => package,
            Self::Detailed(plugin) => &plugin.pkg,
        }
    }

    pub(crate) fn features(&self) -> &[String] {
        match self {
            Self::Package(_) => &[],
            Self::Detailed(plugin) => &plugin.features,
        }
    }

    pub(crate) fn configured_apis(&self) -> Vec<String> {
        match self {
            Self::Package(_) => Vec::new(),
            Self::Detailed(plugin) => {
                let mut apis = plugin.apis.clone();
                apis.extend(
                    plugin
                        .permissions
                        .iter()
                        .filter_map(|permission| permission_api(permission).map(str::to_owned)),
                );
                apis
            },
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub(crate) struct PluginConfig {
    #[schemars(length(min = 1), extend("x-tynd-required" = true))]
    pub pkg: String,
    #[schemars(inner(regex(pattern = r"^[A-Za-z0-9_-]+$")))]
    pub features: Vec<String>,
    #[schemars(with = "Vec<super::schema::ApiName>")]
    pub apis: Vec<String>,
    #[schemars(inner(regex(pattern = r"^[a-z][a-z0-9-]*:(?:\*|[a-z][A-Za-z0-9]*)$")))]
    pub permissions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SidecarConfig {
    #[schemars(regex(pattern = r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$"))]
    pub name: String,
    #[schemars(length(min = 1))]
    pub path: PathBuf,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub(crate) struct BundleConfig {
    #[schemars(
        extend("x-tynd-required" = true),
        regex(pattern = r"^[A-Za-z][A-Za-z0-9-]*(?:\.[A-Za-z0-9-]+){2,}$")
    )]
    pub identifier: String,
    #[schemars(length(min = 1))]
    pub display_name: Option<String>,
    #[schemars(inner(length(min = 1)))]
    pub categories: Vec<String>,
    pub short_description: Option<String>,
    pub long_description: Option<String>,
    pub copyright: Option<String>,
    /// Project-relative license file shown by the Windows installers and shipped by the
    /// Linux packages.
    #[schemars(length(min = 1))]
    pub license_file: Option<String>,
    #[schemars(inner(length(min = 1)))]
    pub resources: Vec<String>,
    pub file_associations: Vec<FileAssociation>,
    pub deb: Option<DebBundleConfig>,
    pub rpm: Option<RpmBundleConfig>,
    pub appimage: Option<AppImageBundleConfig>,
    pub pkgbuild: Option<PkgbuildBundleConfig>,
    pub windows: Option<WindowsBundleConfig>,
    pub macos: Option<MacosBundleConfig>,
    pub sign: Option<SigningConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FileAssociation {
    #[schemars(
        length(min = 1),
        inner(regex(pattern = r"^\.?[A-Za-z0-9][A-Za-z0-9.+_-]*$"))
    )]
    pub ext: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(inner(length(min = 1)))]
    pub content_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<FileAssociationRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<FileAssociationRank>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exported_type: Option<ExportedFileType>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) enum FileAssociationRole {
    Editor,
    Viewer,
    Shell,
    #[serde(rename = "QLGenerator")]
    QlGenerator,
    None,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) enum FileAssociationRank {
    Default,
    Owner,
    Alternate,
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExportedFileType {
    #[schemars(length(min = 1))]
    pub identifier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(inner(length(min = 1)))]
    pub conforms_to: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DebBundleConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(inner(length(min = 1)))]
    pub depends: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub priority: Option<String>,
    /// Project-relative Handlebars template replacing the generated `.desktop` entry
    /// (also used by the AppImage).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub desktop_template: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RpmBundleConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(inner(length(min = 1)))]
    pub requires: Option<Vec<String>>,
    /// Project-relative Handlebars template replacing the generated `.desktop` entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub desktop_template: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PkgbuildBundleConfig {
    /// Arch package names added to the default `gtk3` and `webkit2gtk-4.1`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(inner(length(min = 1)))]
    pub depends: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AppImageBundleConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_xdg_open: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WindowsBundleConfig {
    /// Defaults to `{ type: "skip" }`; no registry probe or runtime download is performed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = { "type": "skip" }))]
    pub webview_install_mode: Option<WebView2InstallMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^\d+(?:\.\d+){3}$"))]
    pub minimum_web_view2_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_downgrades: Option<bool>,
    /// NSIS install scope (default `currentUser`: no elevation).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = "currentUser"))]
    pub install_mode: Option<NsisInstallMode>,
    /// UAC level of the app executable, embedded as its application manifest. Unset means
    /// no manifest (`asInvoker`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_execution_level: Option<ExecutionLevel>,
    /// MSIX package identity, required by the `msix` bundle target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msix: Option<MsixBundleConfig>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NsisInstallMode {
    CurrentUser,
    PerMachine,
    Both,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ExecutionLevel {
    AsInvoker,
    HighestAvailable,
    RequireAdministrator,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MsixBundleConfig {
    /// Publisher distinguished name; must equal the subject of the signing certificate
    /// (for example `CN=Example Corp, O=Example Corp, C=US`).
    #[schemars(regex(pattern = r"^CN="))]
    pub publisher: String,
    /// Defaults to the package author, else the app name.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub publisher_display_name: Option<String>,
    /// `MinVersion` of `Windows.Desktop` (default `10.0.17763.0`, Windows 10 1809).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^\d+\.\d+\.\d+\.\d+$"))]
    pub min_version: Option<String>,
    /// Four-part package version (each part at most 65535). Defaults to the app version
    /// cut to its numeric core, which maps `1.2.3-beta.1` and `1.2.3` to the same
    /// `1.2.3.0`. The Microsoft Store requires the fourth part to be 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^\d+\.\d+\.\d+\.\d+$"))]
    pub version: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum WebView2InstallMode {
    Skip,
    DownloadBootstrapper {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        silent: Option<bool>,
    },
    EmbedBootstrapper {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        silent: Option<bool>,
    },
    OfflineInstaller {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        silent: Option<bool>,
    },
    FixedRuntime {
        #[schemars(length(min = 1))]
        path: String,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MacosBundleConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(inner(regex(pattern = r"^[a-zA-Z][a-zA-Z0-9+\-.]*$")))]
    pub url_schemes: Option<Vec<String>>,
    /// `LSMinimumSystemVersion` (default and floor `14.0`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^\d+(?:\.\d+){0,2}$"), extend("default" = "14.0"))]
    pub minimum_system_version: Option<String>,
    /// Project-relative entitlements plist used when the `.app` is signed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub entitlements: Option<String>,
    /// Project-relative plist merged into the generated `Info.plist`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub info_plist: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SigningConfig {
    pub windows: Option<WindowsSigningConfig>,
    pub macos: Option<MacosSigningConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WindowsSigningConfig {
    #[schemars(length(min = 1))]
    pub certificate: String,
    pub password: Option<String>,
    #[schemars(url, regex(pattern = r"^https?://"))]
    pub timestamp_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MacosSigningConfig {
    #[schemars(length(min = 1))]
    pub identity: PathBuf,
    pub password: Option<String>,
    pub entitlements: Option<PathBuf>,
    pub notarize: Option<MacosNotarizeConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MacosNotarizeConfig {
    #[schemars(length(min = 1))]
    pub api_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum ByteSize {
    Bytes(u64),
    Human(String),
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PromptStyle {
    #[default]
    Modal,
    Tray,
    Silent,
    Manual,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "fields mirror the public updater configuration contract"
)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpdaterConfig {
    /// Manifest URLs tried in order (fallback on network error, HTTP 5xx, 408 and 429). Each
    /// may use `{{channel}}`, `{{target}}` (windows|darwin|linux), `{{arch}}`
    /// (`x86_64`|`aarch64`) and `{{current_version}}`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(
        length(min = 1),
        inner(regex(pattern = r"^https?://")),
        extend("uniqueItems" = true, "x-tynd-required" = true)
    )]
    pub endpoints: Vec<String>,
    /// Trusted Ed25519 public keys (base64). A signature is valid when any key verifies it,
    /// so a rotation ships old + new, signs with the new key, then drops the old one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(
        length(min = 1),
        inner(length(min = 1)),
        extend("uniqueItems" = true, "x-tynd-required" = true)
    )]
    pub public_keys: Vec<String>,
    /// Default release channel (`"stable"` when unset): letters, digits, `.`, `_`, `-`,
    /// 1..64 characters, not starting with `.`. `updater.setChannel()` overrides it at
    /// runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        regex(pattern = r"^[A-Za-z0-9_-][A-Za-z0-9._-]{0,63}$"),
        extend("default" = "stable")
    )]
    pub channel: Option<String>,
    /// `http://` or `https://` proxy (`scheme://host[:port]`, no query or fragment) for
    /// updater traffic. Without it the host falls back to `HTTPS_PROXY` / `ALL_PROXY`.
    /// Backend-only: never injected into the renderer-visible runtime config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^https?://[^?#]+$"))]
    pub proxy: Option<String>,
    /// Fixed request headers for private update servers (e.g. `Authorization`). Values may
    /// use `${env:NAME}`, expanded from the environment / `.env` files when the CLI
    /// generates the backend bootstrap, so a token need not be committed. Sent only to the
    /// origins of the configured endpoints (at least one endpoint needs a fixed origin),
    /// never on a cross-origin redirect. Transport-owned names (`Host`, `Content-Length`,
    /// `Transfer-Encoding`, `Connection`, `Proxy-Authorization`, ...) are rejected; at most
    /// 32 headers, values <= 4096 bytes. Backend-only, like `proxy`. A packaged app carries
    /// the expanded value in its backend bundle: use a low-privilege read token.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend(
        "maxProperties" = 32,
        "propertyNames" = { "pattern": "^[!#$%&'*+.^_`|~0-9A-Za-z-]+$" },
        "additionalProperties" = { "type": "string", "maxLength": 4096 }
    ))]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "yes")]
    pub prefer_patch: bool,
    #[serde(default = "yes")]
    pub allow_patch: bool,
    pub max_patch_size: Option<ByteSize>,
    #[serde(default = "yes")]
    pub fallback_to_full: bool,
    /// Periodic update check every `checkInterval`. `autoCheck`, `checkOnStartup` and
    /// `autoDownload` are driven by the frontend: they only run once the main window has
    /// imported `@tynd/plugin-updater/client` (an app that never loads that module never
    /// checks).
    #[serde(default = "yes")]
    pub auto_check: bool,
    #[serde(default = "default_check_interval")]
    pub check_interval: String,
    /// Run one check at startup. Same requirement as `autoCheck`.
    #[serde(default = "yes")]
    pub check_on_startup: bool,
    /// Download every update a check finds. Same requirement as `autoCheck`.
    #[serde(default = "yes")]
    pub auto_download: bool,
    #[serde(default)]
    pub auto_apply: bool,
    #[serde(default)]
    pub prompt_style: PromptStyle,
    /// Signature checks cannot be turned off.
    #[serde(default = "yes")]
    #[schemars(extend("const" = true))]
    pub require_signature: bool,
    #[serde(default = "yes")]
    pub require_major_prompt: bool,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "resolved fields preserve the public updater configuration contract"
)]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResolvedUpdaterConfig {
    /// Full ordered mirror list.
    pub endpoints: Vec<String>,
    /// Every trusted key, in config order.
    pub public_keys: Vec<String>,
    /// Default release channel; `stable` when the config leaves it unset.
    pub channel: String,
    pub prefer_patch: bool,
    pub allow_patch: bool,
    pub max_patch_size: Option<u64>,
    pub fallback_to_full: bool,
    pub auto_check: bool,
    pub check_interval_secs: u64,
    pub check_on_startup: bool,
    pub auto_download: bool,
    pub auto_apply: bool,
    pub prompt_style: PromptStyle,
    pub require_signature: bool,
    pub require_major_prompt: bool,
}

/// Host-trusted updater network settings, with `${env:NAME}` already expanded.
/// Emitted only into the generated backend bootstrap (`updater.configure`),
/// never into the HTML or `TYND_FRONTEND_RUNTIME_CONFIG`, which the renderer
/// can read. `Debug` redacts the header values (usually credentials).
#[derive(Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolvedUpdaterNetwork {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl fmt::Debug for ResolvedUpdaterNetwork {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedUpdaterNetwork")
            .field("proxy", &self.proxy.as_ref().map(|_| "<redacted>"))
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .finish()
    }
}

// Serialized as-is into the packed `observability.json` entry / the dev
// `TYND_OBSERVABILITY` variable.
/// Opt-in production telemetry. Absent means disabled (no init, no cost). The host compiles
/// `obs-sentry` / `obs-otel` in only for the pipelines you configure.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObservabilityConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentry: Option<SentryConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otel: Option<OtelConfig>,
}

/// Sentry crash reporting. A blank `dsn` disables the pipeline.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(rename = "SentryObservabilityConfig")]
pub(crate) struct SentryConfig {
    /// Sentry DSN. Blank disables the pipeline.
    #[schemars(regex(pattern = r"^(?:$|https?://)"))]
    pub dsn: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub release: Option<String>,
    /// Fraction of events sent, 0..1 (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 1), extend("default" = 1))]
    pub sample_rate: Option<f64>,
}

/// `OpenTelemetry` OTLP/gRPC trace export. A blank `endpoint` disables the pipeline.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(rename = "OtelObservabilityConfig")]
pub(crate) struct OtelConfig {
    /// OTLP/gRPC collector URL. Blank disables the pipeline.
    #[schemars(regex(pattern = r"^(?:$|https?://)"))]
    pub endpoint: String,
    /// Defaults to the executable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub service_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EffectiveHostPlan {
    pub apis: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct ConfigError {
    pub issues: Vec<String>,
    pub path: Option<PathBuf>,
}

impl ConfigError {
    pub(super) fn new(path: Option<&Path>, issues: Vec<String>) -> Self {
        Self {
            issues,
            path: path.map(Path::to_path_buf),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            writeln!(formatter, "invalid Tynd config {}", path.display())?;
        } else {
            writeln!(formatter, "invalid Tynd config")?;
        }
        for issue in &self.issues {
            writeln!(formatter, "- {issue}")?;
        }
        Ok(())
    }
}

impl Error for ConfigError {}

/// Host API a `<api>:<method>` permission selector grants (`network-loopback` and
/// `network-public` both map to `network`), or `None` when it is malformed or
/// names an unknown API.
pub(crate) fn permission_api(permission: &str) -> Option<&str> {
    let (api, method) = permission.split_once(':')?;
    if api.is_empty()
        || method.is_empty()
        || method.contains(':')
        || !(KNOWN_APIS.contains(&api) || matches!(api, "network-loopback" | "network-public"))
        || (method != "*"
            && (!method.starts_with(|character: char| character.is_ascii_lowercase())
                || !method
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())))
    {
        return None;
    }
    Some(if matches!(api, "network-loopback" | "network-public") {
        "network"
    } else {
        api
    })
}

const fn yes() -> bool {
    true
}

fn default_check_interval() -> String {
    "12h".to_owned()
}

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};

const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_RESCAN_INTERVAL_MS: u64 = 5_000;
const DEFAULT_MAX_MCP_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "tynd",
    about = "Tynd — native TypeScript desktop apps.",
    version
)]
pub struct Cli {
    /// Show debug-level logs.
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,

    /// Suppress everything except errors.
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    /// Disable ANSI colors.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Set TYND_ENV for user code.
    #[arg(long, global = true)]
    pub env: Option<String>,

    /// Select the human or JSON log renderer.
    #[arg(long, value_enum, default_value_t = LogFormat::Human, global = true)]
    pub log_format: LogFormat,

    /// Mirror logs to this file.
    #[arg(long, global = true)]
    pub log_file: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum LogFormat {
    #[default]
    Human,
    Json,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Scaffold a new Tynd project.
    Create(CreateArgs),
    /// Add Tynd to an existing project.
    Init(InitArgs),
    /// Add one or more capabilities or plugins.
    Add(AddArgs),
    /// Remove one or more capabilities or plugins.
    Remove(RemoveArgs),
    /// Start the app in development mode.
    Dev(DevArgs),
    /// Run cached bundles without rebuilding.
    Start(StartArgs),
    /// Run the project's end-to-end tests (`bun test`) against the debug socket.
    Test(TestArgs),
    /// Build and package the app.
    Build(BuildArgs),
    /// Generate every app icon (.ico, .icns, Linux PNGs, MSIX assets) from one PNG or SVG.
    Icon(IconArgs),
    /// Remove generated build artifacts.
    Clean(CleanArgs),
    /// Upgrade Tynd packages and plugins.
    Upgrade(UpgradeArgs),
    /// Validate config and project structure.
    Validate(ValidateArgs),
    /// Diagnose project setup and persisted state.
    Doctor(DoctorArgs),
    /// Show environment and project information.
    Info(InfoArgs),
    /// Report app-size contributors.
    Size(SizeArgs),
    /// Explain the runtime, a capability or an error code.
    Explain(ExplainArgs),
    /// Inspect the project configuration.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Author and inspect plugins.
    #[command(subcommand)]
    Plugin(PluginCommand),
    /// Sign, publish and serve updater releases.
    #[command(subcommand)]
    Updater(UpdaterCommand),
    /// Create a metadata-only diagnostic bundle without project contents.
    Report(ReportArgs),
    /// Profile a development session.
    Profile(ProfileArgs),
    /// Print shell completions.
    Completions(CompletionsArgs),
}

/// `tynd config <command>`.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the fully resolved configuration.
    Show(ConfigShowArgs),
}

/// `tynd plugin <command>`.
#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    /// Scaffold a plugin package.
    New(PluginNewArgs),
    /// Validate a plugin package layout.
    Check(PluginCheckArgs),
    /// Develop a plugin against a scratch app.
    Dev(PluginDevArgs),
    /// List configured plugins.
    List(PluginListArgs),
    /// Regenerate native plugin TypeScript types.
    Types(PluginTypesArgs),
}

/// `tynd updater <command>`.
#[derive(Debug, Subcommand)]
pub enum UpdaterCommand {
    /// Generate an updater Ed25519 keypair (generic, or app-scoped with `--app-id`).
    Keygen(KeygenArgs),
    /// Sign a file with an updater private key.
    Sign(SignArgs),
    /// Sign and publish built artifacts as an updater release.
    Publish(PublishArgs),
    /// Run validation, build, and publish.
    Release(ReleaseArgs),
    /// Serve or scaffold an updater server.
    Server(ServerArgs),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Project name.
    pub name: String,

    /// Frontend framework: react (default), vue, svelte, solid,
    /// preact, lit or angular. Every starter uses TypeScript, so a `-ts`
    /// suffix (e.g. `react-ts`) is accepted as an alias.
    #[arg(long, value_name = "FRAMEWORK")]
    pub framework: Option<String>,

    /// Skip dependency installation.
    #[arg(long = "no-install")]
    pub no_install: bool,

    /// Skip git initialization.
    #[arg(long = "no-git")]
    pub no_git: bool,

    /// Overwrite an existing directory.
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Starter template understood by giget.
    #[arg(short = 't', long)]
    pub template: Option<String>,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Capability aliases or plugin package names.
    #[arg(value_name = "CAPABILITY_OR_PACKAGE")]
    pub inputs: Vec<String>,

    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    /// Edit configuration without running the package manager.
    #[arg(long)]
    pub no_install: bool,

    /// Comma-separated OS API allow-list override.
    #[arg(long)]
    pub apis: Option<String>,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Capability aliases (host APIs, catalog plugins) or plugin package names.
    #[arg(value_name = "CAPABILITY_OR_PACKAGE", required = true)]
    pub inputs: Vec<String>,

    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    /// Edit configuration without uninstalling the packages.
    #[arg(long)]
    pub no_uninstall: bool,
}

/// Flat command-line switches intentionally mirror Clap's public CLI contract.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Args)]
pub struct DevArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub inspect: bool,

    #[arg(long)]
    pub inspect_brk: bool,

    /// Do not expose the development debug socket.
    #[arg(long)]
    pub no_debug: bool,

    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Skip self-healing development fixes.
    #[arg(long = "no-fix")]
    pub no_fix: bool,
}

#[derive(Debug, Args)]
pub struct StartArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,
}

/// Default per-test timeout handed to `bun test`: launching an app is slow. Must stay above the
/// `@tynd/test` launch timeout (120 s) plus its cleanup, or an aborted launch leaks the app.
pub(crate) const DEFAULT_TEST_TIMEOUT_MS: u64 = 180_000;

#[derive(Debug, Args)]
pub struct TestArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    /// Per-test timeout in milliseconds (ignored when `--timeout` is passed to bun).
    #[arg(long, default_value_t = DEFAULT_TEST_TIMEOUT_MS)]
    pub timeout_ms: u64,

    /// Rewrite screenshot baselines instead of failing on a mismatch.
    #[arg(long)]
    pub update_snapshots: bool,

    /// Extra arguments forwarded to `bun test` (file filters, `--bail`, ...).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

#[derive(Debug, Args)]
pub struct BuildArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    /// Output path of the app binary.
    #[arg(short = 'o', long)]
    pub out: Option<PathBuf>,

    /// "all" or a comma-separated list of bundle formats.
    #[arg(long)]
    pub bundle: Option<String>,

    /// Let the packaged app serve the debug protocol (`TYND_DEBUG_SOCKET` / `TYND_DEBUG_PORT`).
    /// Off by default: a packaged app never starts its debug server without it. Needed to drive
    /// a packaged binary with `@tynd/test`; do not ship such a build.
    #[arg(long)]
    pub debug_protocol: bool,
}

#[derive(Debug, Args)]
pub struct IconArgs {
    /// Square PNG (at least 512x512, 1024x1024 recommended) or SVG. Defaults to `icon` in
    /// tynd.config.ts.
    pub source: Option<PathBuf>,

    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    /// Output directory, relative to the project root.
    #[arg(short = 'o', long, default_value = "icons")]
    pub out: PathBuf,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(short = 'f', long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct CleanArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(short = 'y', long)]
    pub yes: bool,

    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Also remove the host build cache shared by every project on this machine.
    #[arg(long)]
    pub cache: bool,
}

#[derive(Debug, Args)]
pub struct ValidateArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub ci: bool,
}

#[derive(Debug, Args)]
pub struct UpgradeArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct ConfigShowArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ExplainArgs {
    /// runtime, capabilities, errors, a capability name, or an error code (e.g. BACKEND_DEAD).
    pub topic: Option<String>,

    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SizeArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub json: bool,
}

/// Doctor exposes independent output, inspection, and repair switches.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Args)]
pub struct DoctorArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub fix: bool,

    #[arg(long)]
    pub state: bool,

    #[arg(long)]
    pub repair: bool,

    #[arg(short = 'y', long)]
    pub yes: bool,

    #[arg(long)]
    pub dry_run: bool,
}

/// `tynd updater keygen`: a generic `tynd-updater.key` + `.pub` pair by default; with `--app-id`,
/// the app-scoped key `~/.tynd/keys/<id>.key` whose public key is recorded in `tynd.config`.
#[derive(Debug, Args)]
#[allow(
    clippy::option_option,
    reason = "clap encodes a flag with an optional value as Option<Option<_>>"
)]
pub struct KeygenArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    /// Generate the app-scoped key of this app id (`bundle.identifier` when no value is given).
    #[arg(long, value_name = "ID", num_args = 0..=1)]
    pub app_id: Option<Option<String>>,

    /// Private key path (generic default `tynd-updater.key`; its public key goes to the same path
    /// with a `.pub` extension). A key replaced with `--force` is backed up first.
    #[arg(short = 'o', long)]
    pub out: Option<PathBuf>,

    /// Do not update tynd.config after generating an app-scoped key.
    #[arg(long, requires = "app_id")]
    pub no_config_edit: bool,

    #[arg(short = 'f', long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct SignArgs {
    pub file: PathBuf,

    #[arg(long, default_value = "tynd-updater.key")]
    pub key: PathBuf,

    #[arg(short = 'o', long)]
    pub out: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum PublishTarget {
    R2,
    S3,
    #[default]
    ManifestOnly,
    GithubReleases,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum PatchStrategy {
    #[default]
    SameMajor,
    LastN,
    All,
}

#[derive(Debug, Args)]
pub struct PublishArgs {
    pub version: String,

    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long, value_enum, default_value_t = PublishTarget::ManifestOnly)]
    pub target: PublishTarget,

    #[arg(long)]
    pub key: Option<PathBuf>,

    #[arg(long)]
    pub bucket: Option<String>,

    #[arg(long)]
    pub region: Option<String>,

    #[arg(long)]
    pub endpoint: Option<String>,

    #[arg(long)]
    pub public_base_url: Option<String>,

    #[arg(long, value_enum, default_value_t = PatchStrategy::SameMajor)]
    pub patch_strategy: PatchStrategy,

    #[arg(long, default_value_t = 5)]
    pub max_diffs: usize,

    #[arg(long, default_value_t = 30)]
    pub max_patches: usize,

    #[arg(long)]
    pub notes_file: Option<PathBuf>,

    /// Publish full payloads only, without binary patches.
    #[arg(long)]
    pub no_diffs: bool,

    /// Release channel written into the manifest (stable, beta, ...).
    /// Charset [A-Za-z0-9._-], max 64.
    #[arg(long)]
    pub channel: Option<String>,

    /// Staged rollout percentage 0-100 written into the manifest (omitted = everyone).
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub rollout: Option<u8>,
}

/// `tynd updater release`: validate, build, then publish with every `tynd updater publish` option.
#[derive(Debug, Args)]
pub struct ReleaseArgs {
    #[command(flatten)]
    pub publish: PublishArgs,

    #[arg(long)]
    pub bundle: Option<String>,

    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct ReportArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(short = 'o', long)]
    pub out: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PluginNewArgs {
    /// Target directory; its last component names the plugin.
    pub path: PathBuf,

    /// Include a Rust native plugin crate. Without this flag, scaffold TypeScript only.
    #[arg(long)]
    pub native: bool,

    #[arg(long)]
    pub scope: Option<String>,

    #[arg(short = 'f', long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct PluginCheckArgs {
    /// Plugin package directory.
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PluginDevArgs {
    pub plugin: String,

    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub scratch: Option<PathBuf>,

    #[arg(short = 'f', long)]
    pub force: bool,

    /// Build the scratch app once without starting its development server.
    #[arg(long = "no-watch")]
    pub no_watch: bool,
}

#[derive(Debug, Args)]
pub struct PluginListArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PluginTypesArgs {
    pub plugin: Option<String>,

    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub dry_run: bool,

    #[arg(long)]
    pub check: bool,
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("profile_mode")
        .required(true)
        .multiple(false)
        .args(["cpu", "memory", "ipc"])
))]
pub struct ProfileArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub cpu: bool,

    #[arg(long)]
    pub memory: bool,

    #[arg(long)]
    pub ipc: bool,

    #[arg(short = 'o', long)]
    pub out: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum DeployProvider {
    Cloudflare,
    Vercel,
    Deno,
}

impl DeployProvider {
    /// The provider's CLI name, also its template directory.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cloudflare => "cloudflare",
            Self::Vercel => "vercel",
            Self::Deno => "deno",
        }
    }
}

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    #[arg(long)]
    pub serve: bool,

    #[arg(long, default_value_t = 3_000)]
    pub port: u16,

    #[arg(long, value_enum)]
    pub deploy: Option<DeployProvider>,

    #[arg(long)]
    pub manifest_url: Option<String>,

    #[arg(long)]
    pub public_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    #[arg(value_enum)]
    pub shell: CompletionShell,
}

#[derive(Debug, Parser)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent MCP security and discovery switches are explicit CLI flags"
)]
#[command(name = "tynd-mcp", about = "MCP bridge for running Tynd apps", version)]
pub struct McpCli {
    #[command(subcommand)]
    pub command: Option<McpCommand>,

    /// Allow evaluation tools exposed by the debug bridge.
    #[arg(long, global = true)]
    pub allow_eval: bool,

    /// Allow tools that launch external resources.
    #[arg(long, global = true)]
    pub allow_launch: bool,

    /// Automatically switch to another discovered app.
    #[arg(long, global = true)]
    pub auto_switch: bool,

    /// Permit discovery endpoints outside loopback interfaces.
    #[arg(long, global = true)]
    pub allow_non_loopback_discovery: bool,

    /// Restrict discovery to one application id.
    #[arg(long, global = true)]
    pub app_id: Option<String>,

    #[arg(
        long,
        default_value_t = DEFAULT_RESCAN_INTERVAL_MS,
        global = true
    )]
    pub rescan_interval_ms: u64,

    #[arg(
        long,
        default_value_t = DEFAULT_CONNECT_TIMEOUT_MS,
        global = true
    )]
    pub connect_timeout_ms: u64,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Serve the MCP protocol over stdio.
    Serve(McpServeArgs),
    /// Invoke one debug-bridge tool and print its result.
    Call(McpCallArgs),
    /// Print debug events until interrupted.
    Tap(McpTapArgs),
}

#[derive(Debug, Args, Default)]
pub struct McpServeArgs {}

#[derive(Debug, Args)]
pub struct McpCallArgs {
    /// Tool name.
    pub tool: String,

    /// JSON object passed as tool arguments.
    #[arg(long = "args")]
    pub json_args: Option<String>,

    #[arg(long)]
    pub app_id: Option<String>,

    #[arg(long, default_value_t = DEFAULT_MAX_MCP_BYTES)]
    pub max_bytes: usize,

    #[arg(long, default_value_t = DEFAULT_CONNECT_TIMEOUT_MS)]
    pub connect_timeout_ms: u64,
}

#[derive(Debug, Args)]
pub struct McpTapArgs {
    #[arg(long)]
    pub app_id: Option<String>,

    #[arg(long, default_value_t = DEFAULT_CONNECT_TIMEOUT_MS)]
    pub connect_timeout_ms: u64,
}

#[cfg(test)]
mod tests;

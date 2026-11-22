//! Build orchestration: resolve the inputs, generate the workspace, run
//! cargo and install the host binary in the project.

use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};

use crate::{
    config::{
        resolve_effective_host_plan, updater_pubkey_pin, TyndConfig, CORE_HOST_APIS, KNOWN_APIS,
    },
    fsx::absolute,
    output::{debug, quiet, warn},
    process::{run_streaming, CommandSpec},
    project::{
        host_cache::{
            cache_root, host_cache_key, long_target_dir_warning, prepare_host_target_dir,
        },
        platform::{executable_name, host_binary_path},
        rust_toolchain::{ensure_rust_toolchain, RustToolchain},
        shared::{is_file, sanitize_crate_name},
    },
};

use super::{
    fingerprint::{
        compiler_fingerprint_input, host_build_fingerprint, seed_cargo_lock, FINGERPRINT_FILE,
    },
    manifest::{
        observability_host_features, render_custom_cargo_manifest, render_custom_main,
        write_generated_file,
    },
    plugin_copy::cache_native_plugins,
    plugins::{resolve_native_build_plugins, NativeBuildPlugin},
    sources::{resolve_host_sources, rust_source_paths, HostSources, HOST_SOURCES_PACKAGE},
};

/// One-line notice printed before a project's first host build of a profile.
fn first_build_notice(target_dir: &Path) -> String {
    format!(
        "Compiling Tynd host (first build can take a few minutes; dependencies are cached \
         for every project in {})…",
        target_dir.display()
    )
}

/// Build the app's native host: with the cargo `dev` profile for `tynd dev`
/// (`release == false`), `release` for `tynd build`.
///
/// The workspace is generated under `<cwd>/.tynd/build` and is deterministic
/// for one config: APIs, plugin features, dependency declarations, and
/// registrations are sorted. Cargo builds into a target directory shared by
/// every project using the same host sources and compiler (see
/// [`crate::project::host_cache`]), and compiles the Tynd crates and the installed
/// plugin crates from copies in that cache, at the same path for every
/// project, so they and the dependencies are compiled once per machine
/// (editable plugins are copied to `.tynd/build/plugins`). The binary is then copied
/// to `.tynd/build/bin/<profile>`, which is what this returns. A dev build whose inputs did not change since the last
/// successful one skips cargo and returns the existing copy.
pub(crate) fn build_custom_host(cwd: &Path, config: &TyndConfig, release: bool) -> Result<PathBuf> {
    let cwd = absolute(cwd)?;
    let plan = resolve_effective_host_plan(config, &cwd)?;
    let mut plugins = resolve_native_build_plugins(&cwd, &config.plugins)?;
    plugins.sort_by(|left, right| left.crate_name.cmp(&right.crate_name));
    let apis = collect_host_apis(plan.apis, &plugins)?;

    let cache_root = cache_root()?;
    let host_sources = resolve_host_sources(&cwd, &cache_root)?;
    let sources = rust_source_paths(&host_sources.root)?;
    if !plugins.is_empty() && !sources.plugin_sdk.join("Cargo.toml").is_file() {
        bail!(
            "plugin SDK not found at {}; reinstall the matching @tynd/host-src package",
            sources.plugin_sdk.display()
        );
    }
    let build_dir = prepare_custom_build_directory(&cwd)?;
    cache_native_plugins(&cache_root, &build_dir, &mut plugins, &sources.plugin_sdk)?;

    let app_name = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| "app".to_owned(), sanitize_crate_name);
    let bin_name = host_bin_name(&app_name, &cwd);
    let cargo_manifest = render_custom_cargo_manifest(
        &app_name,
        &bin_name,
        &sources,
        &apis,
        &plugins,
        &observability_host_features(config),
        release,
    );
    let generated_main = render_custom_main(&plugins)?;

    let toolchain = ensure_rust_toolchain(&build_dir)?;
    let binary = host_binary_path(&cwd, release)?;
    let fingerprint_path = binary.with_file_name(FINGERPRINT_FILE);
    // Release builds always go through cargo, which decides what is stale.
    let fingerprint = if release {
        None
    } else {
        host_build_fingerprint(
            &host_sources,
            &cargo_manifest,
            &generated_main,
            &plugins,
            updater_pubkey_pin(config).as_deref(),
            &compiler_fingerprint_input(&toolchain.version_output),
        )?
    };
    if let Some(fingerprint) = &fingerprint {
        if is_file(&binary)
            && fs::read_to_string(&fingerprint_path).is_ok_and(|stored| &stored == fingerprint)
        {
            debug("Tynd host is up to date; skipping cargo");
            return Ok(binary);
        }
    }

    remove_legacy_target_dir(&build_dir);
    seed_cargo_lock(&build_dir, &host_sources.root)?;
    write_generated_file(&build_dir.join("Cargo.toml"), cargo_manifest.as_bytes())?;
    write_generated_file(&build_dir.join("src/main.rs"), generated_main.as_bytes())?;

    let target_dir = shared_target_dir(&cache_root, &host_sources, &toolchain)?;
    if !quiet() && !binary.parent().is_some_and(Path::is_dir) {
        eprintln!("{}", first_build_notice(&target_dir));
    }
    // A failed build must not leave a fingerprint vouching for a stale binary.
    if fingerprint_path.exists() {
        fs::remove_file(&fingerprint_path)
            .with_context(|| format!("remove {}", fingerprint_path.display()))?;
    }
    let cargo = HostCargo {
        build_dir: &build_dir,
        target_dir: &target_dir,
        host_triple: &toolchain.host,
    };
    run_streaming(cargo_build_command(&cargo, config, release)).context("build the Tynd host")?;

    let built = built_host_binary(&cargo, release, &executable_name(&bin_name)?);
    if !is_file(&built) {
        bail!(
            "cargo build succeeded but the host binary is missing: {}",
            built.display()
        );
    }
    install_host_binary(&built, &binary, &build_dir)?;
    if let Some(fingerprint) = fingerprint {
        write_generated_file(&fingerprint_path, fingerprint.as_bytes())?;
    }
    Ok(binary)
}

/// Name of the cargo binary target: the app name plus a digest of the project
/// path. Projects share one target directory, where cargo writes every binary
/// to `<profile>/<bin name>`; two apps with the same directory name would
/// overwrite each other's binary there. Cargo's target-directory lock is
/// released when cargo exits, before the tooling can copy the binary, so the
/// lock cannot protect the copy: a name unique to the project does, without
/// any lock of our own.
fn host_bin_name(app_name: &str, project_root: &Path) -> String {
    let root = fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());
    let digest = blake3::hash(root.to_string_lossy().as_bytes()).to_hex();
    format!("{app_name}_{}", &digest[..8])
}

/// Build directories before the shared host cache kept a cargo target
/// directory in `.tynd/build/target`; nothing uses it now. Best effort.
fn remove_legacy_target_dir(build_dir: &Path) {
    let legacy = build_dir.join("target");
    if legacy.is_dir() {
        let _ = fs::remove_dir_all(&legacy);
    }
}

/// Where one cargo invocation for the app host runs and writes.
struct HostCargo<'a> {
    /// The generated workspace (`.tynd/build`), where cargo runs.
    build_dir: &'a Path,
    /// The shared target directory (see [`crate::project::host_cache`]).
    target_dir: &'a Path,
    /// The compiler's host triple, passed as `--target`.
    host_triple: &'a str,
}

/// The binary cargo writes with `--target <host triple>`:
/// `<target_dir>/<triple>/<profile>/<executable>`.
fn built_host_binary(cargo: &HostCargo<'_>, release: bool, executable: &str) -> PathBuf {
    cargo
        .target_dir
        .join(cargo.host_triple)
        .join(if release { "release" } else { "debug" })
        .join(executable)
}

/// The command `tynd profile --cpu` runs: `cargo flamegraph` against the
/// generated host workspace, with the target directory, host target and
/// environment of a normal build. Needs a previous `tynd dev` or `tynd build`.
pub(crate) fn host_flamegraph_command(
    cwd: &Path,
    config: &TyndConfig,
    output: &Path,
) -> Result<CommandSpec> {
    let cwd = absolute(cwd)?;
    let build_dir = cwd.join(".tynd/build");
    if !build_dir.join("Cargo.toml").is_file() {
        bail!(
            "native flamegraph needs the generated host workspace; run `tynd dev` or `tynd build` \
             first, or use `tynd profile --memory`/`--ipc`"
        );
    }
    let cache_root = cache_root()?;
    let host_sources = resolve_host_sources(&cwd, &cache_root)?;
    let toolchain = ensure_rust_toolchain(&build_dir)?;
    let target_dir = shared_target_dir(&cache_root, &host_sources, &toolchain)?;
    let cargo = HostCargo {
        build_dir: &build_dir,
        target_dir: &target_dir,
        host_triple: &toolchain.host,
    };
    Ok(host_cargo_command(
        vec![
            OsString::from("flamegraph"),
            OsString::from("--output"),
            output.as_os_str().to_owned(),
        ],
        &cargo,
        config,
    ))
}

/// The shared cargo target directory for the given sources and toolchain.
/// Editable sources (a checkout, a linked package) are keyed by their path.
fn shared_target_dir(
    cache_root: &Path,
    sources: &HostSources,
    toolchain: &RustToolchain,
) -> Result<PathBuf> {
    let sources = sources.version.as_ref().map_or_else(
        || {
            let root = fs::canonicalize(&sources.root).unwrap_or_else(|_| sources.root.clone());
            format!("path:{}", root.display())
        },
        |version| format!("{HOST_SOURCES_PACKAGE}@{version}"),
    );
    let key = host_cache_key(&sources, &toolchain.version_output, &toolchain.host);
    let target_dir = prepare_host_target_dir(cache_root, &key)?;
    if let Some(warning) = long_target_dir_warning(&target_dir) {
        warn(warning);
    }
    Ok(target_dir)
}

/// Copy the binary cargo built in the shared target directory to the project
/// (`destination`, under `build_dir`). The copy is written to a temporary
/// file in the destination directory and renamed over the previous binary,
/// so a concurrent reader sees the old or the new binary, never a partial one.
fn install_host_binary(built: &Path, destination: &Path, build_dir: &Path) -> Result<()> {
    let parent = destination.parent().context("host binary has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let build_real = fs::canonicalize(build_dir)?;
    if !fs::canonicalize(parent)?.starts_with(&build_real) {
        bail!("refusing host binary directory outside .tynd/build");
    }
    let mut source = fs::File::open(built).with_context(|| format!("open {}", built.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut source, temporary.as_file_mut())
        .with_context(|| format!("copy {} to {}", built.display(), parent.display()))?;
    temporary.as_file().sync_all()?;
    // Carries the executable bit on Unix.
    fs::set_permissions(temporary.path(), source.metadata()?.permissions())?;
    temporary.persist(destination).map_err(|error| {
        anyhow::Error::new(error.error).context(format!(
            "replace {} (is the app still running?)",
            destination.display()
        ))
    })?;
    Ok(())
}

/// Every host API the build enables: the core APIs (always), the config plan,
/// and the APIs native plugins need. Sorted, and each one must be a known host API.
fn collect_host_apis(
    plan_apis: Vec<String>,
    plugins: &[NativeBuildPlugin],
) -> Result<BTreeSet<String>> {
    let mut apis: BTreeSet<String> = CORE_HOST_APIS.iter().map(|api| (*api).to_owned()).collect();
    apis.extend(plan_apis);
    for plugin in plugins {
        apis.extend(plugin.required_apis.iter().cloned());
    }
    if let Some(api) = apis.iter().find(|api| !KNOWN_APIS.contains(&api.as_str())) {
        bail!("unknown host API \"{api}\"");
    }
    Ok(apis)
}

fn cargo_build_command(cargo: &HostCargo<'_>, config: &TyndConfig, release: bool) -> CommandSpec {
    let mut cargo_args = vec![OsString::from("build")];
    if release {
        cargo_args.push(OsString::from("--release"));
    }
    cargo_args.push(OsString::from("--target-dir"));
    cargo_args.push(cargo.target_dir.as_os_str().to_owned());
    host_cargo_command(cargo_args, cargo, config)
}

/// `cargo <cargo_args>` on the generated host workspace. The binary is copied
/// from `<target_dir>/<host triple>/<profile>`: the target directory and the
/// host target are pinned so `build.target` / `build.target-dir` in a user or
/// ancestor `.cargo/config.toml` cannot move it. The target directory is also
/// set through `CARGO_TARGET_DIR` for cargo subcommands (`cargo flamegraph`)
/// that take no `--target-dir`.
fn host_cargo_command(
    mut cargo_args: Vec<OsString>,
    cargo: &HostCargo<'_>,
    config: &TyndConfig,
) -> CommandSpec {
    cargo_args.push(OsString::from("--manifest-path"));
    cargo_args.push(cargo.build_dir.join("Cargo.toml").into_os_string());
    cargo_args.push(OsString::from("--target"));
    cargo_args.push(OsString::from(cargo.host_triple));

    let color = if std::env::var_os("NO_COLOR").is_some() {
        "never"
    } else {
        "always"
    };
    let command = CommandSpec::new("cargo")
        .args(cargo_args)
        .cwd(cargo.build_dir)
        .env("CARGO_TERM_COLOR", color)
        .env("CARGO_TARGET_DIR", cargo.target_dir)
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("CARGO_BUILD_TARGET_DIR");
    // Every trusted key (rotation), joined with "," - the host pins them all.
    let command = match macos_deployment_target(
        cargo.host_triple,
        std::env::var_os("MACOSX_DEPLOYMENT_TARGET").as_deref(),
    ) {
        Some(version) => command.env("MACOSX_DEPLOYMENT_TARGET", version),
        None => command,
    };
    match updater_pubkey_pin(config) {
        Some(pin) => command.env("TYND_UPDATER_PUBKEY", pin),
        None => command.env_remove("TYND_UPDATER_PUBKEY"),
    }
}

/// `MACOSX_DEPLOYMENT_TARGET` for a macOS host build: Tynd's minimum macOS,
/// unless the user already set one (`existing`). `None` leaves the
/// environment as is.
fn macos_deployment_target(host_triple: &str, existing: Option<&OsStr>) -> Option<&'static str> {
    (host_triple.ends_with("-apple-darwin") && existing.is_none_or(OsStr::is_empty))
        .then_some(tynd_bundle_contract::MACOS_MINIMUM_SYSTEM_VERSION)
}

fn prepare_custom_build_directory(cwd: &Path) -> Result<PathBuf> {
    let cwd_real =
        fs::canonicalize(cwd).with_context(|| format!("resolve project root {}", cwd.display()))?;
    let build_dir = cwd.join(".tynd/build");
    fs::create_dir_all(build_dir.join("src"))
        .with_context(|| format!("create {}", build_dir.display()))?;
    let build_real =
        fs::canonicalize(&build_dir).with_context(|| format!("resolve {}", build_dir.display()))?;
    if !build_real.starts_with(&cwd_real) {
        bail!("refusing custom build directory outside the project root");
    }
    let src_real = fs::canonicalize(build_dir.join("src"))?;
    if !src_real.starts_with(&build_real) {
        bail!("refusing generated source directory outside .tynd/build");
    }
    Ok(build_dir)
}

#[cfg(test)]
mod tests;

use std::{
    ffi::{OsStr, OsString},
    fmt::Write as _,
    fs,
    io::Write as _,
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    process::Child,
    sync::{
        atomic::{AtomicBool, Ordering},
        Once,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, ensure, Context, Result};

use crate::{
    args::{DevArgs, StartArgs},
    config::{self, LoadedConfig, TyndConfig},
    fsx::{absolute, atomic_write, portable_path},
    output::{step, verbose, warn},
    process::{self, CommandSpec},
};

use super::{
    doctor::apply_fixes,
    plugin_imports::warn_undeclared_plugin_imports,
    shared::{
        app_id_from_config, ensure_success, generated_path, is_link_or_reparse, project_name,
        resolve_bun_runtime, with_project_environment, BunRuntime,
    },
};

pub(in crate::commands) fn dev(args: &DevArgs) -> Result<()> {
    let root = config::find_project_root(&args.cwd)?;
    if !args.no_fix && (args.yes || development_setup_incomplete(&root)) {
        apply_fixes(&root, false)?;
    }
    let mut loaded = config::load_typed(&root, None)?;
    warn_undeclared_plugin_imports(&loaded);
    let frontend = crate::project::detect_frontend(&loaded.root)?;
    if let Some(blocked) = &frontend.blocked_by {
        bail!("{blocked} is server-rendered; Tynd requires a static SPA")
    }
    let host = crate::project::build_custom_host(&loaded.root, &loaded.config, false)?;
    let bun = resolve_bun_runtime(&loaded.root)?;
    let backend = resolve_backend_entry(&loaded)?;

    install_interrupt_handler();
    let dev_url = loaded.config.dev_url.clone().or(frontend.dev_url.clone());
    let mut frontend_child = start_frontend_dev(&loaded, &frontend, dev_url.as_deref())?;
    let result = (|| -> Result<()> {
        if let Some(url) = dev_url.as_deref() {
            wait_for_frontend(url, &mut frontend_child, Duration::from_secs(30))?;
        }
        let debug_socket = resolve_debug_socket(
            args.no_debug,
            std::env::var_os("TYND_DEBUG_SOCKET"),
            &debug_socket_app_id(&loaded.config, &loaded.root),
        );
        let mut spec = host_spec(&host, &backend, &loaded.config, &bun, true)?
            .env("TYND_FRONTEND_DIR", loaded.config.frontend_dir.as_os_str())
            .piped_stdin();
        spec = with_project_environment(spec, &loaded);
        if let Some(url) = dev_url {
            spec = spec.env("TYND_DEV_URL", url);
        }
        if let Some(socket) = &debug_socket {
            spec = spec.env("TYND_DEBUG_SOCKET", socket.path.clone());
        } else {
            // `--no-debug`: do not let an inherited socket request enable the debug server.
            spec = spec.env_remove("TYND_DEBUG_SOCKET");
        }
        if args.inspect || args.inspect_brk {
            spec = spec.env("BUN_INSPECT", "1");
        }
        if args.inspect_brk {
            spec = spec.env("BUN_INSPECT_WAIT", "1");
        }

        step(format!("host: {}", host.display()));
        let mut host_child = process::spawn_spec(spec)?;
        let result = supervise_development(&mut host_child, &mut frontend_child, &mut loaded);
        terminate_child(&mut host_child);
        result
    })();
    terminate_optional(&mut frontend_child);
    result
}

pub(in crate::commands) fn start(args: &StartArgs) -> Result<()> {
    let loaded = config::load_typed(&absolute(&args.cwd)?, None)?;
    let frontend = crate::project::detect_frontend(&loaded.root)?;
    let backend = cached_backend_bundle(&loaded.root)?;
    let frontend_dir = cached_frontend_assets(&loaded, &frontend)?;
    let host = crate::project::resolve_host_binary(&loaded.root, true)?;
    let bun = resolve_bun_runtime(&loaded.root)?;
    let spec = with_project_environment(
        host_spec(&host, &backend, &loaded.config, &bun, false)?
            .env("TYND_FRONTEND_DIR", frontend_dir.as_os_str()),
        &loaded,
    );
    let status = process::run_streaming(spec)?;
    ensure_success(status, "Tynd host")
}

fn cached_backend_bundle(root: &Path) -> Result<PathBuf> {
    let candidate = generated_path(root, ".tynd/cache/bundle.dist.js")?;
    ensure!(
        candidate.is_file(),
        "cached backend bundle not found; run `tynd build` first"
    );
    Ok(candidate)
}

fn cached_frontend_assets(
    loaded: &LoadedConfig,
    frontend: &crate::project::FrontendInfo,
) -> Result<PathBuf> {
    if loaded.config.frontend_dir.is_dir() {
        return Ok(loaded.config.frontend_dir.clone());
    }
    let detected = loaded.root.join(&frontend.out_dir);
    ensure!(
        detected.is_dir(),
        "cached frontend output not found: {}; run `tynd build` first",
        detected.display()
    );
    Ok(detected)
}

pub(super) fn resolve_backend_entry(loaded: &LoadedConfig) -> Result<PathBuf> {
    ensure!(
        loaded.config.backend.is_file(),
        "backend not found: {}",
        loaded.config.backend.display()
    );
    let generated = generated_path(&loaded.root, ".tynd/cache/generated")?;
    fs::create_dir_all(&generated)?;
    let bootstrap_path = generated.join("bootstrap.ts");
    let entry_path = generated.join("backend-entry.ts");
    let plugins = config::resolve_backend_plugins(&loaded.config, &loaded.root)?;

    let updater = config::resolve_updater_config(&loaded.config)?;
    let version = crate::project::load_package_json(&loaded.root)?
        .and_then(|package| package.version)
        .unwrap_or_else(|| "0.0.0".into());
    let network = config::resolve_updater_network(&loaded.config, |name| {
        std::env::var(name)
            .ok()
            .or_else(|| loaded.environment.get(name).cloned())
    })?;
    let bootstrap =
        backend_bootstrap_source(&plugins, updater.as_ref(), network.as_ref(), &version)?;
    atomic_write(&bootstrap_path, bootstrap)?;

    let backend_specifier = backend_import_specifier(
        path_utf8(&generated)?,
        path_utf8(&fs::canonicalize(&loaded.config.backend)?)?,
    );
    let entry = format!(
        "import \"./bootstrap.ts\";\nexport * from {};\n",
        serde_json::to_string(&backend_specifier)?
    );
    atomic_write(&entry_path, entry)?;
    Ok(entry_path)
}

/// Source of the generated `bootstrap.ts` imported ahead of the backend. It
/// carries the app version (reported to the host through `app.setInfo`) and the
/// resolved updater config (public keys, endpoints, channel; passed to
/// `updater.configure`). The Bun backend forwards both to the host at startup,
/// which is the only way a packaged app gets a trusted updater. `network` (the
/// updater `proxy` / `headers`, possibly credentials) is merged into that same
/// object and goes nowhere else: the renderer-visible runtime config never
/// carries it.
fn backend_bootstrap_source(
    plugins: &[String],
    updater: Option<&config::ResolvedUpdaterConfig>,
    network: Option<&config::ResolvedUpdaterNetwork>,
    version: &str,
) -> Result<String> {
    let mut bootstrap = String::new();
    if !plugins.is_empty() {
        // `registerPlugin` has one home, the backend entry `@tynd/core` (marked
        // internal there); the isomorphic `@tynd/core/plugin` does not export it.
        bootstrap.push_str(
            "import { registerPlugin as __tyndRegisterPlugin } from \"@tynd/core\";
",
        );
        for (index, package) in plugins.iter().enumerate() {
            writeln!(
                &mut bootstrap,
                "import __tyndPlugin{index} from {};",
                serde_json::to_string(&format!("{package}/backend"))?
            )?;
        }
    }
    // `Object.assign` sets the globals without an `as any` cast, so the
    // generated file stays clean under a user's `biome lint` / `tsc`.
    bootstrap.push_str(
        "Object.assign(globalThis, { __TYND_RUNTIME__: \"bun\" });
",
    );
    writeln!(
        &mut bootstrap,
        "Object.assign(globalThis, {{ __TYND_APP_INFO__: {} }});",
        serde_json::to_string(&serde_json::json!({ "version": version }))?
    )?;
    if let Some(updater) = updater {
        let mut value = serde_json::to_value(updater)?;
        if let (Some(network), Some(object)) = (network, value.as_object_mut()) {
            if let serde_json::Value::Object(extra) = serde_json::to_value(network)? {
                object.extend(extra);
            }
        }
        writeln!(
            &mut bootstrap,
            "Object.assign(globalThis, {{ __TYND_UPDATER_CONFIG__: {} }});",
            serde_json::to_string(&value)?
        )?;
    }
    for index in 0..plugins.len() {
        writeln!(&mut bootstrap, "__tyndRegisterPlugin(__tyndPlugin{index});")?;
    }
    Ok(bootstrap)
}

fn path_utf8(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

/// Import specifier for `target` as written in a module located in `from_dir`.
///
/// Bun's bundler does not resolve `file://` URLs, so this emits a relative path
/// with forward slashes (`./x` or `../x`), which Bun resolves the same way on
/// every OS, in `bun run` (dev) and in `Bun.build` (build). When no relative
/// path exists (another Windows drive or UNC share) it falls back to the
/// absolute path with forward slashes. Characters are kept verbatim: spaces and
/// non-ASCII names are valid in a plain path specifier.
fn backend_import_specifier(from_dir: &str, target: &str) -> String {
    let from = portable_path(from_dir);
    let target = portable_path(target);
    let from_parts: Vec<&str> = from.split('/').filter(|part| !part.is_empty()).collect();
    let target_parts: Vec<&str> = target.split('/').filter(|part| !part.is_empty()).collect();
    let unc = target.starts_with("//");
    let drive = target_parts
        .first()
        .is_some_and(|first| first.len() == 2 && first.ends_with(':'));
    // Windows paths compare case-insensitively; the root (drive, or UNC server
    // and share) must be shared for a relative path to exist.
    let windows = unc || drive;
    let root_len = if unc { 2 } else { usize::from(drive) };
    let common = from_parts
        .iter()
        .zip(&target_parts)
        .take_while(|(left, right)| {
            if windows {
                left.eq_ignore_ascii_case(right)
            } else {
                left == right
            }
        })
        .count();
    if from.starts_with("//") != unc || common < root_len || common == target_parts.len() {
        return target;
    }
    let mut specifier = if common == from_parts.len() {
        "./".to_owned()
    } else {
        "../".repeat(from_parts.len() - common)
    };
    specifier.push_str(&target_parts[common..].join("/"));
    specifier
}

fn start_frontend_dev(
    loaded: &LoadedConfig,
    detected: &crate::project::FrontendInfo,
    dev_url: Option<&str>,
) -> Result<Option<Child>> {
    let (command, detected_command) = if let Some(arguments) = &loaded.config.dev_command_args {
        (
            arguments
                .split_first()
                .map(|(program, args)| crate::project::CommandPlan::new(program, args.to_vec())),
            false,
        )
    } else if let Some(command) = &loaded.config.dev_command {
        let parts = parse_command_line(command)?;
        (
            parts
                .split_first()
                .map(|(program, args)| crate::project::CommandPlan::new(program, args.to_vec())),
            false,
        )
    } else {
        (detected.dev_command.clone(), true)
    };
    command
        .map(|mut command| {
            if detected_command
                && detected.build_tool == crate::project::BuildTool::Vite
                && !command
                    .args
                    .iter()
                    .any(|argument| argument == "--strictPort")
            {
                command.args.push("--strictPort".to_owned());
            }
            if let Some(url) = dev_url {
                ensure_dev_url_unserved(url)?;
            }
            process::spawn_spec(with_project_environment(
                CommandSpec::new(command.program)
                    .args(command.args)
                    .cwd(&loaded.root),
                loaded,
            ))
        })
        .transpose()
}

fn parse_command_line(input: &str) -> Result<Vec<String>> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut argument_started = false;
    let mut characters = input.chars().peekable();

    while let Some(character) = characters.next() {
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else if character == '\\' && delimiter == '"' && characters.peek() == Some(&'"') {
                current.push(characters.next().expect("peeked quote exists"));
            } else {
                current.push(character);
            }
            continue;
        }

        if matches!(character, '\'' | '"') {
            quote = Some(character);
            argument_started = true;
        } else if character == '\\' {
            if characters
                .peek()
                .is_some_and(|next| next.is_whitespace() || matches!(next, '\'' | '"'))
            {
                current.push(characters.next().expect("peeked escaped character exists"));
            } else {
                current.push(character);
            }
            argument_started = true;
        } else if character.is_whitespace() {
            if argument_started {
                arguments.push(std::mem::take(&mut current));
                argument_started = false;
            }
        } else {
            current.push(character);
            argument_started = true;
        }
    }

    ensure!(quote.is_none(), "devCommand contains an unterminated quote");
    if argument_started {
        arguments.push(current);
    }
    ensure!(!arguments.is_empty(), "devCommand must not be empty");
    Ok(arguments)
}

/// Fails when something already accepts connections on the dev URL before the
/// project's own dev server is spawned. `localhost` resolves to both `::1` and
/// `127.0.0.1`: a foreign server on one family does not stop the new dev server
/// from binding the other (even with `--strictPort`), and the webview would
/// then silently load the foreign page (often a blank one) instead.
fn ensure_dev_url_unserved(url: &str) -> Result<()> {
    for address in frontend_socket_addresses(url)? {
        ensure!(
            TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_err(),
            "{url} is already served by another process ({address}); the app window would load \
             that page instead of this project's dev server. Stop it or free the port (set \
             `devUrl` and `devCommand` to a free one), or set `devCommandArgs: []` in \
             tynd.config.ts if you start the dev server yourself"
        );
    }
    Ok(())
}

fn wait_for_frontend(url: &str, child: &mut Option<Child>, timeout: Duration) -> Result<()> {
    let addresses = frontend_socket_addresses(url)?;
    let deadline = Instant::now() + timeout;
    loop {
        ensure!(
            !interrupted(),
            "interrupted before the frontend became ready"
        );
        if let Some(child) = child.as_mut() {
            if let Some(status) = child.try_wait()? {
                bail!(
                    "frontend development server exited before becoming ready (code {})",
                    status.code().unwrap_or(-1)
                );
            }
        }
        for address in &addresses {
            if TcpStream::connect_timeout(address, Duration::from_millis(250)).is_ok() {
                return Ok(());
            }
        }
        ensure!(
            Instant::now() < deadline,
            "frontend development server did not become reachable at {url} within {} seconds",
            timeout.as_secs()
        );
        thread::sleep(Duration::from_millis(100));
    }
}

fn frontend_socket_addresses(url: &str) -> Result<Vec<std::net::SocketAddr>> {
    let (scheme, remainder) = url
        .split_once("://")
        .with_context(|| format!("dev URL must include http:// or https://: {url}"))?;
    ensure!(
        matches!(scheme, "http" | "https"),
        "unsupported dev URL scheme {scheme:?}"
    );
    let authority = remainder.split('/').next().unwrap_or_default();
    ensure!(
        !authority.is_empty() && !authority.contains('@'),
        "dev URL has an invalid authority"
    );
    let default_port = if scheme == "https" { 443 } else { 80 };
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let close = rest.find(']').context("invalid bracketed IPv6 dev URL")?;
        let host = &rest[..close];
        let suffix = &rest[close + 1..];
        let port = if suffix.is_empty() {
            default_port
        } else {
            suffix
                .strip_prefix(':')
                .context("invalid IPv6 dev URL port")?
                .parse::<u16>()?
        };
        (host, port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        (host, port.parse::<u16>()?)
    } else {
        (authority, default_port)
    };
    let addresses = (host, port).to_socket_addrs()?.collect::<Vec<_>>();
    ensure!(
        !addresses.is_empty(),
        "dev URL host did not resolve: {host}"
    );
    Ok(addresses)
}

fn development_setup_incomplete(root: &Path) -> bool {
    config::find_config_path(root).is_none()
        || !root.join("package.json").is_file()
        || !root.join("tsconfig.json").is_file()
}

fn supervise_development(
    host: &mut Child,
    frontend: &mut Option<Child>,
    loaded: &mut LoadedConfig,
) -> Result<()> {
    let mut digest = development_digest(loaded)?;
    loop {
        if interrupted() {
            step("interrupted, stopping the host and the frontend");
            return Ok(());
        }
        if let Some(status) = host.try_wait()? {
            return ensure_success(status, "Tynd host");
        }
        if let Some(frontend) = frontend.as_mut() {
            if let Some(status) = frontend.try_wait()? {
                bail!(
                    "frontend development server exited with code {}",
                    status.code().unwrap_or(-1)
                );
            }
        }
        thread::sleep(Duration::from_millis(250));
        let next = development_digest(loaded)?;
        if next == digest {
            continue;
        }
        thread::sleep(Duration::from_millis(120));
        match config::load_typed(&loaded.root, None) {
            Ok(fresh) => {
                ensure!(
                    serde_json::to_value(&fresh.config)? == serde_json::to_value(&loaded.config)?,
                    "Tynd configuration changed; restart `tynd dev` so native capabilities, environment and host arguments are rebuilt"
                );
                resolve_backend_entry(&fresh)?;
                let stdin = host
                    .stdin
                    .as_mut()
                    .context("Tynd host reload channel closed")?;
                stdin.write_all(b"reload\n")?;
                stdin.flush()?;
                *loaded = fresh;
                digest = development_digest(loaded)?;
                step("backend reloaded");
            },
            Err(error) => {
                warn(format!(
                    "reload deferred until configuration is valid: {error:#}"
                ));
                digest = next;
            },
        }
    }
}

fn development_digest(loaded: &LoadedConfig) -> Result<blake3::Hash> {
    let mut hasher = blake3::Hasher::new();
    let mut roots = Vec::new();
    if let Some(parent) = loaded.config.backend.parent() {
        roots.push(parent.to_path_buf());
    }
    roots.extend(loaded.sources.iter().cloned());
    if let Ok(entries) = fs::read_dir(&loaded.root) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name == ".env" || name.starts_with(".env."))
                && entry.path().is_file()
            {
                roots.push(entry.path());
            }
        }
    }
    for package in config::resolve_backend_plugins(&loaded.config, &loaded.root)? {
        if let Some(path) = crate::project::resolve_package_dir(&package, &loaded.root)? {
            roots.push(path);
        }
    }
    roots.sort();
    roots.dedup();
    for root in roots {
        if root.is_file() {
            hash_development_file(&root, &mut hasher)?;
            continue;
        }
        if !root.is_dir() {
            continue;
        }
        // A root is a declared source (backend, sources, plugin package). It may
        // itself be a linked package (`link:` / workspace junction); resolve the
        // root only — links inside it are still refused below.
        let root = fs::canonicalize(&root)
            .with_context(|| format!("resolve development source {}", root.display()))?;
        let walker = walkdir::WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                !matches!(
                    entry.file_name().to_str(),
                    Some("node_modules" | ".tynd" | "target" | "release" | ".git")
                )
            });
        for entry in walker {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            ensure!(
                !is_link_or_reparse(&metadata),
                "refusing linked development source {}",
                entry.path().display()
            );
            if entry.file_type().is_file() && development_source_file(entry.path()) {
                hash_development_file(entry.path(), &mut hasher)?;
            }
        }
    }
    Ok(hasher.finalize())
}

fn development_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ts" | "mts"
                    | "cts"
                    | "tsx"
                    | "js"
                    | "mjs"
                    | "cjs"
                    | "jsx"
                    | "json"
                    | "jsonc"
                    | "toml"
                    | "yaml"
                    | "yml"
            )
        })
}

fn hash_development_file(path: &Path, hasher: &mut blake3::Hasher) -> Result<()> {
    let metadata = fs::metadata(path)?;
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(&metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            hasher.update(&duration.as_nanos().to_le_bytes());
        }
    }
    Ok(())
}

fn host_spec(
    host: &Path,
    backend: &Path,
    config: &TyndConfig,
    bun: &BunRuntime,
    debug: bool,
) -> Result<CommandSpec> {
    let mut arguments = vec![
        OsString::from("--backend-entry"),
        backend.as_os_str().to_os_string(),
    ];
    if debug {
        arguments.push(OsString::from("--debug"));
    }
    arguments.extend(binary_args_with_separator(&config.binary_args));
    let mut spec = CommandSpec::new(host.as_os_str())
        .args(arguments)
        .env("TYND_BUN_PATH", bun.path.as_os_str())
        .env("TYND_ENTRY", backend.as_os_str());
    if verbose() {
        spec = spec.env("TYND_LOG", "debug");
    }
    spec = spec.env(
        "TYND_FRONTEND_RUNTIME_CONFIG",
        serde_json::to_string(&config::resolve_updater_config(config)?)?,
    );
    // Unpacked runs (`dev`, `start`) have no pack entry, so the runtime reads the
    // telemetry config from the environment; packaged apps ignore it.
    if let Some(json) = config::resolve_observability_json(config)? {
        spec = spec.env("TYND_OBSERVABILITY", json);
    }
    // Same for the app identifier, which names the app's own directories; a
    // packaged app reads it from its `app.identifier` pack entry instead.
    if let Some(identifier) = app_id_from_config(config) {
        spec = spec.env("TYND_APP_IDENTIFIER", identifier);
    }
    Ok(spec)
}

/// `binaryArgs` belong to the app: the runtime parser only forwards what
/// follows a `--` separator, so they never collide with its own flags.
fn binary_args_with_separator(binary_args: &[String]) -> Vec<OsString> {
    if binary_args.is_empty() {
        return Vec::new();
    }
    std::iter::once(OsString::from("--"))
        .chain(binary_args.iter().map(OsString::from))
        .collect()
}

fn terminate_optional(child: &mut Option<Child>) {
    if let Some(child) = child {
        terminate_child(child);
    }
}

fn terminate_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Turn SIGINT/SIGTERM (and Ctrl+C / console close on Windows) into a flag the supervisor polls,
/// so `tynd dev` tears its host and frontend children down instead of orphaning them.
fn install_interrupt_handler() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        if let Err(error) = ctrlc::set_handler(|| INTERRUPTED.store(true, Ordering::Release)) {
            warn(format!("signal handler not installed: {error}"));
        }
    });
}

fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::Acquire)
}

/// Debug socket the host serves on, plus the private directory (Unix) that holds it when `tynd dev`
/// generated the endpoint itself. The directory is removed on drop, after the host has exited.
struct DebugSocket {
    path: OsString,
    private_dir: Option<PathBuf>,
}

impl Drop for DebugSocket {
    fn drop(&mut self) {
        if let Some(dir) = self.private_dir.take() {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

/// Name the dev debug socket is derived from: `bundle.identifier`, else the
/// project directory, so it identifies the app rather than a placeholder.
fn debug_socket_app_id(config: &TyndConfig, root: &Path) -> String {
    app_id_from_config(config).unwrap_or_else(|| project_name(root))
}

/// Socket the host should serve its debug protocol on. A `TYND_DEBUG_SOCKET` already present in the
/// environment is kept so a caller (`@tynd/test`) can find the host by that exact name. The host
/// publishes the endpoint it actually bound in its discovery file, which is what clients read, so the
/// generated name does not need to be predictable.
fn resolve_debug_socket(
    no_debug: bool,
    preset: Option<OsString>,
    app_id: &str,
) -> Option<DebugSocket> {
    if no_debug {
        return None;
    }
    if let Some(path) = preset.filter(|value| !value.is_empty()) {
        return Some(DebugSocket {
            path,
            private_dir: None,
        });
    }
    match generated_debug_socket(app_id) {
        Ok(socket) => Some(socket),
        Err(error) => {
            warn(format!("debug socket disabled: {error:#}"));
            None
        },
    }
}

fn generated_debug_socket(app_id: &str) -> Result<DebugSocket> {
    let safe = app_id
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || matches!(value, '.' | '_' | '-') {
                value
            } else {
                '_'
            }
        })
        .take(48)
        .collect::<String>();
    if cfg!(windows) {
        // Pipe names are enumerable by every local account: an unguessable suffix keeps another
        // user from squatting the name before the host binds it (the host also applies an
        // owner-only DACL).
        let mut random = [0u8; 8];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("random pipe suffix: {error}"))?;
        let suffix = random.iter().fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        });
        Ok(DebugSocket {
            path: OsString::from(format!(
                r"\\.\pipe\tynd-{safe}-{}-{suffix}",
                std::process::id()
            )),
            private_dir: None,
        })
    } else {
        let dir = private_socket_dir()?;
        let path = dir.join("debug.sock").into_os_string();
        Ok(DebugSocket {
            path,
            private_dir: Some(dir),
        })
    }
}

/// Fresh, randomly named directory owned by the current user with no group/other access, under
/// `$XDG_RUNTIME_DIR` when set (already per-user and `0700`) and the temp directory otherwise. A
/// `mkdir` that succeeds proves we created it, so nobody can have pre-created the name or planted a
/// symlink; the mode is still verified because a umask or an odd filesystem could loosen it.
fn private_socket_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute() && dir.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let dir = tempfile::Builder::new()
        .prefix("tynd-dev-")
        .tempdir_in(&base)
        .with_context(|| format!("create a private directory in {}", base.display()))?
        .keep();
    if let Err(error) = verify_private_dir(&dir) {
        let _ = fs::remove_dir_all(&dir);
        return Err(error);
    }
    Ok(dir)
}

#[cfg(unix)]
fn verify_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let metadata = fs::symlink_metadata(dir).with_context(|| format!("stat {}", dir.display()))?;
    ensure!(metadata.is_dir(), "{} is not a directory", dir.display());
    let mode = metadata.permissions().mode();
    ensure!(
        // No group/other bits set (low six bits of the mode are zero).
        mode.trailing_zeros() >= 6,
        "{} must not be accessible by group/other (mode {:o})",
        dir.display(),
        mode & 0o777
    );
    Ok(())
}

#[cfg(not(unix))]
fn verify_private_dir(_dir: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;

use std::{ffi::OsStr, fs, path::Path};

use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Map, Value};

use crate::{
    args::{CreateArgs, InitArgs},
    config,
    fsx::{absolute, atomic_write, read_json, write_json_file},
    output::{step, success, warn},
    process::{self, CommandSpec},
    VERSION,
};

use super::shared::{
    catalog_add_name, ensure_line_block, ensure_success, is_link_or_reparse, normalize_capability,
    project_name, resolve_bun_runtime, valid_project_name,
};
use super::tsconfig::ensure_typescript_dx;

const PROJECT_NAME_PATTERN: &str = "letters, numbers, dot, underscore and dash";
const DEFAULT_FRAMEWORK: &str = "react";
const FRAMEWORKS: &[(&str, Option<&str>)] = &[
    ("react", Some("react-ts")),
    ("vue", Some("vue-ts")),
    ("svelte", Some("svelte-ts")),
    ("solid", Some("solid-ts")),
    ("preact", Some("preact-ts")),
    ("lit", Some("lit-ts")),
    ("angular", None),
];

pub(in crate::commands) fn create(args: &CreateArgs) -> Result<()> {
    let name = args.name.as_str();
    ensure!(
        valid_project_name(name),
        "invalid project name {name:?}; use {PROJECT_NAME_PATTERN}"
    );
    let requested_framework = args.framework.as_deref().unwrap_or(DEFAULT_FRAMEWORK);
    let (framework, vite_template) = resolve_framework(requested_framework).with_context(|| {
        format!(
            "unknown framework {requested_framework:?}; expected one of: {} (a `-ts` suffix is accepted)",
            framework_names()
        )
    })?;

    let parent = std::env::current_dir()?;
    let target = parent.join(name);
    if target.exists() {
        ensure!(
            args.force,
            "{} already exists; pass --force to replace it",
            target.display()
        );
        ensure!(
            target.parent() == Some(parent.as_path()) && target != parent,
            "unsafe project path"
        );
        let metadata = fs::symlink_metadata(&target)?;
        ensure!(
            metadata.is_dir() && !is_link_or_reparse(&metadata),
            "refusing to replace linked or non-directory project target {}",
            target.display()
        );
        fs::remove_dir_all(&target).with_context(|| format!("remove {}", target.display()))?;
    }

    if let Some(template) = &args.template {
        run_streaming(
            "bunx",
            [
                "giget",
                template.as_str(),
                target.to_string_lossy().as_ref(),
                "--force",
            ],
            &parent,
        )?;
    } else if framework == "angular" {
        run_streaming(
            "bunx",
            [
                "@angular/cli@latest",
                "new",
                name,
                "--defaults",
                "--skip-git",
                "--skip-install",
                "--ssr=false",
            ],
            &parent,
        )?;
    } else {
        let template = vite_template
            .with_context(|| format!("framework {framework} has no starter template"))?;
        run_streaming(
            "bun",
            ["create", "vite@latest", name, "--template", template],
            &parent,
        )?;
    }

    init_at_root(&target, false)?;
    if !args.no_git {
        if let Err(error) = run_streaming("git", ["init", "--quiet"], &target) {
            warn(format!("git init skipped: {error:#}"));
        }
    }
    if !args.no_install {
        if let Err(error) = run_streaming("bun", ["install"], &target) {
            bail!(
                "dependency install failed: {error:#}
{name} was scaffolded in {} but its dependencies are missing; fix the error above, then run `cd {name} && bun install`",
                target.display()
            );
        }
    }
    warn_if_rust_missing(&target);
    success(format!("{name} is ready"));
    step(next_steps(name, !args.no_install));
    Ok(())
}

/// Maps a `--framework` value (optionally suffixed with `-ts`, since every
/// starter already uses TypeScript) to its canonical name and vite template.
fn resolve_framework(input: &str) -> Option<(&'static str, Option<&'static str>)> {
    let base = input.strip_suffix("-ts").unwrap_or(input);
    FRAMEWORKS.iter().find(|(value, _)| *value == base).copied()
}

fn framework_names() -> String {
    FRAMEWORKS
        .iter()
        .map(|(value, _)| *value)
        .collect::<Vec<_>>()
        .join(", ")
}

fn next_steps(name: &str, installed: bool) -> String {
    let install = if installed {
        ""
    } else {
        "
    bun install"
    };
    format!(
        "Next steps:
    cd {name}{install}
    tynd dev      # run the app with hot reload (the first run compiles the native host)
    tynd build    # produce a release binary"
    )
}

pub(in crate::commands) fn init(args: &InitArgs) -> Result<()> {
    let requested = absolute(&args.cwd)?;
    let root = config::find_project_root(&requested)?;
    init_at_root(&root, args.force)?;
    warn_if_rust_missing(&root);
    Ok(())
}

/// Scaffolding works without Rust, but `tynd dev` and `tynd build` compile the
/// app host with it: warn now rather than on the first run.
fn warn_if_rust_missing(root: &Path) {
    if let Err(error) = crate::project::ensure_rust_toolchain(root) {
        warn(format!(
            "{error:#}
`tynd dev` and `tynd build` need it; run `tynd doctor` to check the setup"
        ));
    }
}

fn init_at_root(root: &Path, force: bool) -> Result<()> {
    let existing_config = config::find_config_in_directory(root);
    let package_path = root.join("package.json");
    let mut package = if package_path.is_file() {
        read_json(&package_path)?
    } else {
        json!({ "name": project_name(root), "version": "0.0.0", "private": true })
    };
    let name = package
        .get("name")
        .and_then(Value::as_str)
        .map_or_else(|| project_name(root), str::to_owned);
    let frontend = detect_frontend(root);
    let frontend_dir = frontend
        .output
        .clone()
        .unwrap_or_else(|| "frontend".to_owned());
    // Tynd needs a frontend build tool whose output it serves from
    // `frontendDir`. Checked before anything (even the root directory) is
    // created. A project that already has a Tynd config keeps its own
    // `frontendDir`, so `fix`/`doctor --fix`/`add` are not blocked.
    ensure!(
        frontend.tool.is_some() || existing_config.is_some(),
        "no frontend found in {}; scaffold one first (e.g. `bun create vite@latest`) or start fresh with `tynd create <name> --framework {DEFAULT_FRAMEWORK}`",
        root.display()
    );
    fs::create_dir_all(root)?;

    // A config in another format (e.g. tynd.config.json) is left untouched.
    let default_config = root.join("tynd.config.ts");
    match existing_config {
        Some(existing) if existing != default_config => {},
        _ => write_scaffold(&default_config, render_config(&name, &frontend_dir), force)?,
    }
    write_scaffold(
        &root.join("backend/main.ts"),
        render_backend(&name, &frontend_dir),
        force,
    )?;

    patch_package(&mut package)?;
    write_json_file(&package_path, &package)?;
    ensure_line_block(
        &root.join(".gitignore"),
        &[
            ".tynd/",
            "release/",
            "node_modules/",
            "dist/",
            ".env",
            ".env.local",
            "*.key",
        ],
    )?;
    pin_bun_version(root)?;
    ensure_typescript_dx(root)?;
    success("Tynd project initialized");
    Ok(())
}

/// Writes `.bun-version` with the resolved runtime's version unless a pin
/// already exists; an uninspectable runtime only warns.
fn pin_bun_version(root: &Path) -> Result<()> {
    let pin = root.join(".bun-version");
    if pin.exists() {
        return Ok(());
    }
    match resolve_bun_runtime(root) {
        Ok(bun) => atomic_write(&pin, format!("{}\n", bun.version)),
        Err(error) => {
            warn(format!(
                "Bun version pin not created because the runtime could not be inspected: {error:#}"
            ));
            Ok(())
        },
    }
}

fn patch_package(package: &mut Value) -> Result<()> {
    let object = package
        .as_object_mut()
        .context("package.json must contain an object")?;
    let scripts = object
        .entry("scripts")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("package.json scripts must be an object")?;
    park_script(scripts, "dev", "dev:ui", "tynd dev");
    park_script(scripts, "build", "build:ui", "tynd build");
    scripts.insert("dev".into(), Value::String("tynd dev".into()));
    scripts.insert("start".into(), Value::String("tynd start".into()));
    scripts.insert("build".into(), Value::String("tynd build".into()));

    let dependencies = object
        .entry("dependencies")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("dependencies must be an object")?;
    dependencies
        .entry("@tynd/core")
        .or_insert_with(|| Value::String(format!("^{VERSION}")));
    let host_sources_is_runtime_dependency =
        dependencies.contains_key(crate::project::HOST_SOURCES_PACKAGE);
    let cli_is_runtime_dependency = dependencies.contains_key("@tynd/cli");
    let bun_types_is_runtime_dependency = dependencies.contains_key(BUN_TYPES_PACKAGE);
    let dev_dependencies = object
        .entry("devDependencies")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("devDependencies must be an object")?;
    if !cli_is_runtime_dependency {
        dev_dependencies
            .entry("@tynd/cli")
            .or_insert_with(|| Value::String(format!("^{VERSION}")));
    }
    // Rust sources `tynd dev` and `tynd build` compile the app host from.
    if !host_sources_is_runtime_dependency {
        dev_dependencies
            .entry(crate::project::HOST_SOURCES_PACKAGE)
            .or_insert_with(|| Value::String(format!("^{VERSION}")));
    }
    // The backend runs on Bun; its tsconfig references the `bun` types.
    if !bun_types_is_runtime_dependency {
        dev_dependencies
            .entry(BUN_TYPES_PACKAGE)
            .or_insert_with(|| Value::String(BUN_TYPES_VERSION.to_owned()));
    }
    Ok(())
}

fn park_script(
    scripts: &mut Map<String, Value>,
    source: &str,
    destination: &str,
    replacement: &str,
) {
    if let Some(existing) = scripts.get(source).and_then(Value::as_str) {
        if existing != replacement && !scripts.contains_key(destination) {
            scripts.insert(destination.to_owned(), Value::String(existing.to_owned()));
        }
    }
}

fn write_scaffold(path: &Path, content: String, force: bool) -> Result<()> {
    if path.exists() && !force {
        return Ok(());
    }
    atomic_write(path, content)
}

/// Whether `input` is a host API or a `tynd add` name from plugins/catalog.json,
/// rather than an arbitrary npm plugin package.
pub(in crate::commands) fn is_capability_alias(input: &str) -> bool {
    normalize_capability(input).is_some() || catalog_add_name(input).is_some()
}

/// Plugin package backing a capability alias or catalog plugin name, if any.
pub(in crate::commands) fn capability_plugin_package(input: &str) -> Option<&'static str> {
    if let Some(api) = normalize_capability(input) {
        return config::API_PLUGIN_PACKAGES
            .iter()
            .find(|(candidate, _)| *candidate == api)
            .map(|(_, package)| *package);
    }
    catalog_add_name(input).map(|(package, _)| package)
}

/// Enables the host API named by `input` (creating the config when missing). A
/// catalog plugin without a host API of its own only needs the config to exist:
/// the caller declares its package. Returns the API when the `apis` edit
/// added it; the caller reports it once the whole `tynd add` has succeeded.
pub(in crate::commands) fn add_capability(cwd: &Path, input: &str) -> Result<Option<&'static str>> {
    let api = normalize_capability(input);
    ensure!(
        api.is_some() || catalog_add_name(input).is_some(),
        "unknown capability {input:?}"
    );
    let requested = absolute(cwd)?;
    if config::find_config_path(&requested).is_none() {
        init(&InitArgs {
            cwd: requested.clone(),
            force: false,
        })?;
    }
    let Some(api) = api else {
        return Ok(None);
    };
    let editable = config::resolve_editable_config(&requested)?;
    let source = fs::read_to_string(&editable.path)
        .with_context(|| format!("read {}", editable.path.display()))?;
    let next =
        config::mutate_config_source(&editable.path, &source, config::ConfigMutation::AddApi(api))?;
    if next == source {
        success(format!("capability {api} already configured"));
        return Ok(None);
    }
    atomic_write(&editable.path, next)?;
    Ok(Some(api))
}

/// Disables the host API named by `input`, the inverse of [`add_capability`].
/// A catalog plugin without a host API of its own has nothing in `apis`.
/// Returns the API when the `apis` edit removed it.
pub(in crate::commands) fn remove_capability(
    cwd: &Path,
    input: &str,
) -> Result<Option<&'static str>> {
    let api = normalize_capability(input);
    ensure!(
        api.is_some() || catalog_add_name(input).is_some(),
        "unknown capability {input:?}"
    );
    let Some(api) = api else {
        return Ok(None);
    };
    let editable = config::resolve_editable_config(&absolute(cwd)?)?;
    let source = fs::read_to_string(&editable.path)
        .with_context(|| format!("read {}", editable.path.display()))?;
    let next = config::mutate_config_source(
        &editable.path,
        &source,
        config::ConfigMutation::RemoveApi(api),
    )?;
    if next == source {
        step(format!("capability {api} is not configured"));
        return Ok(None);
    }
    atomic_write(&editable.path, next)?;
    Ok(Some(api))
}

fn run_streaming<I, S>(program: &str, arguments: I, cwd: &Path) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = process::run_streaming(
        CommandSpec::new(program)
            .args(
                arguments
                    .into_iter()
                    .map(|value| value.as_ref().to_os_string()),
            )
            .cwd(cwd)
            .passthrough_sensitive_env(true),
    )?;
    ensure_success(status, program)
}

struct InitFrontend {
    tool: Option<String>,
    output: Option<String>,
}

fn detect_frontend(root: &Path) -> InitFrontend {
    let detected = crate::project::detect_frontend(root).ok();
    let tool = detected.as_ref().and_then(|value| {
        (value.build_tool != crate::project::BuildTool::None).then(|| value.build_tool.to_string())
    });
    let output = detected
        .as_ref()
        .map(|value| value.out_dir.to_string_lossy().into_owned());
    InitFrontend { tool, output }
}

fn render_config(name: &str, frontend_dir: &str) -> String {
    format!(
        "import {{ defineConfig }} from \"@tynd/cli\";\n\n// {name} — Tynd configuration\nexport default defineConfig({{\n  backend: \"backend/main.ts\",\n  frontendDir: {},\n}});\n",
        serde_json::to_string(frontend_dir).unwrap_or_else(|_| "\"frontend\"".into())
    )
}

fn render_backend(name: &str, frontend_dir: &str) -> String {
    let name = serde_json::to_string(name).unwrap_or_else(|_| "\"Tynd app\"".into());
    let directory = escape_template_literal(frontend_dir);
    format!(
        r#"import {{ app }} from "@tynd/core";

const appName = {name};

export async function greet(name: string): Promise<string> {{
  return `Hello, ${{name}}! Welcome to ${{appName}}.`;
}}

app.start({{
  frontendDir: `${{import.meta.dir}}/../{directory}`,
  capabilities: [],
  window: {{ title: appName, width: 1200, height: 800, center: true }},
}});
"#
    )
}

/// Escapes `value` for verbatim use inside a JavaScript template literal.
fn escape_template_literal(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace("${", "\\${")
}

const BUN_TYPES_PACKAGE: &str = "@types/bun";
const BUN_TYPES_VERSION: &str = "^1.4.5";

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        escape_template_literal, init_at_root, next_steps, render_backend, resolve_framework,
        DEFAULT_FRAMEWORK, FRAMEWORKS,
    };

    #[test]
    fn package_gets_host_sources_instead_of_a_prebuilt_host() {
        let range = format!("^{}", crate::VERSION);
        let mut package = serde_json::json!({ "name": "app" });
        super::patch_package(&mut package).expect("patch");
        assert_eq!(package["devDependencies"]["@tynd/host-src"], range.as_str());
        assert_eq!(package["dependencies"]["@tynd/core"], range.as_str());
        assert!(package["dependencies"].get("@tynd/host").is_none());
        assert!(package["devDependencies"].get("@tynd/host").is_none());

        // A range the app already declares is kept, wherever it is declared.
        let mut package = serde_json::json!({ "dependencies": { "@tynd/host-src": "1.0.0" } });
        super::patch_package(&mut package).expect("patch");
        assert_eq!(package["dependencies"]["@tynd/host-src"], "1.0.0");
        assert!(package["devDependencies"].get("@tynd/host-src").is_none());
    }

    #[test]
    fn framework_accepts_ts_aliases() {
        assert_eq!(
            resolve_framework("react"),
            Some(("react", Some("react-ts")))
        );
        assert_eq!(
            resolve_framework("react-ts"),
            Some(("react", Some("react-ts")))
        );
        assert_eq!(resolve_framework("angular-ts"), Some(("angular", None)));
        assert_eq!(resolve_framework("ember"), None);
        assert_eq!(resolve_framework("ts"), None);
    }

    #[test]
    fn default_framework_is_react() {
        assert_eq!(
            resolve_framework(DEFAULT_FRAMEWORK),
            Some(("react", Some("react-ts")))
        );
    }

    #[test]
    fn only_angular_lacks_a_vite_template() {
        for (name, template) in FRAMEWORKS {
            assert_eq!(template.is_none(), *name == "angular", "{name}");
        }
    }

    #[test]
    fn init_without_frontend_fails_and_writes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = init_at_root(dir.path(), true).expect_err("no frontend");
        let message = format!("{error:#}");
        assert!(message.contains("no frontend found in"), "{message}");
        assert!(
            message.contains("tynd create <name> --framework react"),
            "{message}"
        );
        assert_eq!(fs::read_dir(dir.path()).expect("read dir").count(), 0);

        // A bare static frontend is not adopted either.
        fs::create_dir_all(dir.path().join("frontend")).expect("create frontend");
        fs::write(dir.path().join("frontend/index.html"), "<html></html>").expect("write html");
        init_at_root(dir.path(), true).expect_err("static frontend is not adopted");
        assert!(!dir.path().join("tynd.config.ts").exists());
        assert!(!dir.path().join("backend").exists());

        let missing = dir.path().join("typo");
        init_at_root(&missing, false).expect_err("no frontend");
        assert!(!missing.exists());
    }

    #[test]
    fn init_with_existing_config_skips_frontend_check() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = "export default { backend: \"backend/main.ts\", frontendDir: \"web\" };
";
        fs::write(dir.path().join("tynd.config.ts"), config).expect("write config");
        fs::create_dir_all(dir.path().join("web")).expect("create web");
        fs::write(dir.path().join("web/index.html"), "<html></html>").expect("write html");
        init_at_root(dir.path(), false).expect("existing project is fixed");
        assert_eq!(
            fs::read_to_string(dir.path().join("tynd.config.ts")).expect("read config"),
            config
        );
        assert!(dir.path().join("package.json").is_file());
        assert!(dir.path().join("backend/main.ts").is_file());
    }

    #[test]
    fn init_without_frontend_leaves_package_json_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let package = "{ \"name\": \"app\" }";
        fs::write(dir.path().join("package.json"), package).expect("write package.json");
        init_at_root(dir.path(), false).expect_err("no frontend");
        assert_eq!(fs::read_dir(dir.path()).expect("read dir").count(), 1);
        assert_eq!(
            fs::read_to_string(dir.path().join("package.json")).expect("read package.json"),
            package
        );
    }

    #[test]
    fn backend_starter_has_no_ready_event() {
        let source = render_backend("my-app", "dist");
        assert!(!source.contains("createEmitter") && !source.contains("onReady"));
        assert!(source.contains("export async function greet"));
    }

    #[test]
    fn next_steps_mention_install_only_when_skipped() {
        assert!(!next_steps("app", true).contains("bun install"));
        let skipped = next_steps("app", false);
        assert!(skipped.contains(
            "cd app
    bun install"
        ));
        assert!(skipped.contains("tynd dev") && skipped.contains("tynd build"));
    }

    #[test]
    fn backend_starter_uses_template_literals() {
        let source = render_backend("my-app", "dist");
        assert!(source.contains("frontendDir: `${import.meta.dir}/../dist`,"));
        assert!(source.contains("const appName = \"my-app\";"));
        assert!(!source.contains(" + "));
        assert_eq!(escape_template_literal(r"a`b${c}\d"), r"a\`b\${c}\\d");
    }
}

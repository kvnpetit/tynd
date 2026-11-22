//! `tynd doctor [--fix]`.

use std::{fs, path::Path};

use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

use crate::{
    args::{DoctorArgs, InitArgs},
    config,
    output::{display_path, step, success, write_json, write_line},
};

use super::{
    scaffold::init,
    shared::{directory_size, generated_path, resolve_bun_runtime},
};

pub(in crate::commands) fn doctor(args: &DoctorArgs) -> Result<()> {
    ensure!(
        !args.dry_run || args.fix || args.repair,
        "doctor --dry-run requires --fix or --repair"
    );
    let root = config::find_project_root(&args.cwd)?;
    if args.fix {
        apply_fixes(&root, args.dry_run)?;
    }
    if args.repair {
        repair_generated_state(&root, args.yes, args.dry_run)?;
    }
    let mut result = validate_report(&root)?;
    if args.state {
        let state = generated_state_report(&root);
        let state_ok = state["ok"].as_bool().unwrap_or(false);
        result["state"] = state;
        if !state_ok {
            result["ok"] = Value::Bool(false);
        }
    }
    let ok = result["ok"].as_bool().unwrap_or(false);
    if args.json {
        write_json(&result)?;
    } else {
        for check in result["checks"].as_array().into_iter().flatten() {
            print_doctor_check(check)?;
        }
        if let Some(entries) = result
            .get("state")
            .and_then(|state| state.get("entries"))
            .and_then(Value::as_array)
        {
            for entry in entries {
                write_line(format_args!(
                    "{} state {} ({} bytes)",
                    if entry["ok"].as_bool().unwrap_or(false) {
                        "✓"
                    } else {
                        "×"
                    },
                    entry["name"].as_str().unwrap_or("unknown"),
                    entry["bytes"].as_u64().unwrap_or(0)
                ))?;
            }
        }
    }
    ensure!(ok, "doctor found failing checks");
    Ok(())
}

fn print_doctor_check(check: &Value) -> Result<()> {
    let marker = match check["level"].as_str() {
        Some("ok") => "✓",
        Some("hint") => "!",
        _ => "×",
    };
    write_line(format_args!(
        "{marker} {}",
        check["name"].as_str().unwrap_or("check")
    ))?;
    if let Some(detail) = check["detail"].as_str() {
        for line in detail.lines() {
            write_line(format_args!("    {line}"))?;
        }
    }
    if let Some(fix) = check["fix"].as_str() {
        write_line(format_args!("    fix: {fix}"))?;
    }
    if let Some(info) = check["info"].as_str() {
        write_line(format_args!("    {info}"))?;
    }
    Ok(())
}

/// Informational check: where host builds are cached for every project.
fn host_cache_check() -> Value {
    let cache = crate::project::host_cache_dir();
    let long_path = cache
        .as_ref()
        .ok()
        .and_then(|path| crate::project::long_host_cache_warning(path));
    let mut check = doctor_check(
        "shared host build cache",
        cache.is_ok() && long_path.is_none(),
        true,
        cache
            .as_ref()
            .err()
            .map(|error| format!("{error:#}"))
            .or(long_path),
        Some(format!(
            "set {} to a short, writable directory",
            crate::project::CACHE_DIR_VARIABLE
        )),
    );
    if let Ok(path) = &cache {
        check["info"] = json!(format!(
            "{} (override with {}, remove with `tynd clean --cache`)",
            display_path(path),
            crate::project::CACHE_DIR_VARIABLE
        ));
    }
    check["path"] = json!(cache.ok());
    check
}

fn generated_state_report(root: &Path) -> Value {
    let mut entries = Vec::new();
    let mut ok = true;
    for relative in [".tynd/cache", ".tynd/build", "release"] {
        match generated_path(root, relative) {
            Ok(path) => {
                let result = if path.exists() {
                    directory_size(&path)
                } else {
                    Ok(0)
                };
                match result {
                    Ok(bytes) => entries.push(json!({
                        "name": relative,
                        "path": path,
                        "exists": path.exists(),
                        "ok": true,
                        "bytes": bytes
                    })),
                    Err(error) => {
                        ok = false;
                        entries.push(json!({
                            "name": relative,
                            "path": path,
                            "exists": path.exists(),
                            "ok": false,
                            "error": format!("{error:#}")
                        }));
                    },
                }
            },
            Err(error) => {
                ok = false;
                entries.push(json!({
                    "name": relative,
                    "ok": false,
                    "error": format!("{error:#}")
                }));
            },
        }
    }
    json!({ "ok": ok, "entries": entries })
}

fn repair_generated_state(root: &Path, yes: bool, dry_run: bool) -> Result<()> {
    ensure!(
        yes || dry_run,
        "doctor --repair removes rebuildable caches; pass --yes or --dry-run"
    );
    let root = fs::canonicalize(root)?;
    for relative in [".tynd/cache", ".tynd/build"] {
        let path = generated_path(&root, relative)?;
        if !path.exists() {
            continue;
        }
        if dry_run {
            step(format!(
                "would repair state by removing {}",
                display_path(&path)
            ));
        } else {
            fs::remove_dir_all(&path)
                .with_context(|| format!("remove repairable state {}", path.display()))?;
        }
    }
    if !dry_run {
        success("rebuildable project state repaired");
    }
    Ok(())
}

/// `tynd doctor --fix` (and `tynd dev` self-healing): scaffolds what is
/// missing and normalizes the Tynd scripts through `tynd init`.
pub(super) fn apply_fixes(root: &Path, dry_run: bool) -> Result<()> {
    let mut changes = Vec::new();
    if config::find_config_in_directory(root).is_none() {
        changes.push("create tynd.config.ts and app scaffold");
    }
    if !root.join("package.json").exists() {
        changes.push("create package.json");
    }
    changes.push("normalize Tynd scripts and TypeScript editor types");
    if dry_run {
        for change in changes {
            step(format!("would {change}"));
        }
        return Ok(());
    }
    init(&InitArgs {
        cwd: root.to_path_buf(),
        force: false,
    })
}

/// One `tynd doctor` check. A failing check carries the reason (`detail`) and
/// the command or edit that fixes it (`fix`); a `hint` is reported but does not
/// fail the run.
fn doctor_check(
    name: &str,
    ok: bool,
    hint: bool,
    detail: Option<String>,
    fix: Option<String>,
) -> Value {
    json!({
        "name": name,
        "ok": ok,
        "level": if ok { "ok" } else if hint { "hint" } else { "error" },
        "detail": if ok { None } else { detail },
        "fix": if ok { None } else { fix },
    })
}

fn validate_report(root: &Path) -> Result<Value> {
    let config_result = config::load_typed(root, None);
    let project_root = config_result
        .as_ref()
        .map_or(root, |loaded| loaded.root.as_path());
    let plan_result = config_result.as_ref().ok().map(|loaded| {
        config::resolve_effective_host_plan(&loaded.config, &loaded.root).map(|_| ())
    });
    // The binary lookup only means something once the plan resolves.
    let host_result = match (&config_result, &plan_result) {
        (Ok(loaded), Some(Ok(()))) => Some(
            crate::project::find_host_binary(&loaded.root).and_then(|binary| {
                binary.context("not compiled yet: no host binary under .tynd/build/bin")
            }),
        ),
        _ => None,
    };
    let bundler_result = crate::project::resolve_binary(crate::project::TyndBinary::Bundler);
    let bun_result = resolve_bun_runtime(project_root);

    let mut checks = vec![doctor_check(
        "configuration",
        config_result.is_ok(),
        false,
        config_result
            .as_ref()
            .err()
            .map(|error| format!("{error:#}")),
        Some("fix tynd.config.ts, or run `tynd init` to create it".to_owned()),
    )];
    let skipped = || Some("skipped: the configuration did not load".to_owned());
    if let Ok(loaded) = &config_result {
        checks.push(doctor_check(
            "backend entry",
            loaded.config.backend.is_file(),
            false,
            Some(format!("not found: {}", loaded.config.backend.display())),
            Some("create the file or set `backend` in tynd.config.ts".to_owned()),
        ));
        checks.push(frontend_assets_check(loaded));
    } else {
        checks.push(doctor_check("backend entry", false, false, skipped(), None));
        checks.push(doctor_check(
            "frontend assets",
            false,
            false,
            skipped(),
            None,
        ));
    }
    checks.extend(host_build_checks(plan_result.as_ref(), project_root));
    // The host is compiled on the first `tynd dev` / `tynd build`, so a missing
    // binary is a hint, not an error.
    let mut host = doctor_check(
        "native host",
        matches!(host_result, Some(Ok(_))),
        host_result.is_some(),
        match &host_result {
            Some(Err(error)) => Some(format!("{error:#}")),
            _ if matches!(plan_result, Some(Err(_))) => {
                Some("skipped: the native host plan did not resolve".to_owned())
            },
            _ => skipped(),
        },
        // A skipped check (`None`: the config did not load) gets no fix line;
        // the configuration check already says how to fix it.
        host_result
            .is_some()
            .then(|| "run `tynd dev` or `tynd build` to compile the host".to_owned()),
    );
    host["path"] = json!(host_result.and_then(Result::ok));
    checks.push(host);
    let mut bundler = doctor_check(
        "native bundler",
        bundler_result.is_ok(),
        false,
        bundler_result
            .as_ref()
            .err()
            .map(|error| format!("{error:#}")),
        Some(bundler_binary_hint(project_root)),
    );
    bundler["path"] = json!(bundler_result.ok());
    checks.push(bundler);
    let mut bun = doctor_check(
        "Bun executable and version pin",
        bun_result.is_ok(),
        false,
        bun_result.as_ref().err().map(|error| format!("{error:#}")),
        Some(
            "install Bun (https://bun.sh); if .bun-version exists, install that exact version"
                .to_owned(),
        ),
    );
    bun["path"] = json!(bun_result.as_ref().ok().map(|runtime| runtime.path.clone()));
    bun["version"] = json!(bun_result
        .as_ref()
        .ok()
        .map(|runtime| runtime.version.clone()));
    checks.push(bun);

    let ok = checks
        .iter()
        .all(|check| check["level"].as_str() != Some("error"));
    Ok(json!({ "ok": ok, "checks": checks }))
}

/// Checks for what compiling the host needs: a resolvable host plan (`None`
/// when the configuration did not load), the Rust toolchain, and the
/// `@tynd/host-src` sources.
fn host_build_checks(plan_result: Option<&Result<()>>, project_root: &Path) -> Vec<Value> {
    let mut checks = Vec::new();
    // Probe from where cargo runs so a `rust-toolchain.toml` override applies.
    let build_dir = project_root.join(".tynd/build");
    let toolchain_result = crate::project::ensure_rust_toolchain(if build_dir.is_dir() {
        &build_dir
    } else {
        project_root
    });
    checks.push(match plan_result {
        Some(result) => doctor_check(
            "native host plan",
            result.is_ok(),
            false,
            result.as_ref().err().map(|error| format!("{error:#}")),
            Some(
                "fix `apis` and `plugins` in tynd.config.ts; install every listed plugin package"
                    .to_owned(),
            ),
        ),
        None => doctor_check(
            "native host plan",
            false,
            false,
            Some("skipped: the configuration did not load".to_owned()),
            None,
        ),
    });
    let mut rust = doctor_check(
        "Rust toolchain",
        toolchain_result.is_ok(),
        false,
        toolchain_result
            .as_ref()
            .err()
            .map(|error| format!("{error:#}")),
        Some(format!(
            "install Rust {} or newer from {} (or run `rustup update`)",
            crate::project::HOST_MSRV,
            crate::project::RUST_INSTALL_URL
        )),
    );
    rust["required"] = json!(crate::project::HOST_MSRV);
    rust["version"] = json!(toolchain_result
        .as_ref()
        .ok()
        .map(|toolchain| toolchain.rustc.to_string()));
    checks.push(rust);
    checks.push(host_cache_check());
    checks.push(doctor_check(
        "Tynd host sources",
        crate::project::host_sources_available(project_root),
        false,
        Some(format!(
            "{} is not installed",
            crate::project::HOST_SOURCES_PACKAGE
        )),
        Some(format!(
            "run `{}`",
            crate::project::host_sources_install_command(project_root)
        )),
    ));
    checks
}

/// A missing frontend output directory is expected until the first build
/// produces it, so it is a hint until `tynd build` has run once.
fn frontend_assets_check(loaded: &config::LoadedConfig) -> Value {
    let built = generated_path(&loaded.root, ".tynd/cache/bundle.dist.js")
        .is_ok_and(|bundle| bundle.is_file());
    doctor_check(
        "frontend assets",
        loaded.config.frontend_dir.is_dir(),
        !built,
        Some(format!(
            "frontend output not found: {}{}",
            loaded.config.frontend_dir.display(),
            if built { "" } else { " (expected before the first build)" }
        )),
        Some(
            "run `tynd build` (or `tynd dev` with a dev server); check `frontendDir` in tynd.config.ts"
                .to_owned(),
        ),
    )
}

fn bundler_binary_hint(cwd: &Path) -> String {
    if std::env::var_os("TYND_BUNDLER_BIN").is_some() {
        "point TYND_BUNDLER_BIN at an existing tynd-bundler executable, or unset it".to_owned()
    } else if cwd
        .ancestors()
        .any(|directory| directory.join("crates/bundler/Cargo.toml").is_file())
    {
        "run: cargo build -p tynd-bundler (in the same profile as tynd)".to_owned()
    } else {
        "reinstall the Tynd CLI (tynd-bundler ships next to tynd): bun add -d @tynd/cli".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::validate_report;

    #[test]
    fn host_plan_failure_is_an_error_with_its_message() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("tynd.config.json"),
            json!({ "plugins": ["@acme/missing-plugin"] }).to_string(),
        )
        .expect("write config");
        let report = validate_report(dir.path()).expect("report");
        let checks = report["checks"].as_array().expect("checks array");
        let find = |name: &str| {
            checks
                .iter()
                .find(|check| check["name"] == name)
                .unwrap_or_else(|| panic!("{name} check: {report}"))
        };
        let plan = find("native host plan");
        assert_eq!(plan["level"], "error", "{plan}");
        assert!(
            plan["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("@acme/missing-plugin")),
            "{plan}"
        );
        let host = find("native host");
        assert!(host["fix"].is_null(), "{host}");
        assert_eq!(report["ok"], false);
        let sources = find("Tynd host sources");
        assert!(
            sources["fix"]
                .as_str()
                .is_some_and(|fix| fix.contains("bun add -d @tynd/host-src")),
            "{sources}"
        );
        assert_eq!(find("Rust toolchain")["required"], "1.95");
        let cache = find("shared host build cache");
        assert_ne!(cache["level"], "error", "{cache}");
        if cache["ok"] == true {
            assert!(
                cache["info"]
                    .as_str()
                    .is_some_and(|info| info.contains("tynd clean --cache")),
                "{cache}"
            );
        }
    }

    #[test]
    fn skipped_host_check_has_no_fix_hint() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("tynd.config.json"), "{ not json").expect("write config");
        let report = validate_report(dir.path()).expect("report");
        let checks = report["checks"].as_array().expect("checks array");
        let host = checks
            .iter()
            .find(|check| check["name"] == "native host")
            .expect("host check");
        assert_eq!(host["ok"], false, "{host}");
        assert!(
            host["detail"]
                .as_str()
                .is_some_and(|detail| detail.starts_with("skipped")),
            "{host}"
        );
        assert!(host["fix"].is_null(), "{host}");
    }
}

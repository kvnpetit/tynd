//! `tynd validate`.

use anyhow::{ensure, Result};
use serde_json::{json, Value};

use crate::{
    args::ValidateArgs,
    config,
    fsx::absolute,
    output::{success, warn, write_json},
};

use super::shared::resolve_bun_runtime;

pub(in crate::commands) fn validate(args: &ValidateArgs) -> Result<()> {
    let requested_root = absolute(&args.cwd)?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let loaded = match config::load_typed(&requested_root, None) {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(format!("{error:#}"));
            None
        },
    };
    if let Some(loaded) = &loaded {
        collect_project_validation_issues(loaded, args.ci, &mut errors, &mut warnings);
        if args.ci {
            collect_ci_validation_issues(loaded, &mut errors);
        }
    }
    let report = json!({
        "ok": errors.is_empty(),
        "mode": if args.ci { "ci" } else { "development" },
        "errors": errors,
        "warnings": warnings
    });
    if args.json {
        write_json(&report)?;
    } else {
        for warning in report["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            warn(warning);
        }
        for error in report["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            eprintln!("error: {error}");
        }
        if errors.is_empty() {
            success(if args.ci {
                "project valid for CI"
            } else {
                "project valid"
            });
        }
    }
    ensure!(errors.is_empty(), "project validation failed");
    Ok(())
}

fn collect_project_validation_issues(
    loaded: &config::LoadedConfig,
    ci: bool,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    if loaded.path.is_none() {
        errors.push("no tynd.config file found".into());
    }
    if !loaded.config.backend.is_file() {
        errors.push(format!(
            "backend not found: {}",
            loaded.config.backend.display()
        ));
    }
    if !loaded.config.frontend_dir.is_dir() {
        push_validation_input_issue(
            ci,
            format!(
                "frontend directory not found: {}",
                loaded.config.frontend_dir.display()
            ),
            errors,
            warnings,
        );
    }
    if !loaded.root.join("package.json").is_file() {
        push_validation_input_issue(ci, "package.json not found".into(), errors, warnings);
    }
    super::plugin_imports::collect_undeclared_plugin_imports(loaded, warnings);
    super::security_policy::collect_open_scope_warning(loaded, warnings);
}

fn collect_ci_validation_issues(loaded: &config::LoadedConfig, errors: &mut Vec<String>) {
    if !loaded.root.join(".bun-version").is_file() {
        errors.push(".bun-version is required for reproducible CI builds".into());
    }
    if let Err(error) = resolve_bun_runtime(&loaded.root) {
        errors.push(format!("Bun runtime: {error:#}"));
    }
    // The host is compiled by `tynd build`; CI only needs what that build reads.
    if let Err(error) = config::resolve_effective_host_plan(&loaded.config, &loaded.root) {
        errors.push(format!("native host plan: {error:#}"));
    }
    if !crate::project::host_sources_available(&loaded.root) {
        errors.push(format!(
            "native host: Tynd host sources are not installed; run `{}`",
            crate::project::host_sources_install_command(&loaded.root)
        ));
    }
    if let Err(error) = crate::project::resolve_binary(crate::project::TyndBinary::Bundler) {
        errors.push(format!("native bundler: {error:#}"));
    }
}

fn push_validation_input_issue(
    ci: bool,
    issue: String,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    if ci {
        errors.push(issue);
    } else {
        warnings.push(issue);
    }
}

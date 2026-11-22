use std::{fs, path::Path, process::Command};

use anyhow::{bail, ensure, Context, Result};

use crate::{
    config::{self, ConfigMutation},
    fsx::{atomic_write, read_json},
    output::{step, success, warn},
    project,
};

use super::{
    shared::{resolve_plugin_manifest, RegexCache},
    PluginMutationRequest,
};

fn validate_plugin_package(package: &str) -> Result<()> {
    ensure!(
        RegexCache::package_name().is_match(package),
        "plugin package must use @scope/plugin-name or plugin-name; versions, paths, and flags are not accepted"
    );
    Ok(())
}

pub(in crate::commands) fn plugin_add(request: &PluginMutationRequest) -> Result<()> {
    validate_plugin_package(&request.package)?;
    for selector in &request.permissions {
        ensure!(
            config::is_valid_permission_selector(selector),
            "invalid plugin permission selector {selector:?}; expected api:method"
        );
    }
    let editable = config::resolve_editable_config(&request.cwd)?;
    let original = fs::read_to_string(&editable.path)
        .with_context(|| format!("read {}", editable.path.display()))?;
    let next = config::mutate_config_source(
        &editable.path,
        &original,
        ConfigMutation::AddPlugin {
            package: &request.package,
            permissions: &request.permissions,
        },
    )?;
    let already_configured = next == original;
    let mut dependency_added = false;

    if !request.config_only {
        let dependency_declared = package_declares_dependency(&editable.root, &request.package)?;
        let installed = resolve_plugin_manifest(&editable.root, &request.package).is_some();
        let mut command = Command::new("bun");
        if !dependency_declared {
            command.args(["add", &install_spec(&request.package)]);
            dependency_added = true;
        } else if !installed {
            command.arg("install");
        }
        if !dependency_declared || !installed {
            let status = command
                .current_dir(&editable.root)
                .status()
                .context("start bun dependency install")?;
            if !status.success() {
                bail!("bun dependency install failed with {status}");
            }
        }
    }

    if already_configured {
        step(format!("{} already declared", request.package));
        return Ok(());
    }
    if let Err(error) = atomic_write(&editable.path, next).context("write plugin config") {
        if dependency_added {
            let rollback = Command::new("bun")
                .args(["remove", &request.package])
                .current_dir(&editable.root)
                .status();
            if !rollback.is_ok_and(|status| status.success()) {
                warn(format!(
                    "could not roll back newly added dependency {}",
                    request.package
                ));
            }
        }
        return Err(error);
    }
    success(format!("Added {}", request.package));
    Ok(())
}

/// `bun add` argument for `package`: `<package>@<tag>` for a catalog plugin
/// published under a dist-tag other than `latest` (plugins/catalog.json
/// `distTag`), which a plain `bun add <package>` cannot resolve.
fn install_spec(package: &str) -> String {
    config::PLUGIN_DIST_TAGS
        .iter()
        .find(|(name, _)| *name == package)
        .map_or_else(|| package.to_owned(), |(_, tag)| format!("{package}@{tag}"))
}

/// Installs `@tynd/host-src` (the Rust sources the app host is compiled from)
/// as a devDependency unless it is already there.
pub(in crate::commands) fn ensure_host_sources(cwd: &Path) -> Result<()> {
    let editable = config::resolve_editable_config(cwd)?;
    let root = &editable.root;
    if package_declares_dependency(root, project::HOST_SOURCES_PACKAGE)?
        || project::host_sources_available(root)
    {
        return Ok(());
    }
    let command = project::host_sources_install_command(root);
    step(format!("Tynd host sources are missing; running {command}"));
    let status = Command::new("bun")
        .args(["add", "-d", &project::host_sources_spec(root)])
        .current_dir(root)
        .status()
        .context("start bun dependency install")?;
    if !status.success() {
        bail!("bun dependency install failed with {status}; run `{command}` manually");
    }
    success(format!("Added {}", project::HOST_SOURCES_PACKAGE));
    Ok(())
}

fn package_declares_dependency(cwd: &Path, package: &str) -> Result<bool> {
    let manifest_path = cwd.join("package.json");
    if !manifest_path.is_file() {
        return Ok(false);
    }
    let manifest = read_json(&manifest_path)?;
    Ok([
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ]
    .into_iter()
    .any(|section| {
        manifest
            .get(section)
            .and_then(|value| value.get(package))
            .is_some()
    }))
}

pub(in crate::commands) fn plugin_remove(request: &PluginMutationRequest) -> Result<()> {
    validate_plugin_package(&request.package)?;
    let editable = config::resolve_editable_config(&request.cwd)?;
    let original = fs::read_to_string(&editable.path)
        .with_context(|| format!("read {}", editable.path.display()))?;
    let next = config::mutate_config_source(
        &editable.path,
        &original,
        ConfigMutation::RemovePlugin(&request.package),
    )?;
    let changed = next != original;
    // `bun remove` on an undeclared package reinstalls everything and may
    // drop the lockfile, so it only runs for a declared dependency.
    let uninstall =
        !request.config_only && package_declares_dependency(&editable.root, &request.package)?;
    if !changed && !uninstall {
        let reason = if request.config_only {
            "is not declared in the config"
        } else {
            "is not installed"
        };
        step(format!("{} {reason}; nothing to remove", request.package));
        return Ok(());
    }
    if changed {
        atomic_write(&editable.path, &next)?;
    } else {
        step(format!("{} is not declared in the config", request.package));
    }

    if uninstall {
        let status = Command::new("bun")
            .args(["remove", &request.package])
            .current_dir(&editable.root)
            .status()
            .context("start bun remove")?;
        if !status.success() {
            if changed {
                atomic_write(&editable.path, original)?;
            }
            bail!("bun remove failed with {status}; config restored");
        }
        success(format!("Removed {}", request.package));
    } else {
        success(format!("Removed {} from the config", request.package));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{install_spec, plugin_remove, PluginMutationRequest};

    fn remove(cwd: &std::path::Path, package: &str) {
        plugin_remove(&PluginMutationRequest {
            cwd: cwd.to_path_buf(),
            package: package.to_owned(),
            config_only: false,
            permissions: Vec::new(),
        })
        .expect("remove succeeds without running bun");
    }

    // Without a package.json `bun remove` fails (as does a missing bun), so
    // success proves the uninstall was skipped for an undeclared package.
    #[test]
    fn removing_an_undeclared_package_skips_bun() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("tynd.config.json");
        let source = "{ \"plugins\": [\"@tynd/plugin-a\"] }\n";
        fs::write(&config, source).expect("write config");

        remove(dir.path(), "@tynd/plugin-clipboard");
        assert_eq!(fs::read_to_string(&config).expect("read config"), source);

        remove(dir.path(), "@tynd/plugin-a");
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config).expect("read config"))
                .expect("valid JSON");
        assert_eq!(value.get("plugins"), None, "an emptied list is dropped");
        assert!(!dir.path().join("bun.lock").exists());
    }

    #[test]
    fn plugins_published_under_a_dist_tag_install_with_it() {
        assert_eq!(
            install_spec("@tynd/plugin-llm"),
            "@tynd/plugin-llm@experimental"
        );
        assert_eq!(install_spec("@tynd/plugin-fs"), "@tynd/plugin-fs");
        assert_eq!(install_spec("@acme/plugin-x"), "@acme/plugin-x");
    }
}

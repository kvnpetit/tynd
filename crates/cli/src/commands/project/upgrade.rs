//! `tynd upgrade`.

use std::fs;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::{
    args::UpgradeArgs,
    config,
    output::{step, success},
    process::{self, CommandSpec},
};

use super::shared::ensure_success;

pub(in crate::commands) fn upgrade(args: &UpgradeArgs) -> Result<()> {
    let requested_root = config::find_project_root(&args.cwd)?;
    let (root, plugins) = if args.all {
        let loaded = config::load_typed(&requested_root, None)
            .context("load project config for `tynd upgrade --all`")?;
        let plugins = config::resolve_plugins(&loaded.config);
        (loaded.root, plugins)
    } else {
        (requested_root, Vec::new())
    };
    let manifest = fs::read(root.join("package.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let (packages, dev_packages) = upgrade_packages(manifest.as_ref(), plugins);
    step("upgrading Tynd packages to the latest compatible release");
    for (dev, packages) in [(false, packages), (true, dev_packages)] {
        if packages.is_empty() {
            continue;
        }
        let command = CommandSpec::new("bun")
            .arg("add")
            .args(dev.then_some("-d"))
            .args(packages)
            .cwd(&root)
            .passthrough_sensitive_env(true);
        ensure_success(process::run_streaming(command)?, "bun add")?;
    }
    success("Tynd dependencies upgraded");
    Ok(())
}

/// Build-time Tynd packages, installed as devDependencies.
const DEV_PACKAGES: [&str; 2] = ["@tynd/cli", "@tynd/host-src"];

/// `bun add` arguments for `tynd upgrade`: (dependencies, devDependencies).
/// `@tynd/cli` and `@tynd/host-src` are build tools and go to devDependencies
/// unless the app already declares them in `dependencies`.
fn upgrade_packages(manifest: Option<&Value>, plugins: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut packages = vec!["@tynd/core@latest".to_owned()];
    let mut dev_packages = Vec::new();
    for package in DEV_PACKAGES {
        let runtime = manifest
            .and_then(|manifest| manifest.get("dependencies"))
            .and_then(|dependencies| dependencies.get(package))
            .is_some();
        let spec = format!("{package}@latest");
        if runtime {
            packages.push(spec);
        } else {
            dev_packages.push(spec);
        }
    }
    packages.extend(plugins.into_iter().map(|name| format!("{name}@latest")));
    (packages, dev_packages)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::upgrade_packages;

    #[test]
    fn upgrade_installs_build_tools_as_dev_dependencies() {
        let (packages, dev) = upgrade_packages(None, vec!["@acme/plugin".to_owned()]);
        assert_eq!(packages, ["@tynd/core@latest", "@acme/plugin@latest"]);
        assert_eq!(dev, ["@tynd/cli@latest", "@tynd/host-src@latest"]);

        let manifest = json!({ "dependencies": { "@tynd/cli": "^1.0.0" } });
        let (packages, dev) = upgrade_packages(Some(&manifest), Vec::new());
        assert_eq!(packages, ["@tynd/core@latest", "@tynd/cli@latest"]);
        assert_eq!(dev, ["@tynd/host-src@latest"]);
    }
}

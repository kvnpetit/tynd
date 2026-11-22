//! `tynd add` / `tynd remove`: capabilities and plugins.

use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{plugins, project};
use crate::args::{AddArgs, RemoveArgs};
use crate::config::find_project_root;

/// `tynd add`: capability aliases edit `apis`; plugin-first capabilities also
/// add their plugin package; anything else is a plugin package. All or
/// nothing: when any step fails (a scaffold, an install, the host sources),
/// every project file the command touched is restored.
pub(super) fn add(args: AddArgs) -> Result<()> {
    let root = find_project_root(&args.cwd)?;
    let snapshot = ProjectSnapshot::capture(&root);
    let mut added_apis = Vec::new();
    match add_inputs(&args, &mut added_apis) {
        Ok(()) => {
            for api in added_apis {
                crate::output::success(format!("added capability {api}"));
            }
            Ok(())
        },
        Err(error) => {
            snapshot
                .restore()
                .context("restore project files after a failed `tynd add`")?;
            crate::output::warn("tynd add failed; project files restored");
            Err(error)
        },
    }
}

fn add_inputs(args: &AddArgs, added_apis: &mut Vec<&'static str>) -> Result<()> {
    for input in &args.inputs {
        let (package, permissions) = if project::is_capability_alias(input) {
            added_apis.extend(project::add_capability(&args.cwd, input)?);
            match project::capability_plugin_package(input) {
                Some(package) => (package.to_owned(), Vec::new()),
                None => continue,
            }
        } else {
            (input.clone(), split_csv(args.apis.as_deref()))
        };
        plugins::plugin_add(&plugins::PluginMutationRequest {
            cwd: args.cwd.clone(),
            package,
            config_only: args.no_install,
            permissions,
        })?;
    }
    if !args.no_install {
        plugins::ensure_host_sources(&args.cwd)?;
    }
    Ok(())
}

/// `tynd remove`: the inverse of `tynd add`. Capability aliases leave
/// `apis`; plugin-backed capabilities also remove their plugin package;
/// anything else is a plugin package. All or nothing, like `tynd add`.
pub(super) fn remove(args: &RemoveArgs) -> Result<()> {
    let root = find_project_root(&args.cwd)?;
    let snapshot = ProjectSnapshot::capture(&root);
    let mut removed_apis = Vec::new();
    match remove_inputs(args, &mut removed_apis) {
        Ok(()) => {
            for api in removed_apis {
                crate::output::success(format!("removed capability {api}"));
            }
            Ok(())
        },
        Err(error) => {
            snapshot
                .restore()
                .context("restore project files after a failed `tynd remove`")?;
            crate::output::warn("tynd remove failed; project files restored");
            Err(error)
        },
    }
}

fn remove_inputs(args: &RemoveArgs, removed_apis: &mut Vec<&'static str>) -> Result<()> {
    for input in &args.inputs {
        let package = if project::is_capability_alias(input) {
            removed_apis.extend(project::remove_capability(&args.cwd, input)?);
            match project::capability_plugin_package(input) {
                Some(package) => package,
                None => continue,
            }
        } else {
            input.as_str()
        };
        plugins::plugin_remove(&plugins::PluginMutationRequest {
            cwd: args.cwd.clone(),
            package: package.to_owned(),
            config_only: args.no_uninstall,
            permissions: Vec::new(),
        })?;
    }
    Ok(())
}

/// Project files `tynd add` / `tynd remove` may write (the config, `package.json`, the lock
/// file, and whatever `tynd init` scaffolds: root-level files and
/// `backend/main.ts`), recorded so a failed command can put them back.
struct ProjectSnapshot {
    root: PathBuf,
    root_existed: bool,
    backend_existed: bool,
    /// Every tracked file with its content, `None` when it did not exist.
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl ProjectSnapshot {
    /// Root-level files above this size are not tracked (nor ever written).
    const MAX_TRACKED_BYTES: u64 = 16 * 1024 * 1024;

    fn capture(root: &Path) -> Self {
        let backend_main = root.join("backend/main.ts");
        let mut files: Vec<(PathBuf, Option<Vec<u8>>)> = Self::root_files(root)
            .into_iter()
            .filter(|path| {
                fs::metadata(path).is_ok_and(|meta| meta.len() <= Self::MAX_TRACKED_BYTES)
            })
            .map(|path| {
                let content = fs::read(&path).ok();
                (path, content)
            })
            .collect();
        files.push((backend_main.clone(), fs::read(&backend_main).ok()));
        Self {
            root: root.to_path_buf(),
            root_existed: root.is_dir(),
            backend_existed: root.join("backend").exists(),
            files,
        }
    }

    fn root_files(root: &Path) -> Vec<PathBuf> {
        fs::read_dir(root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .collect()
    }

    fn restore(&self) -> Result<()> {
        for (path, content) in &self.files {
            match content {
                Some(content) => {
                    if fs::read(path).ok().as_ref() != Some(content) {
                        crate::fsx::atomic_write(path, content)?;
                    }
                },
                None if path.exists() => {
                    fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
                },
                None => {},
            }
        }
        // Root-level files created during the command (e.g. by `tynd init`).
        for path in Self::root_files(&self.root) {
            if !self.files.iter().any(|(tracked, _)| *tracked == path) {
                fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            }
        }
        // Directories the scaffold created are removed only when left empty.
        if !self.backend_existed {
            let _ = fs::remove_dir(self.root.join("backend"));
        }
        if !self.root_existed {
            let _ = fs::remove_dir(&self.root);
        }
        Ok(())
    }
}

fn split_csv(value: Option<&str>) -> Vec<String> {
    value
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests;

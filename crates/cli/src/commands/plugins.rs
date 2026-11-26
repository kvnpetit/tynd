use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::args::{PluginListArgs, PluginNewArgs};

mod development;
mod mutation;
mod scaffold;
mod shared;
mod validation;

pub(super) use development::{gen_types, plugin_dev};
pub(super) use mutation::{ensure_host_sources, plugin_add, plugin_remove};
pub(super) use scaffold::plugin_new;
pub(super) use validation::{plugin_check, plugin_list};

#[derive(Debug)]
pub(super) struct PluginNewRequest {
    pub(super) name: String,
    pub(super) parent: PathBuf,
    pub(super) native: bool,
    pub(super) force: bool,
}

#[derive(Debug)]
pub(super) struct PluginMutationRequest {
    pub(super) cwd: PathBuf,
    pub(super) package: String,
    pub(super) config_only: bool,
    pub(super) permissions: Vec<String>,
}

#[derive(Debug)]
pub(super) struct PluginCheckRequest {
    pub(super) directory: PathBuf,
    pub(super) json: bool,
}

#[derive(Debug)]
pub(super) struct PluginListRequest {
    pub(super) root: PathBuf,
    pub(super) json: bool,
}

#[derive(Debug)]
pub(super) struct PluginDevRequest {
    pub(super) plugin: String,
    pub(super) cwd: PathBuf,
    pub(super) scratch: Option<PathBuf>,
    pub(super) force: bool,
    pub(super) watch: bool,
}

#[derive(Debug)]
pub(super) struct GenTypesRequest {
    pub(super) cwd: PathBuf,
    pub(super) plugin: Option<String>,
    pub(super) dry_run: bool,
    pub(super) check: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct PluginCheck {
    pub(super) name: String,
    pub(super) status: &'static str,
    pub(super) detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) hint: Option<String>,
}

/// `tynd plugin new <path>`: the last path component names the plugin;
/// `--scope` turns it into `@<scope>/<name>`.
pub(super) fn new(args: PluginNewArgs) -> Result<()> {
    let (parent, mut name) = plugin_target(&args.path)?;
    if let Some(scope) = args.scope {
        name = format!("@{}/{}", scope.trim_start_matches('@'), name);
    }
    plugin_new(&PluginNewRequest {
        name,
        parent,
        native: args.native,
        force: args.force,
    })
    .map(|_| ())
}

/// `(parent directory, plugin name)` of a `tynd plugin new` target path.
fn plugin_target(path: &Path) -> Result<(PathBuf, String)> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("{} does not name a plugin directory", path.display()))?
        .to_owned();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    Ok((parent, name))
}

pub(super) fn list(args: PluginListArgs) -> Result<()> {
    let loaded = crate::config::load_typed(&crate::fsx::absolute(&args.cwd)?, None)?;
    plugin_list(
        &PluginListRequest {
            root: loaded.root,
            json: args.json,
        },
        &loaded.config,
    )
}

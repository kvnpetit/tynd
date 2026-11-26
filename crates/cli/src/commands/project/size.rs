//! `tynd size`.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
};

use anyhow::Result;
use serde_json::json;

use crate::{
    args::SizeArgs,
    config,
    output::{warn, write_json, write_line},
};

use super::shared::{directory_size, human_bytes};

pub(in crate::commands) fn size(args: &SizeArgs) -> Result<()> {
    let root = config::find_project_root(&args.cwd)?;
    let loaded = config::load_typed(&root, None).ok();
    let frontend_dir = loaded.as_ref().map_or_else(
        || root.join("frontend"),
        |loaded| loaded.config.frontend_dir.clone(),
    );
    let candidates = [
        ("frontend", frontend_dir),
        ("release", root.join("release")),
        (".tynd/cache", root.join(".tynd/cache")),
    ];
    let mut entries = BTreeMap::new();
    let mut seen = HashSet::new();
    for (label, path) in candidates {
        if path.exists() {
            let identity = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if seen.insert(identity) {
                entries.insert(label, directory_size(&path)?);
            }
        }
    }
    let total: u64 = entries.values().sum();
    let built = entries.contains_key("release");
    if args.json {
        write_json(&json!({ "totalBytes": total, "built": built, "entries": entries }))
    } else {
        if !built {
            warn(
                "no build output yet (release/ does not exist); sizes below are sources and caches only. Run `tynd build` to measure the packaged app",
            );
        }
        for (name, bytes) in &entries {
            write_line(format_args!("{name:16} {}", human_bytes(*bytes)))?;
        }
        write_line(format_args!("{:<16} {}", "total", human_bytes(total)))?;
        Ok(())
    }
}

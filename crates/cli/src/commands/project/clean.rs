//! `tynd clean`.

use std::fs;

use anyhow::{bail, Context, Result};

use crate::{
    args::CleanArgs,
    config,
    output::{display_path, success, write_line},
};

use super::shared::{ensure_generated_target, generated_path};

pub(in crate::commands) fn clean(args: &CleanArgs) -> Result<()> {
    let root = fs::canonicalize(config::find_project_root(&args.cwd)?)?;
    let targets = [
        generated_path(&root, ".tynd/cache")?,
        generated_path(&root, ".tynd/build")?,
        generated_path(&root, "release")?,
    ];
    if !args.yes && !args.dry_run {
        bail!("clean removes generated artifacts; pass --yes or --dry-run")
    }
    for target in targets {
        ensure_generated_target(&root, &target)?;
        if !target.exists() {
            continue;
        }
        if args.dry_run {
            write_line(format_args!("would remove {}", display_path(&target)))?;
        } else {
            fs::remove_dir_all(&target).with_context(|| format!("remove {}", target.display()))?;
        }
    }
    if args.cache {
        // Shared by every project: only removed on request.
        let host_cache = crate::project::host_cache_dir()?;
        if crate::project::remove_host_cache(&host_cache, args.dry_run)? && args.dry_run {
            write_line(format_args!("would remove {}", display_path(&host_cache)))?;
        }
    }
    if !args.dry_run {
        success(if args.cache {
            "generated artifacts and shared host cache removed"
        } else {
            "generated artifacts removed"
        });
    }
    Ok(())
}

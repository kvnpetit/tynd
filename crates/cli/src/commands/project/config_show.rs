//! `tynd config show`.

use anyhow::Result;

use crate::{
    args::ConfigShowArgs,
    config,
    fsx::absolute,
    output::{display_path, write_json},
};

pub(in crate::commands) fn config_show(args: &ConfigShowArgs) -> Result<()> {
    let loaded = config::load(&absolute(&args.cwd)?, None)?;
    if args.json {
        write_json(&loaded.value)
    } else {
        write_json(&loaded.value)?;
        if let Some(path) = loaded.path {
            eprintln!("config: {}", display_path(&path));
        }
        Ok(())
    }
}

//! `tynd info`.

use anyhow::Result;
use serde_json::json;

use crate::{
    args::InfoArgs,
    config,
    output::{display_path, write_json, write_line},
    process, VERSION,
};

pub(in crate::commands) fn info(args: &InfoArgs) -> Result<()> {
    let root = config::find_project_root(&args.cwd)?;
    let loaded = config::load_typed(&root, None).ok();
    let host = crate::project::find_host_binary(&root).ok().flatten();
    let bundler = crate::project::resolve_binary(crate::project::TyndBinary::Bundler).ok();
    let bun_version = process::run("bun", ["--version"], Some(&root))
        .ok()
        .map(|value| value.stdout_trimmed().to_owned());
    let value = json!({
        "toolingVersion": VERSION,
        "root": root,
        "config": loaded.as_ref().and_then(|value| value.path.clone()),
        "host": host,
        "bundler": bundler,
        "bun": bun_version,
        "checks": ["config", "paths", "native binaries"]
    });
    if args.json {
        write_json(&value)
    } else {
        write_line(format_args!("Tynd {VERSION}"))?;
        write_line(format_args!("project: {}", display_path(&root)))?;
        write_line(format_args!(
            "bun: {}",
            value["bun"].as_str().unwrap_or("not found")
        ))?;
        write_line(format_args!(
            "host: {}",
            value["host"].as_str().unwrap_or("not found")
        ))?;
        write_line(format_args!(
            "bundler: {}",
            value["bundler"].as_str().unwrap_or("not found")
        ))?;
        Ok(())
    }
}

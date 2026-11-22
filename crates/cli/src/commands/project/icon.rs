//! `tynd icon`: renders one square PNG/SVG into every icon the bundle targets
//! need, through `tynd-bundler icon`.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    args::IconArgs,
    config,
    fsx::absolute,
    output::{display_path, success, write_line},
    process::{self, CommandSpec},
};

pub(in crate::commands) fn icon(args: &IconArgs) -> Result<()> {
    let root = config::find_project_root(&absolute(&args.cwd)?)?;
    let source = if let Some(source) = &args.source {
        absolute(source)?
    } else {
        let loaded = config::load_typed(&root, None)?;
        let icon = loaded.config.icon.context(
            "no icon source: pass one (`tynd icon assets/logo.svg`) or set `icon` in \
             tynd.config.ts",
        )?;
        resolve_in(&root, &icon)
    };
    let out_dir = resolve_in(&root, &args.out);
    let bundler = crate::project::resolve_binary(crate::project::TyndBinary::Bundler)?;
    let output = process::run_spec(
        CommandSpec::new(bundler.as_os_str())
            .args([
                OsString::from("icon"),
                OsString::from("--source"),
                source.into_os_string(),
                OsString::from("--out-dir"),
                out_dir.clone().into_os_string(),
            ])
            .cwd(&root),
    )?;
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    let written = output.stdout.lines().filter(|line| !line.trim().is_empty());
    let mut count = 0_usize;
    for line in written {
        write_line(format_args!("{}", display_path(Path::new(line.trim()))))?;
        count += 1;
    }
    success(format!(
        "generated {count} icons in {}; point `icon` in tynd.config.ts at {} (or keep the SVG)",
        display_path(&out_dir),
        display_path(&out_dir.join("icon.png"))
    ));
    Ok(())
}

fn resolve_in(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_resolve_against_the_project_root() {
        let root = std::env::temp_dir().join("tynd-icon-root");
        assert_eq!(resolve_in(&root, Path::new("icons")), root.join("icons"));
        let absolute = std::env::temp_dir().join("elsewhere");
        assert_eq!(resolve_in(&root, &absolute), absolute);
    }
}

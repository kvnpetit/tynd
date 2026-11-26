//! Embeds the MCP static catalog into `tynd-mcp`.
//!
//! `scripts/gen-mcp-catalog.ts` generates `packages/mcp/catalog/` (run
//! `bun run gen:mcp-catalog`); this script only indexes what it wrote, so the
//! file list lives in the generator alone. The index is
//! `$OUT_DIR/mcp_catalog.rs`: two `&[(file name, contents)]` tables built with
//! `include_str!`, sorted by file name.

use std::{
    env,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};

fn main() -> io::Result<()> {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "CARGO_MANIFEST_DIR is not set")
        })?);
    let catalog = manifest.join("../../packages/mcp/catalog");
    let mut index = String::new();
    for (constant, directory, suffix) in [
        ("FRAMEWORK_DOCS", "framework-docs", ".md"),
        ("PLUGIN_SCHEMAS", "plugin-schemas", ".gen.ts"),
    ] {
        let directory = catalog.join(directory);
        println!("cargo:rerun-if-changed={}", directory.display());
        index.push_str(&table(constant, &directory, suffix)?);
    }
    let out = PathBuf::from(
        env::var_os("OUT_DIR")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "OUT_DIR is not set"))?,
    );
    fs::write(out.join("mcp_catalog.rs"), index)
}

fn table(constant: &str, directory: &Path, suffix: &str) -> io::Result<String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "read MCP catalog {}: {error}; run `bun run gen:mcp-catalog`",
                directory.display()
            ),
        )
    })? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(suffix) && entry.file_type()?.is_file() {
            files.push((name, entry.path()));
        }
    }
    files.sort();
    let mut table = format!("pub(super) static {constant}: &[(&str, &str)] = &[\n");
    for (name, path) in files {
        let path = path.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "catalog path is not UTF-8")
        })?;
        writeln!(table, "    ({name:?}, include_str!({path:?})),").map_err(io::Error::other)?;
    }
    table.push_str("];\n");
    Ok(table)
}

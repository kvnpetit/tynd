//! `tynd profile`: CPU flamegraph, Bun heap snapshot or IPC timing.

use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::fsx::absolute;
use crate::output::{display_path, step, success};

#[derive(Debug)]
pub(super) struct ProfileRequest {
    pub(super) cwd: PathBuf,
    pub(super) flamegraph: bool,
    pub(super) bun_memory: bool,
    pub(super) ipc: bool,
    pub(super) out: Option<PathBuf>,
}

pub(super) fn profile(request: &ProfileRequest) -> Result<()> {
    let project_root = absolute(&request.cwd)?;
    if request.flamegraph {
        return profile_flamegraph(&project_root, request.out.as_deref());
    }
    if !request.bun_memory && !request.ipc {
        bail!("select one profile mode: --cpu, --memory, or --ipc");
    }
    if request.ipc && request.out.is_some() {
        bail!("--out is only supported with --cpu or --memory");
    }
    profile_dev_session(&project_root, request)
}

/// `tynd profile --cpu`: runs `cargo flamegraph` against the native host.
fn profile_flamegraph(project_root: &Path, output: Option<&Path>) -> Result<()> {
    let output = resolve_profile_output(project_root, output, "flamegraph.svg");
    if project_root.join(".tynd/build/Cargo.toml").is_file() {
        // The generated host workspace: same target directory, host target
        // and build environment as `tynd dev` / `tynd build`.
        let loaded = crate::config::load_typed(project_root, None)?;
        let command =
            crate::project::host_flamegraph_command(&loaded.root, &loaded.config, &output)?;
        step("profiling native host with cargo flamegraph");
        crate::process::run_streaming(command).context("run cargo flamegraph")?;
        success(format!("Flamegraph -> {}", display_path(&output)));
        return Ok(());
    }
    let manifest = checkout_flamegraph_manifest(project_root)?;
    step("profiling native host with cargo flamegraph");
    let status = Command::new("cargo")
        .args(["flamegraph", "--output"])
        .arg(&output)
        .arg("--manifest-path")
        .arg(&manifest)
        .args(["--bin", "tynd-runtime"])
        .env_clear()
        .envs(crate::process::sanitized_environment())
        .current_dir(manifest.parent().unwrap_or(project_root))
        .status()
        .context("start cargo flamegraph")?;
    if !status.success() {
        bail!("cargo flamegraph failed with {status}");
    }
    success(format!("Flamegraph -> {}", display_path(&output)));
    Ok(())
}

/// `tynd profile --memory` / `--ipc`: re-runs `tynd dev` with Bun heap
/// profiling and/or IPC trace logging enabled.
fn profile_dev_session(project_root: &Path, request: &ProfileRequest) -> Result<()> {
    let current = std::env::current_exe()?;
    let mut command = Command::new(current);
    command
        .arg("dev")
        .arg("--cwd")
        .arg(project_root)
        .env_clear()
        .envs(crate::process::sanitized_environment());
    let memory_output = if request.bun_memory {
        let output = resolve_profile_output(
            project_root,
            request.out.as_deref(),
            "tynd-dev.heapsnapshot",
        );
        command.env("BUN_OPTIONS", heap_profile_options(&output)?);
        Some(output)
    } else {
        None
    };
    if request.ipc {
        command.env("RUST_LOG", "tynd::ipc=trace,tynd::debug=debug");
    }
    let status = command.status()?;
    if !status.success() {
        bail!("profiled dev session exited with {status}");
    }
    if let Some(output) = memory_output {
        if !output.is_file() {
            bail!(
                "Bun exited without writing the requested heap snapshot: {}",
                output.display()
            );
        }
        success(format!("Heap snapshot -> {}", display_path(&output)));
    }
    Ok(())
}

/// `BUN_OPTIONS` value writing a heap snapshot to `output`; creates the
/// output directory.
fn heap_profile_options(output: &Path) -> Result<String> {
    let parent = output
        .parent()
        .context("memory profile output has no parent")?;
    fs::create_dir_all(parent)?;
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .context("memory profile filename must be UTF-8")?;
    let directory = parent
        .to_str()
        .context("memory profile directory must be UTF-8")?;
    if name.contains('"') || directory.contains('"') {
        bail!("memory profile output must not contain double quotes");
    }
    Ok(format!(
        "--heap-prof --heap-prof-name=\"{name}\" --heap-prof-dir=\"{directory}\""
    ))
}

fn resolve_profile_output(root: &Path, output: Option<&Path>, default_name: &str) -> PathBuf {
    match output {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => root.join(path),
        None => root.join(default_name),
    }
}

/// Without a generated host workspace, a project inside a Tynd checkout
/// profiles the checkout's `tynd-runtime`.
fn checkout_flamegraph_manifest(project_root: &Path) -> Result<PathBuf> {
    for directory in project_root.ancestors() {
        let manifest = directory.join("Cargo.toml");
        if manifest.is_file() && directory.join("crates/runtime/Cargo.toml").is_file() {
            return Ok(manifest);
        }
    }
    bail!(
        "native flamegraph needs the generated host workspace; run `tynd dev` or `tynd build` first, or use `tynd profile --memory`/`--ipc`"
    )
}

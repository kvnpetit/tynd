use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::config::{LoadedConfig, WebView2InstallMode};
use anyhow::{ensure, Context, Result};

use super::shared::{copy_tree_strict, ensure_existing_path_is_unlinked, is_link_or_reparse};

#[derive(Clone, Debug)]
pub(super) struct FixedWebviewRuntime {
    pub(super) relative: String,
    source: PathBuf,
}

pub(super) fn fixed_webview_runtime(loaded: &LoadedConfig) -> Result<Option<FixedWebviewRuntime>> {
    if !cfg!(windows) {
        return Ok(None);
    }
    let Some(WebView2InstallMode::FixedRuntime { path: configured }) = loaded
        .config
        .bundle
        .as_ref()
        .and_then(|bundle| bundle.windows.as_ref())
        .and_then(|windows| windows.webview_install_mode.as_ref())
    else {
        return Ok(None);
    };
    let segments = configured
        .split(['/', '\\'])
        .map(str::trim)
        .collect::<Vec<_>>();
    ensure!(
        !segments.is_empty()
            && segments.iter().all(|segment| {
                !segment.is_empty() && *segment != "." && *segment != ".." && !segment.contains(':')
            }),
        "fixedRuntime WebView path must be a safe project-relative path"
    );
    let relative = segments.join("/");
    let mut source = loaded.root.clone();
    for segment in &segments {
        source.push(segment);
    }
    ensure!(
        source.is_dir(),
        "WebView2 fixed runtime directory not found: {}",
        source.display()
    );
    let root = fs::canonicalize(&loaded.root)?;
    let source = fs::canonicalize(&source)?;
    ensure!(
        source.starts_with(&root),
        "WebView2 fixed runtime escapes the project root"
    );
    for entry in walkdir::WalkDir::new(&source).follow_links(false) {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(
            !is_link_or_reparse(&metadata),
            "WebView2 fixed runtime contains a linked path: {}",
            entry.path().display()
        );
    }
    Ok(Some(FixedWebviewRuntime { relative, source }))
}

pub(super) fn stage_fixed_webview_runtime(
    root: &Path,
    output: &Path,
    runtime: &FixedWebviewRuntime,
) -> Result<()> {
    let output_parent = fs::canonicalize(output.parent().context("build output has no parent")?)?;
    let destination = runtime
        .relative
        .split('/')
        .fold(output_parent.clone(), |path, segment| path.join(segment));
    if destination.exists()
        && fs::canonicalize(&destination).is_ok_and(|path| path == runtime.source)
    {
        return Ok(());
    }
    ensure_existing_path_is_unlinked(&output_parent, &destination)?;
    let root = fs::canonicalize(root)?;
    let generated_output = output_parent.strip_prefix(&root).is_ok_and(|relative| {
        let relative = relative.to_string_lossy().replace('\\', "/");
        relative == "release" || relative.starts_with("release/")
    });
    if destination.exists() {
        ensure!(
            generated_output,
            "refusing to replace existing fixed WebView runtime outside release/: {}",
            destination.display()
        );
        let metadata = fs::symlink_metadata(&destination)?;
        ensure!(
            !is_link_or_reparse(&metadata) && metadata.is_dir(),
            "refusing unsafe fixed WebView destination {}",
            destination.display()
        );
        fs::remove_dir_all(&destination)?;
    }
    let destination_parent = destination
        .parent()
        .context("WebView destination has no parent")?;
    fs::create_dir_all(destination_parent)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging = destination_parent.join(format!(
        ".tynd-webview-stage-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&staging)?;
    let copied = copy_tree_strict(&runtime.source, &staging);
    if let Err(error) = copied {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    fs::rename(&staging, &destination)
        .with_context(|| format!("install fixed WebView runtime at {}", destination.display()))?;
    Ok(())
}

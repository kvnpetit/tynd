use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use camino::{Utf8Path, Utf8PathBuf};
use cap_std::fs::Dir;
use serde_json::{json, Value};

use super::roots::{absolute_lexical, resolve};

const MAX_FULL_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// String arg extraction without security side-effects. Authorization stays
/// at the operation boundary because relative symlink targets must first be
/// resolved against the link parent.
///
/// JSON strings are UTF-8 by definition, so the returned `&str` already
/// satisfies camino's `Utf8Path` invariant — callers that want a typed
/// path can wrap with `Utf8Path::new(s)`.
pub(super) fn string_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("fs: missing '{name}' argument"))
}

/// Optional boolean arg; `default` when absent or not a boolean.
pub(super) fn bool_arg(args: &Value, name: &str, default: bool) -> bool {
    args.get(name).and_then(Value::as_bool).unwrap_or(default)
}

/// Typed Utf8 view of a string arg. Saves the `Utf8Path::new` boilerplate
/// at call sites that want camino semantics without changing the wire shape.
pub(super) fn utf8_path_arg<'a>(args: &'a Value, name: &str) -> Result<&'a Utf8Path, String> {
    string_arg(args, name).map(Utf8Path::new)
}

/// String + policy check + cap-std root resolution. Returns the original
/// path (Utf8 since the wire is JSON) plus `(Dir, relative)`. The
/// relative path stays `PathBuf` because cap-std / std::fs operate on
/// `Path` — we only convert at the boundary, not internally.
pub(super) fn path_arg<'a>(
    args: &'a Value,
    name: &str,
) -> Result<(&'a Utf8Path, &'static Dir, PathBuf), String> {
    let p = utf8_path_arg(args, name)?;
    let (dir, rel) = resolve(p.as_str())?;
    Ok((p, dir, rel))
}

/// Convert an FS-sourced path (dir entry, symlink target, watch event)
/// into a Utf8 string for the JS surface. Surfaces a structured error
/// instead of `to_string_lossy` so non-UTF8 Windows entries don't get
/// silently mangled into JSON.
pub(super) fn fs_path_to_utf8(p: PathBuf, ctx: &str) -> Result<Utf8PathBuf, String> {
    Utf8PathBuf::from_path_buf(p)
        .map_err(|bad| format!("{ctx}: path '{}' is not valid UTF-8", bad.to_string_lossy()))
}

pub(super) fn read_text(args: &Value) -> Result<Value, String> {
    let (path, dir, rel) = path_arg(args, "path")?;
    ensure_file_within_cap(dir, &rel, "fs.readText")?;
    let s = dir
        .read_to_string(&rel)
        .map_err(|e| format!("fs.readText({path}): {e}"))?;
    Ok(Value::String(s))
}

pub(super) fn write_text(args: &Value) -> Result<Value, String> {
    let (path, dir, rel) = path_arg(args, "path")?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "fs.writeText: missing 'content'".to_string())?;
    if bool_arg(args, "createDirs", false) {
        // Skip `create_dir_all("")` — saves a syscall at the Dir root.
        if let Some(parent) = rel.parent().filter(|p| !p.as_os_str().is_empty()) {
            dir.create_dir_all(parent)
                .map_err(|e| format!("fs.writeText: {e}"))?;
        }
    }
    dir.write(&rel, content)
        .map_err(|e| format!("fs.writeText({path}): {e}"))?;
    Ok(Value::Null)
}

pub(super) fn exists(args: &Value) -> Result<Value, String> {
    let s = string_arg(args, "path")?;
    // Out-of-scope and not-found both collapse to `false` — a hard error
    // here would leak whether scopes match.
    match resolve(s) {
        Ok((dir, rel)) => Ok(Value::Bool(dir.try_exists(&rel).unwrap_or(false))),
        Err(_) => Ok(Value::Bool(false)),
    }
}

pub(super) fn stat(args: &Value) -> Result<Value, String> {
    let (path, dir, rel) = path_arg(args, "path")?;
    let meta = dir
        .metadata(&rel)
        .map_err(|e| format!("fs.stat({path}): {e}"))?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.into_std().duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    Ok(json!({
        "size": meta.len(),
        "isFile": meta.is_file(),
        "isDir": meta.is_dir(),
        "isSymlink": meta.file_type().is_symlink(),
        "mtime": mtime,
    }))
}

pub(super) fn read_dir(args: &Value) -> Result<Value, String> {
    let (path, dir, rel) = path_arg(args, "path")?;
    let entries = dir
        .read_dir(&rel)
        .map_err(|e| format!("fs.readDir({path}): {e}"))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("fs.readDir: {e}"))?;
        // Reject non-UTF8 filenames at the boundary instead of lossy-converting —
        // the JS side expects clean strings, and silent mangling can desync
        // round-trip ops (rename/remove on the displayed name would miss).
        let name_os = entry.file_name();
        let name = name_os.to_str().ok_or_else(|| {
            format!(
                "fs.readDir({path}): entry '{}' is not valid UTF-8",
                name_os.to_string_lossy()
            )
        })?;
        let ft = entry.file_type().map_err(|e| format!("fs.readDir: {e}"))?;
        out.push(json!({
            "name": name,
            "isFile": ft.is_file(),
            "isDir": ft.is_dir(),
            "isSymlink": ft.is_symlink(),
        }));
    }
    Ok(Value::Array(out))
}

pub(super) fn mkdir(args: &Value) -> Result<Value, String> {
    let (path, dir, rel) = path_arg(args, "path")?;
    let created = if bool_arg(args, "recursive", false) {
        dir.create_dir_all(&rel)
    } else {
        dir.create_dir(&rel)
    };
    created.map_err(|e| format!("fs.mkdir({path}): {e}"))?;
    Ok(Value::Null)
}

pub(super) fn remove(args: &Value) -> Result<Value, String> {
    let (path, dir, rel) = path_arg(args, "path")?;
    let recursive = bool_arg(args, "recursive", false);
    // `symlink_metadata` so we delete the link, not the target.
    let removed = dir.symlink_metadata(&rel).and_then(|meta| {
        if !meta.is_dir() {
            dir.remove_file(&rel)
        } else if recursive {
            dir.remove_dir_all(&rel)
        } else {
            dir.remove_dir(&rel)
        }
    });
    removed.map_err(|e| format!("fs.remove({path}): {e}"))?;
    Ok(Value::Null)
}

pub(super) fn rename(args: &Value) -> Result<Value, String> {
    let (from, src_dir, src_rel) = path_arg(args, "from")?;
    let (to, dst_dir, dst_rel) = path_arg(args, "to")?;
    src_dir
        .rename(&src_rel, dst_dir, &dst_rel)
        .map_err(|e| format!("fs.rename({from} -> {to}): {e}"))?;
    Ok(Value::Null)
}

pub(super) fn copy(args: &Value) -> Result<Value, String> {
    let (from, src_dir, src_rel) = path_arg(args, "from")?;
    let (to, dst_dir, dst_rel) = path_arg(args, "to")?;
    // `Dir::copy` is intra-Dir; cross-root falls back to read+write.
    ensure_file_within_cap(src_dir, &src_rel, "fs.copy")?;
    let bytes = src_dir
        .read(&src_rel)
        .map_err(|e| format!("fs.copy({from} -> {to}): {e}"))?;
    dst_dir
        .write(&dst_rel, &bytes)
        .map_err(|e| format!("fs.copy({from} -> {to}): {e}"))?;
    Ok(Value::Null)
}

pub(super) fn copy_dir(args: &Value) -> Result<Value, String> {
    let from_str = string_arg(args, "from")?;
    let to_str = string_arg(args, "to")?;
    let (src_dir, src_rel) = resolve(from_str)?;
    let (dst_dir, dst_rel) = resolve(to_str)?;
    copy_dir_recursive(src_dir, &src_rel, dst_dir, &dst_rel)
        .map_err(|e| format!("fs.copyDir({from_str} -> {to_str}): {e}"))?;
    Ok(Value::Null)
}

fn copy_dir_recursive(src_dir: &Dir, src: &Path, dst_dir: &Dir, dst: &Path) -> std::io::Result<()> {
    dst_dir.create_dir_all(dst)?;
    for entry in src_dir.read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let name = entry.file_name();
        let src_child = src.join(&name);
        let dst_child = dst.join(&name);
        if ty.is_dir() {
            copy_dir_recursive(src_dir, &src_child, dst_dir, &dst_child)?;
        } else if ty.is_file() {
            ensure_file_within_cap(src_dir, &src_child, "fs.copyDir")
                .map_err(std::io::Error::other)?;
            let bytes = src_dir.read(&src_child)?;
            dst_dir.write(&dst_child, &bytes)?;
        }
        // Symlinks skipped — use the explicit `symlink` API.
    }
    Ok(())
}

fn ensure_file_within_cap(dir: &Dir, rel: &Path, op: &str) -> Result<(), String> {
    let meta = dir
        .metadata(rel)
        .map_err(|e| format!("{op}: metadata '{}': {e}", rel.display()))?;
    if !meta.is_file() {
        return Err(format!("{op}: '{}' is not a regular file", rel.display()));
    }
    if meta.len() > MAX_FULL_FILE_BYTES {
        return Err(format!(
            "{op}: '{}' is {} bytes; full-file operations are capped at {MAX_FULL_FILE_BYTES} \
             bytes",
            rel.display(),
            meta.len()
        ));
    }
    Ok(())
}

pub(super) fn symlink(args: &Value) -> Result<Value, String> {
    let target_str = string_arg(args, "target")?;
    let link_str = string_arg(args, "link")?;
    let link_absolute = absolute_lexical(link_str)?;
    let target = Path::new(target_str);
    #[cfg(windows)]
    if !target.is_absolute()
        && matches!(
            target.components().next(),
            Some(std::path::Component::Prefix(_))
        )
    {
        return Err(
            "fs.symlink: drive-relative targets are ambiguous and are not allowed".to_string(),
        );
    }
    let effective_target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        link_absolute
            .parent()
            .ok_or_else(|| "fs.symlink: link has no parent".to_string())?
            .join(target)
    };
    let effective_target = effective_target.to_str().ok_or_else(|| {
        format!(
            "fs.symlink: effective target '{}' is not valid UTF-8",
            effective_target.display()
        )
    })?;
    // Both ends are independently authorized. A dangling target is allowed
    // only when its deepest existing ancestor and missing suffix remain inside
    // the same capability/policy boundary.
    crate::security::policy::authorize_fs_path(effective_target)?;
    let (link_dir, link_rel) = resolve(link_str)?;
    platform_symlink(link_dir, target_str, &link_rel).map_err(|e| format!("fs.symlink: {e}"))?;
    Ok(Value::Null)
}

#[cfg(unix)]
fn platform_symlink(dir: &Dir, target: &str, link: &Path) -> std::io::Result<()> {
    // Writes the literal target verbatim — same as `std::os::unix::fs::symlink`.
    dir.symlink_contents(Path::new(target), link)
}

#[cfg(windows)]
fn platform_symlink(dir: &Dir, target: &str, link: &Path) -> std::io::Result<()> {
    // Windows splits file vs dir symlinks. `Path::is_dir` is just a hint
    // here (ambient authority OK since we're branching, not granting).
    let p = Path::new(target);
    if p.is_dir() {
        dir.symlink_dir(p, link)
    } else {
        dir.symlink_file(p, link)
    }
}

pub(super) fn readlink(args: &Value) -> Result<Value, String> {
    let path_str = string_arg(args, "path")?;
    let (dir, rel) = resolve(path_str)?;
    let target = dir
        .read_link_contents(&rel)
        .map_err(|e| format!("fs.readlink({path_str}): {e}"))?;
    // Reject non-UTF8 symlink targets — JS gets a typed string back.
    let target_utf8 = fs_path_to_utf8(target, &format!("fs.readlink({path_str})"))?;
    Ok(Value::String(target_utf8.into_string()))
}

pub(super) fn hardlink(args: &Value) -> Result<Value, String> {
    let original_str = string_arg(args, "original")?;
    let link_str = string_arg(args, "link")?;
    let (src_dir, src_rel) = resolve(original_str)?;
    let (dst_dir, dst_rel) = resolve(link_str)?;
    src_dir
        .hard_link(&src_rel, dst_dir, &dst_rel)
        .map_err(|e| format!("fs.hardlink: {e}"))?;
    Ok(Value::Null)
}

pub(super) fn trash_path(args: &Value) -> Result<Value, String> {
    let path = utf8_path_arg(args, "path")?;
    // `trash` has no Dir-relative variant. Consume the resolved path from the
    // central authorization gate and retain the documented ambient TOCTOU
    // limitation instead of checking one spelling and deleting another.
    let resolved = crate::security::policy::authorize_fs_path(path.as_str())?;
    trash::delete(&resolved).map_err(|e| format!("fs.trash({path}): {e}"))?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests;

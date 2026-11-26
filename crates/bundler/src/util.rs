use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use goblin::Object;
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, ZipWriter};

/// Returns "x86_64", "aarch64", "universal" (fat Mach-O), or "unknown".
pub(crate) fn detect_binary_arch(path: &Utf8Path) -> Result<String> {
    let buf = std::fs::read(path.as_std_path()).with_context(|| format!("read binary {path}"))?;
    let arch = match Object::parse(&buf).context("parse binary")? {
        Object::Elf(elf) => match elf.header.e_machine {
            goblin::elf::header::EM_X86_64 => "x86_64",
            goblin::elf::header::EM_AARCH64 => "aarch64",
            _ => "unknown",
        },
        Object::PE(pe) => {
            if pe.is_64 {
                "x86_64"
            } else {
                "i386"
            }
        },
        Object::Mach(goblin::mach::Mach::Fat(_)) => "universal",
        Object::Mach(goblin::mach::Mach::Binary(m)) => {
            if m.header.cputype == goblin::mach::cputype::CPU_TYPE_X86_64 {
                "x86_64"
            } else if m.header.cputype == goblin::mach::cputype::CPU_TYPE_ARM64 {
                "aarch64"
            } else {
                "unknown"
            }
        },
        _ => "unknown",
    };
    Ok(arch.to_string())
}

/// APPNOTE.TXT 4.4.17.1 mandates `/` for ZIP entry names regardless of
/// host OS. Windows producers default to `\\` which trips up extractors
/// on macOS/Linux and `jar`-style tools, so we normalise unconditionally.
fn to_zip_name(rel: &str) -> String {
    if std::path::MAIN_SEPARATOR == '/' {
        rel.to_string()
    } else {
        rel.replace('\\', "/")
    }
}

/// Archives only final bundle artifacts, never raw binaries, intermediate
/// WiX/NSIS files, or the archive itself.
pub(crate) fn zip_artifacts(artifact_paths: &[PathBuf], out_path: &Utf8Path) -> Result<()> {
    let out_file = std::fs::File::create(out_path.as_std_path())
        .with_context(|| format!("create zip {out_path}"))?;
    let mut zip = ZipWriter::new(out_file);
    let opts = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);

    let mut sorted_paths = artifact_paths.to_vec();
    sorted_paths.sort();
    for artifact in sorted_paths {
        if artifact == out_path.as_std_path() {
            continue;
        }

        let base = artifact.parent().context("artifact has no parent")?;
        for entry in WalkDir::new(&artifact)
            .sort_by_file_name()
            .follow_links(false)
        {
            let entry = entry.context("walk artifact entry")?;
            let path = entry.path();
            if path == out_path.as_std_path() {
                continue;
            }
            let rel = path.strip_prefix(base).context("strip artifact prefix")?;
            let name = to_zip_name(&rel.to_string_lossy());
            if entry.file_type().is_dir() {
                zip.add_directory(&name, SimpleFileOptions::default())
                    .context("zip add_directory")?;
                continue;
            }
            zip.start_file(&name, opts).context("zip start_file")?;
            let mut f = std::fs::File::open(path).context("open file for zip")?;
            std::io::copy(&mut f, &mut zip).context("zip copy")?;
        }
    }
    zip.finish().context("zip finish")?;
    Ok(())
}

/// Archive one signed `.app`, including the bundle directory as ZIP root.
pub(crate) fn zip_app_bundle(app_path: &Utf8Path, out_path: &Utf8Path) -> Result<()> {
    anyhow::ensure!(
        app_path.extension() == Some("app") && app_path.is_dir(),
        "archive source must be an existing .app directory: {app_path}"
    );
    let base = app_path.parent().context(".app has no parent")?;
    let out_file = std::fs::File::create(out_path.as_std_path())
        .with_context(|| format!("create updater archive {out_path}"))?;
    let mut zip = ZipWriter::new(out_file);

    for entry in WalkDir::new(app_path.as_std_path())
        .sort_by_file_name()
        .follow_links(false)
    {
        let entry = entry.context("walk app bundle")?;
        anyhow::ensure!(
            !entry.file_type().is_symlink(),
            "app archive does not support symlink entry {}",
            entry.path().display()
        );
        let relative = entry
            .path()
            .strip_prefix(base.as_std_path())
            .context("strip app archive prefix")?;
        let name = to_zip_name(&relative.to_string_lossy());
        if entry.file_type().is_dir() {
            zip.add_directory(&name, SimpleFileOptions::default().unix_permissions(0o755))
                .context("zip app directory")?;
            continue;
        }

        let is_executable = relative
            .components()
            .any(|component| component.as_os_str() == "MacOS");
        let mode = if is_executable { 0o755 } else { 0o644 };
        zip.start_file(
            &name,
            SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(mode),
        )
        .context("zip app file")?;
        let mut source = std::fs::File::open(entry.path()).context("open app file")?;
        std::io::copy(&mut source, &mut zip).context("copy app file")?;
    }
    zip.finish().context("finish updater archive")?;
    Ok(())
}

/// Returns an error rather than silently lossy-converting non-UTF-8 paths.
pub(crate) fn to_utf8_path(path: impl AsRef<Path>) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path.as_ref().to_path_buf())
        .map_err(|p| anyhow::anyhow!("path '{}' is not valid UTF-8", p.display()))
}

/// Human-readable form of `path` for log lines: drops the Windows verbatim
/// prefix (`\\?\C:\…` → `C:\…`, `\\?\UNC\host\share` → `\\host\share`) that
/// canonicalized paths carry. Display only — keep the original for I/O.
pub(crate) fn display_path(path: &Path) -> String {
    simplify_verbatim(&path.display().to_string())
}

fn simplify_verbatim(text: &str) -> String {
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest)
            if rest.len() >= 3
                && rest.as_bytes()[0].is_ascii_alphabetic()
                && rest.as_bytes()[1] == b':'
                && rest.as_bytes()[2] == b'\\' =>
        {
            rest.to_owned()
        },
        _ => text.to_owned(),
    }
}

#[cfg(test)]
mod display_tests {
    use super::simplify_verbatim;

    #[test]
    fn verbatim_prefixes_are_dropped_for_display() {
        assert_eq!(simplify_verbatim(r"\\?\C:\app\out.exe"), r"C:\app\out.exe");
        assert_eq!(
            simplify_verbatim(r"\\?\UNC\host\share\a"),
            r"\\host\share\a"
        );
        assert_eq!(simplify_verbatim(r"\\?\Volume{x}\a"), r"\\?\Volume{x}\a");
        assert_eq!(simplify_verbatim("/home/a/out"), "/home/a/out");
    }
}

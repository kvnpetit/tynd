//! macOS disk-image creation from an already signed and stapled application.

use std::path::Path;

use anyhow::Result;

#[cfg(target_os = "macos")]
pub(crate) fn create_from_signed_app(app: &Path, output: &Path, volume_name: &str) -> Result<()> {
    platform::create_from_signed_app(app, output, volume_name)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn create_from_signed_app(
    _app: &Path,
    _output: &Path,
    _volume_name: &str,
) -> Result<()> {
    anyhow::bail!("DMG creation requires macOS and the system hdiutil/ditto tools")
}

#[cfg(target_os = "macos")]
mod platform {
    use std::{
        ffi::OsStr,
        fs,
        os::unix::fs::symlink,
        path::Path,
        process::{Command, Output},
    };

    use anyhow::{ensure, Context, Result};

    pub(super) fn create_from_signed_app(
        app: &Path,
        output: &Path,
        volume_name: &str,
    ) -> Result<()> {
        validate_inputs(app, output, volume_name)?;

        let output_parent = output.parent().context("DMG output has no parent")?;
        fs::create_dir_all(output_parent)
            .with_context(|| format!("create DMG output directory {}", output_parent.display()))?;

        let staging = tempfile::Builder::new()
            .prefix("tynd-dmg-content-")
            .tempdir()
            .context("create temporary DMG content directory")?;
        let staged_app = staging.path().join(
            app.file_name()
                .context("signed application path has no file name")?,
        );
        run(
            Command::new("/usr/bin/ditto")
                .args(["--rsrc", "--extattr", "--acl"])
                .arg(app)
                .arg(&staged_app),
            "copy signed application with ditto",
        )?;
        symlink("/Applications", staging.path().join("Applications"))
            .context("add Applications shortcut to DMG")?;

        let image_staging = tempfile::Builder::new()
            .prefix(".tynd-dmg-")
            .tempdir_in(output_parent)
            .with_context(|| {
                format!(
                    "create temporary DMG beside final output {}",
                    output.display()
                )
            })?;
        let temporary_output = image_staging.path().join("image.dmg");
        run(
            Command::new("/usr/bin/hdiutil")
                .args(["create", "-ov", "-format", "UDZO", "-volname"])
                .arg(volume_name)
                .arg("-srcfolder")
                .arg(staging.path())
                .arg(&temporary_output),
            "create compressed DMG with hdiutil",
        )?;
        ensure!(
            temporary_output.is_file() && fs::metadata(&temporary_output)?.len() > 0,
            "hdiutil did not create a non-empty DMG at {}",
            temporary_output.display()
        );
        if output.exists() {
            let metadata = fs::symlink_metadata(output)
                .with_context(|| format!("inspect existing DMG {}", output.display()))?;
            ensure!(
                metadata.file_type().is_file() || metadata.file_type().is_symlink(),
                "refusing to replace non-file DMG output {}",
                output.display()
            );
        }
        fs::rename(&temporary_output, output)
            .with_context(|| format!("publish DMG {}", output.display()))?;
        Ok(())
    }

    fn validate_inputs(app: &Path, output: &Path, volume_name: &str) -> Result<()> {
        let metadata = fs::symlink_metadata(app)
            .with_context(|| format!("inspect signed application {}", app.display()))?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "signed application must be a real directory: {}",
            app.display()
        );
        ensure!(
            app.extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app")),
            "signed application must use the .app extension: {}",
            app.display()
        );
        ensure!(
            app.join("Contents/Info.plist").is_file(),
            "application is missing Contents/Info.plist: {}",
            app.display()
        );
        ensure!(
            output
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("dmg")),
            "DMG output must use the .dmg extension: {}",
            output.display()
        );
        ensure!(
            !volume_name.trim().is_empty()
                && volume_name.len() <= 128
                && !volume_name
                    .chars()
                    .any(|character| matches!(character, '/' | '\\' | '\0')),
            "DMG volume name must contain 1-128 bytes and no path separators"
        );
        Ok(())
    }

    fn run(command: &mut Command, operation: &str) -> Result<()> {
        let output = command
            .output()
            .with_context(|| format!("{operation}: failed to launch command"))?;
        if output.status.success() {
            return Ok(());
        }
        anyhow::bail!("{operation} failed: {}", render_output(&output))
    }

    fn render_output(output: &Output) -> String {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let details = format!("{}{}", stdout.trim(), stderr.trim());
        if details.is_empty() {
            format!("exit status {}", output.status)
        } else {
            details
        }
    }
}

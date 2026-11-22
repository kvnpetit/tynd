//! CLI output: the process-wide logger (human or JSON lines, optional log
//! file), user-facing messages, and JSON written to stdout.

use crate::args::LogFormat;
use anyhow::{bail, Result};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

struct Logger {
    verbose: bool,
    quiet: bool,
    format: LogFormat,
    file: Option<Mutex<File>>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub(crate) fn configure(
    verbose: bool,
    quiet: bool,
    format: LogFormat,
    log_file: Option<&Path>,
) -> Result<()> {
    let file = log_file
        .map(|path| -> Result<Mutex<File>> {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            if let Ok(metadata) = fs::symlink_metadata(path) {
                if metadata.file_type().is_symlink() {
                    bail!("refusing symbolic log file {}", path.display());
                }
            }
            Ok(Mutex::new(
                OpenOptions::new().create(true).append(true).open(path)?,
            ))
        })
        .transpose()?;
    LOGGER
        .set(Logger {
            verbose,
            quiet,
            format,
            file,
        })
        .map_err(|_| anyhow::anyhow!("CLI logger is already configured"))?;
    Ok(())
}

fn logger() -> &'static Logger {
    LOGGER.get_or_init(|| Logger {
        verbose: false,
        quiet: false,
        format: LogFormat::Human,
        file: None,
    })
}

/// Path as shown to the user, without the Windows verbatim prefix (`\\?\`)
/// that `fs::canonicalize` adds. Display only: keep the canonical path for I/O.
pub(crate) fn display_path(path: &Path) -> std::path::Display<'_> {
    dunce::simplified(path).display()
}

pub(crate) fn step(message: impl AsRef<str>) {
    emit("info", "step", "→", message.as_ref(), false);
}

pub(crate) fn success(message: impl AsRef<str>) {
    emit("info", "success", "✓", message.as_ref(), false);
}

pub(crate) fn warn(message: impl AsRef<str>) {
    emit("warn", "warning", "!", message.as_ref(), false);
}

pub(crate) fn debug(message: impl AsRef<str>) {
    emit("debug", "debug", "·", message.as_ref(), true);
}

pub(crate) fn quiet() -> bool {
    logger().quiet
}

pub(crate) fn verbose() -> bool {
    logger().verbose
}

fn emit(level: &str, kind: &str, symbol: &str, message: &str, debug_only: bool) {
    let logger = logger();
    if debug_only && !logger.verbose {
        return;
    }
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let line = match logger.format {
        LogFormat::Human => format!("{symbol} {message}"),
        LogFormat::Json => serde_json::json!({
            "timestampMs": timestamp_ms,
            "level": level,
            "kind": kind,
            "message": message
        })
        .to_string(),
    };
    if !logger.quiet {
        eprintln!("{line}");
    }
    if let Some(file) = &logger.file {
        if let Ok(mut file) = file.lock() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

pub(crate) fn write_json(value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_stdout(&bytes)
}

/// Writes `bytes` to stdout and flushes. A reader that closes the pipe early
/// (`tynd completions bash | head -3`) is not an error: the output it wanted
/// was delivered, so `BrokenPipe` counts as success.
pub(crate) fn write_stdout(bytes: &[u8]) -> Result<()> {
    write_ignoring_broken_pipe(&mut io::stdout().lock(), bytes)?;
    Ok(())
}

/// Writes one line of command output to stdout, with [`write_stdout`]'s
/// closed-pipe handling. Diagnostics go to stderr through [`step`]/[`warn`].
pub(crate) fn write_line(line: impl std::fmt::Display) -> Result<()> {
    write_stdout(format!("{line}\n").as_bytes())
}

fn write_ignoring_broken_pipe(out: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    match out.write_all(bytes).and_then(|()| out.flush()) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use super::write_ignoring_broken_pipe;

    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_closed_pipe_is_not_an_error() {
        assert!(write_ignoring_broken_pipe(&mut ClosedPipe, b"script").is_ok());
    }

    #[test]
    fn other_write_errors_propagate() {
        let mut full = [0_u8; 2];
        let error = write_ignoring_broken_pipe(&mut full.as_mut_slice(), b"script")
            .expect_err("a full buffer fails");
        assert_eq!(error.kind(), io::ErrorKind::WriteZero);
    }
}

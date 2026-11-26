use std::{
    fmt::Display,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use cap_std::fs::Dir;
use serde_json::{json, Value};

use crate::ipc::error::IpcError;

use super::{
    check_url_scheme, header_str, string_headers,
    transport::{execute_watched, AbortHandle, AbortRead, Net, Outgoing},
    DownloadArgs, ProgressEmitter, DEFAULT_CONNECT_TIMEOUT, DOWNLOAD_IDLE_TIMEOUT,
};

const MAX_DOWNLOAD: u64 = 10 * 1024 * 1024 * 1024;

/// `http.download: <msg>` error string.
fn download_error(msg: impl Display) -> String {
    IpcError::new("http", "download", msg).into_string()
}

pub(super) fn download(args: Value) -> Result<Value, String> {
    let mut a: DownloadArgs = serde_json::from_value(args)
        .map_err(|e| download_error(format!("invalid arguments: {e}")))?;
    let url = a.url.as_str();
    check_url_scheme(url, "download")?;
    crate::security::policy::check_http(url)?;
    let dest = a.dest.as_str();
    crate::security::policy::check_fs(dest)?;
    let parsed = url::Url::parse(url).map_err(|e| download_error(format!("invalid URL: {e}")))?;

    // No overall deadline by default (a large file streams for as long as
    // bytes keep coming); without `timeoutMs`, connecting is still bounded.
    let mut net = Net::parse(&a.net, "download", None)?;
    if net.timeout.is_none() && net.connect_timeout.is_none() {
        net.connect_timeout = Some(DEFAULT_CONNECT_TIMEOUT);
    }
    let abort = AbortHandle::register(a.net.abort_id.take(), "download")?;
    let agent = net.agent(Some(DOWNLOAD_IDLE_TIMEOUT));
    let outgoing = Outgoing {
        method: "GET".to_owned(),
        url: parsed,
        headers: string_headers(a.headers.as_ref())
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        body: None,
        progress_id: None,
    };
    let mut resp = execute_watched(agent, outgoing, net, &abort, None, "download")?.response;

    // A non-2xx body is an error page, not the requested file: fail
    // before touching the destination.
    let status = resp.status();
    if !status.is_success() {
        return Err(download_error(format!(
            "({url}): HTTP status {}",
            status.as_u16()
        )));
    }

    let total: Option<u64> =
        header_str(&resp, "content-length").and_then(|s| s.parse::<u64>().ok());
    if let Some(t) = total.filter(|&t| t > MAX_DOWNLOAD) {
        return Err(download_error(format!(
            "response size {t} exceeds the {MAX_DOWNLOAD}-byte cap"
        )));
    }

    let (cap_dir, rel) = crate::api::fs::resolve(dest)?;
    let mut reader = AbortRead {
        inner: resp.body_mut().as_reader(),
        flag: abort.flag(),
    }
    .take(MAX_DOWNLOAD + 1);
    let mut emitter = ProgressEmitter::new(a.progress_id, "download", total);
    let written = write_atomically(cap_dir, &rel, &mut reader, &mut emitter).map_err(|e| {
        if abort.is_aborted() {
            download_error("aborted")
        } else {
            e
        }
    })?;
    Ok(json!({ "path": dest, "bytes": written }))
}

/// Stream `reader` into a sibling temp file, fsync it, then rename it over
/// `rel`, creating missing parent directories first. The temp file is
/// removed on every failure path.
fn write_atomically(
    dir: &Dir,
    rel: &Path,
    reader: &mut impl Read,
    emitter: &mut ProgressEmitter,
) -> Result<u64, String> {
    if let Some(parent) = rel.parent().filter(|p| !p.as_os_str().is_empty()) {
        dir.create_dir_all(parent)
            .map_err(|e| download_error(format!("mkdir {}: {e}", parent.display())))?;
    }
    let tmp_rel = download_temp_path(rel);
    let mut tmp = dir
        .create(&tmp_rel)
        .map_err(|e| download_error(format!("tempfile {}: {e}", tmp_rel.display())))?;
    // Removes the temp file on every early return; disarmed only once
    // the rename has moved it into place.
    let mut tmp_guard = TempFileGuard {
        dir,
        rel: &tmp_rel,
        armed: true,
    };
    let written = stream_body(reader, &mut tmp, emitter)?;
    tmp.flush()
        .map_err(|e| download_error(format!("flush: {e}")))?;
    tmp.sync_all()
        .map_err(|e| download_error(format!("fsync: {e}")))?;
    drop(tmp);
    dir.rename(&tmp_rel, dir, rel).map_err(|e| {
        download_error(format!(
            "rename {} -> {}: {e}",
            tmp_rel.display(),
            rel.display()
        ))
    })?;
    tmp_guard.armed = false;
    Ok(written)
}

/// Copies the response body into `out`, enforcing [`MAX_DOWNLOAD`] and
/// reporting progress; returns the byte count.
fn stream_body(
    reader: &mut impl Read,
    out: &mut impl Write,
    emitter: &mut ProgressEmitter,
) -> Result<u64, String> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| download_error(format!("read: {e}")))?;
        if n == 0 {
            emitter.advance(0, true);
            return Ok(written);
        }
        out.write_all(&buf[..n])
            .map_err(|e| download_error(format!("write: {e}")))?;
        written += n as u64;
        if written > MAX_DOWNLOAD {
            return Err(download_error(format!(
                "response body exceeds the {MAX_DOWNLOAD}-byte cap"
            )));
        }
        emitter.advance(n, false);
    }
}

/// Deletes the download temp file when dropped while still armed.
struct TempFileGuard<'a> {
    dir: &'a Dir,
    rel: &'a Path,
    armed: bool,
}

impl Drop for TempFileGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.dir.remove_file(self.rel);
        }
    }
}

fn download_temp_path(destination: &Path) -> PathBuf {
    let mut temporary = destination.to_path_buf();
    let mut name = destination
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(format!(".tmp.{}", temp_suffix()));
    temporary.set_file_name(name);
    temporary
}

fn temp_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let pid = u64::from(std::process::id());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{pid:x}-{nanos:x}-{n:x}")
}

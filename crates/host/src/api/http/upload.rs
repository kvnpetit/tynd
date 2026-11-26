use std::{fmt::Display, fs, io::Read, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

use crate::ipc::error::IpcError;

use super::{
    build_agent, check_url_scheme, string_headers, ProgressEmitter, UploadArgs, MAX_RESPONSE_BODY,
};

/// `http.upload: <msg>` error string.
fn upload_error(msg: impl Display) -> String {
    IpcError::new("http", "upload", msg).into_string()
}

/// Escape `\` and `"` for a quoted multipart header parameter; reject
/// CR / LF / NUL, which would let the value inject headers.
pub(super) fn sanitize_multipart_param(value: &str) -> Result<String, String> {
    if value.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0)) {
        return Err(upload_error(
            "field name / filename contains forbidden control character (CR/LF/NUL)",
        ));
    }
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    Ok(out)
}

/// Random multipart boundary (`----TyndFormBoundary<26 hex>`).
pub(super) fn make_boundary() -> String {
    let mut rng_bytes = [0u8; 13];
    // OS CSPRNG failure is unrecoverable and practically unreachable.
    getrandom::fill(&mut rng_bytes).expect("OS CSPRNG unavailable");
    let mut s = String::with_capacity(20 + rng_bytes.len() * 2);
    s.push_str("----TyndFormBoundary");
    s.push_str(&crate::util::hex_lower(&rng_bytes));
    s
}

/// Streaming `multipart/form-data` body: preamble, file bytes, epilogue,
/// reporting upload progress as it is read.
pub(super) struct MultipartBody {
    pub(super) preamble: std::io::Cursor<Vec<u8>>,
    pub(super) file: Option<fs::File>,
    pub(super) epilogue: std::io::Cursor<Vec<u8>>,
    pub(super) emitter: ProgressEmitter,
}

impl Read for MultipartBody {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.preamble.read(buf)?;
        if n > 0 {
            self.emitter.advance(n, false);
            return Ok(n);
        }
        if let Some(f) = self.file.as_mut() {
            let n = f.read(buf)?;
            if n > 0 {
                self.emitter.advance(n, false);
                return Ok(n);
            }
            self.file = None;
        }
        let n = self.epilogue.read(buf)?;
        if n > 0 {
            self.emitter.advance(n, false);
        } else {
            self.emitter.advance(0, true);
        }
        Ok(n)
    }
}

pub(super) fn upload(args: Value) -> Result<Value, String> {
    let a: UploadArgs = serde_json::from_value(args)
        .map_err(|e| upload_error(format!("invalid arguments: {e}")))?;
    let url = a.url.as_str();
    check_url_scheme(url, "upload")?;
    crate::security::policy::check_http(url)?;
    let file_path = a.file_path.as_str();
    crate::security::policy::check_fs(file_path)?;
    let field_name = sanitize_multipart_param(a.field_name.as_deref().unwrap_or("file"))?;

    let (cap_dir, rel) = crate::api::fs::resolve(file_path)?;
    let file = cap_dir
        .open(&rel)
        .map_err(|e| upload_error(format!("open {file_path}: {e}")))?
        .into_std();
    let file_len = file
        .metadata()
        .map_err(|e| upload_error(format!("stat {file_path}: {e}")))?
        .len();

    let boundary = make_boundary();
    let (preamble, epilogue) = multipart_frame(&boundary, &field_name, Path::new(file_path))?;
    let total_len = preamble.len() as u64 + file_len + epilogue.len() as u64;

    let agent = build_agent(a.timeout_ms);
    let req_builder = string_headers(a.headers.as_ref())
        .fold(agent.post(url), |req, (name, value)| {
            req.header(name, value)
        })
        .header(
            "content-type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .header("content-length", &total_len.to_string());

    let mut body = MultipartBody {
        preamble: std::io::Cursor::new(preamble.into_bytes()),
        file: Some(file),
        epilogue: std::io::Cursor::new(epilogue.into_bytes()),
        emitter: ProgressEmitter::new(a.progress_id, "upload", Some(total_len)),
    };

    let mut resp = req_builder
        .send(ureq::SendBody::from_reader(&mut body))
        .map_err(|e| upload_error(format!("({url}): {e}")))?;

    let status = resp.status().as_u16();
    let mut buf = Vec::new();
    resp.body_mut()
        .as_reader()
        .take((MAX_RESPONSE_BODY + 1) as u64)
        .read_to_end(&mut buf)
        .map_err(|e| upload_error(format!("read response: {e}")))?;
    if buf.len() > MAX_RESPONSE_BODY {
        return Err(upload_error(format!(
            "response body exceeds the {MAX_RESPONSE_BODY}-byte cap"
        )));
    }

    Ok(json!({
        "status": status,
        "body": STANDARD.encode(&buf),
    }))
}

/// Multipart preamble (part headers for the file) and epilogue around the
/// file bytes. `field_name` must already be sanitized.
fn multipart_frame(
    boundary: &str,
    field_name: &str,
    file_path: &Path,
) -> Result<(String, String), String> {
    let raw_filename = file_path
        .file_name()
        .map_or_else(|| "file".to_string(), |s| s.to_string_lossy().into_owned());
    let filename = sanitize_multipart_param(&raw_filename)?;
    let content_type = crate::util::mime_for_path(file_path);
    let preamble = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{field_name}\"; \
         filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
    );
    let epilogue = format!("\r\n--{boundary}--\r\n");
    Ok((preamble, epilogue))
}

use std::{fmt::Display, io::Read, mem};

use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::{Bytes, BytesMut};
use serde_json::{json, Value};

use crate::ipc::error::IpcError;

use super::{
    build_agent, check_url_scheme, header_str, string_headers, ProgressEmitter, ProgressReader,
    RequestArgs, MAX_REQUEST_BODY, MAX_RESPONSE_BODY,
};

type Response = ureq::http::Response<ureq::Body>;

/// `http.request: <msg>` error string.
fn request_error(msg: impl Display) -> String {
    IpcError::new("http", "request", msg).into_string()
}

fn build_error(error: impl Display) -> String {
    request_error(format!("build request: {error}"))
}

pub(super) fn request(args: Value) -> Result<Value, String> {
    let a: RequestArgs = serde_json::from_value(args)
        .map_err(|e| request_error(format!("invalid arguments: {e}")))?;
    let url = a.url.as_str();
    check_url_scheme(url, "request")?;
    crate::security::policy::check_http(url)?;
    let method = a.method.as_deref().unwrap_or("GET");
    let agent = build_agent(a.timeout_ms);

    // ureq 3 doesn't expose a generic `agent.request(method, url)`;
    // build an `http::Request` directly for runtime-selected verbs.
    let builder = string_headers(a.headers.as_ref()).fold(
        ureq::http::Request::builder().method(method).uri(url),
        |builder, (name, value)| builder.header(name, value),
    );
    let mut resp = send(&agent, builder, a.body.as_ref(), a.progress_id.clone())?
        .map_err(|e| request_error(format!("({url}): {e}")))?;
    let status = resp.status().as_u16();
    let status_text = resp.status().canonical_reason().unwrap_or("").to_string();
    let total: Option<u64> =
        header_str(&resp, "content-length").and_then(|s| s.parse::<u64>().ok());
    let headers_map = response_headers(&resp);
    let response_type = a.response_type.as_deref().unwrap_or("text");
    let body_bytes = read_response_body(&mut resp, a.progress_id, total)?;
    let body = decode_response_body(&body_bytes, response_type)?;

    Ok(json!({
        "status": status,
        "statusText": status_text,
        "headers": Value::Object(headers_map),
        "body": body,
    }))
}

/// Attach `body` (if any) and run the request. The outer `Err` is a local
/// failure (oversized or unencodable body, invalid request); the inner
/// result is ureq's.
fn send(
    agent: &ureq::Agent,
    mut builder: ureq::http::request::Builder,
    body: Option<&Value>,
    progress_id: Option<String>,
) -> Result<Result<Response, ureq::Error>, String> {
    let Some(body) = body else {
        let req = builder.body(ureq::SendBody::none()).map_err(build_error)?;
        return Ok(agent.run(req));
    };
    let (body_bytes, content_type_set) = encode_body(body, &builder)?;
    if body_bytes.len() > MAX_REQUEST_BODY {
        return Err(request_error(format!(
            "request body of {} bytes exceeds the {MAX_REQUEST_BODY}-byte cap",
            body_bytes.len()
        )));
    }
    if !content_type_set {
        builder = builder.header("content-type", "application/json");
    }
    if progress_id.is_none() {
        let req = builder
            .body(ureq::SendBody::from_owned_reader(std::io::Cursor::new(
                body_bytes,
            )))
            .map_err(build_error)?;
        return Ok(agent.run(req));
    }
    let total = Some(body_bytes.len() as u64);
    let mut reader = ProgressReader {
        inner: std::io::Cursor::new(body_bytes),
        emitter: ProgressEmitter::new(progress_id, "upload", total),
    };
    let req = builder
        .body(ureq::SendBody::from_reader(&mut reader))
        .map_err(build_error)?;
    Ok(agent.run(req))
}

/// Body bytes plus whether a content type is already settled: strings are
/// sent verbatim (no default content type), anything else is JSON-encoded
/// and needs one unless the caller supplied a `content-type` header.
fn encode_body(
    body: &Value,
    builder: &ureq::http::request::Builder,
) -> Result<(Vec<u8>, bool), String> {
    match body {
        Value::String(s) => Ok((s.as_bytes().to_vec(), true)),
        other => {
            let json_body = serde_json::to_vec(other).map_err(|e| e.to_string())?;
            let has_content_type = builder.headers_ref().is_some_and(|headers| {
                headers
                    .keys()
                    .any(|name| name.as_str().eq_ignore_ascii_case("content-type"))
            });
            Ok((json_body, has_content_type))
        },
    }
}

fn response_headers(response: &Response) -> serde_json::Map<String, Value> {
    let mut headers = serde_json::Map::new();
    for (name, value) in response.headers() {
        let Ok(value) = value.to_str() else {
            continue;
        };
        let key = name.as_str().to_ascii_lowercase();
        match headers.entry(&key) {
            serde_json::map::Entry::Vacant(entry) => {
                entry.insert(Value::String(value.to_string()));
            },
            serde_json::map::Entry::Occupied(mut entry) => {
                let current = entry.get_mut();
                if let Value::Array(values) = current {
                    values.push(Value::String(value.to_string()));
                } else {
                    let previous = mem::replace(current, Value::Null);
                    *current = Value::Array(vec![previous, Value::String(value.to_string())]);
                }
            },
        }
    }
    headers
}

fn read_response_body(
    response: &mut Response,
    progress_id: Option<String>,
    total: Option<u64>,
) -> Result<Bytes, String> {
    let mut buffer = BytesMut::with_capacity(total.unwrap_or(0).min(8 * 1024 * 1024) as usize);
    let mut reader = response
        .body_mut()
        .as_reader()
        .take((MAX_RESPONSE_BODY + 1) as u64);
    let mut emitter = ProgressEmitter::new(progress_id, "download", total);
    let mut chunk = vec![0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| request_error(format!("read body: {error}")))?;
        if read == 0 {
            emitter.advance(0, true);
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_RESPONSE_BODY {
            return Err(request_error(format!(
                "response body exceeds the {MAX_RESPONSE_BODY}-byte cap"
            )));
        }
        emitter.advance(read, false);
    }
    Ok(buffer.freeze())
}

fn decode_response_body(body: &Bytes, response_type: &str) -> Result<Value, String> {
    match response_type {
        "binary" => Ok(Value::String(STANDARD.encode(body))),
        "json" => serde_json::from_slice(body).map_err(|error| {
            IpcError::new(
                "http",
                "fetch",
                format!("response is not valid JSON: {error}"),
            )
            .into_string()
        }),
        _ => Ok(Value::String(String::from_utf8_lossy(body).into_owned())),
    }
}

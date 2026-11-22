//! JSON-RPC 2.0 message envelope shared by every debug method.
//!
//! Wire format: 4-byte little-endian length prefix + JSON body. MCP
//! clients can reuse a single length-prefixed read-loop primitive.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Maximum frame body size (16 MiB). Guards against a hostile client
/// sending a 4 GiB length and starving the host of memory.
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// JSON-RPC 2.0 inbound request (or notification — `id == None`).
#[derive(Debug, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub id: Option<Value>,
}

/// JSON-RPC 2.0 outbound response.
#[derive(Debug, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// Carries subscription frames pushed by the server.
#[derive(Debug, Serialize)]
pub struct Notification {
    pub jsonrpc: &'static str,
    pub method: &'static str,
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn parse_error() -> Self {
        Self::new(-32700, "parse error")
    }

    pub fn invalid_request() -> Self {
        Self::new(-32600, "invalid request")
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(-32601, format!("method not found: {method}"))
    }

    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self::new(-32602, msg)
    }

    pub fn internal_error(msg: impl Into<String>) -> Self {
        Self::new(-32603, msg)
    }

    pub fn already_attached() -> Self {
        Self::new(-32099, "already_attached")
    }
}

impl Response {
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Option<Value>, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// Read one length-prefixed frame from `r`. Returns `Ok(None)` on a
/// clean EOF (peer closed before the next frame), `Err(...)` on a
/// partial frame or oversized length.
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read(&mut len_buf[..1]) {
        Ok(0) => return Ok(None),
        Ok(1) => {},
        Ok(_) => unreachable!("one-byte read returned more than one byte"),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => return read_frame(r),
        Err(e) => return Err(e),
    }
    r.read_exact(&mut len_buf[1..])?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {len} > {MAX_FRAME_BYTES}"),
        ));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

/// Write `body` as a length-prefixed frame.
pub fn write_frame<W: Write + ?Sized>(w: &mut W, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame body > u32::MAX"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(body)?;
    w.flush()
}

/// Encode a Response or Notification as a JSON byte buffer ready for
/// `write_frame`.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    serde_json::to_vec(value).map_err(|e| format!("encode: {e}"))
}

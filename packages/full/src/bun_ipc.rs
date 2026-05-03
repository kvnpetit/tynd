//! Bun↔host IPC wire-format selector (U1 Layer 1).
//!
//! Two formats live side by side:
//!
//! - **JSON** (default, always-on): `\n`-delimited JSON lines, the legacy
//!   text wire that the Bun runtime has used since day one. Backward
//!   compatible with every existing app.
//! - **CBOR** (opt-in, `binary-ipc` feature): length-prefixed CBOR
//!   frames. ~10× faster decode on typed payloads, 30-50% smaller wire
//!   size. RFC 8949, mature JS encoders (`cbor-x` on the Bun side).
//!
//! ## Selection
//!
//! Format is picked once at process start by reading `TYND_IPC_FORMAT`:
//!
//! - `TYND_IPC_FORMAT=cbor` (and `binary-ipc` feature on) → CBOR frames.
//! - anything else / unset → JSON lines (current behaviour).
//!
//! The Bun child inherits the env var so both sides agree on the format
//! before the first frame is written. The Rust spawner sets the var
//! explicitly to make the contract auditable in process listings.
//!
//! ## CBOR frame format
//!
//! ```text
//! [u32 LE: body_len][body: CBOR-encoded BunWire payload]
//! ```
//!
//! `body_len` is exclusive of the 4-byte header. A `body_len` of 0 is
//! invalid and treated as a framing error.
//!
//! ## Why CBOR over postcard
//!
//! The hybrid recommendation in `OPTIMIZATION_BREAKING_OK.md` defaulted
//! to postcard, but CBOR's TS ecosystem (`cbor-x`, native `cbor` in
//! Deno) is more mature than postcard's experimental TS port. Wire
//! perf is comparable (both varint + length-prefix, both schema-less);
//! CBOR wins on tooling and debug ergonomics (any HTTP debugger can
//! decode it inline).

#![cfg_attr(not(feature = "binary-ipc"), allow(dead_code))]

#[cfg(feature = "binary-ipc")]
use std::io::{Read, Write};

/// Wire format negotiated for the lifetime of the process. Sampled
/// once at startup from `TYND_IPC_FORMAT` and cached so we don't pay
/// an env lookup on every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireFormat {
    /// Newline-delimited JSON. Default, always available.
    Json,
    /// Length-prefixed CBOR. Requires the `binary-ipc` feature.
    #[cfg(feature = "binary-ipc")]
    Cbor,
}

impl WireFormat {
    /// Resolve the wire format from `TYND_IPC_FORMAT`. Falls back to
    /// JSON when the env var is missing, set to `json`, or set to
    /// `cbor` while the `binary-ipc` feature is off (so a misconfig
    /// can't dead-lock the IPC channel — the host quietly stays on
    /// the format the Bun side will fall back to as well).
    pub(crate) fn from_env() -> Self {
        match std::env::var("TYND_IPC_FORMAT").as_deref() {
            #[cfg(feature = "binary-ipc")]
            Ok("cbor") => Self::Cbor,
            _ => Self::Json,
        }
    }

    /// Stringly tag for logs and the env var the Bun child inherits.
    /// Used by the spawner to make the contract explicit in process
    /// listings instead of relying on the env-var being inherited
    /// from the parent shell.
    pub(crate) fn as_env_value(self) -> &'static str {
        match self {
            Self::Json => "json",
            #[cfg(feature = "binary-ipc")]
            Self::Cbor => "cbor",
        }
    }
}

// ---------------------------------------------------------------------------
// CBOR frame I/O — used only when `binary-ipc` is on AND the env var
// asks for CBOR. The functions are conditionally compiled to keep
// the binary size of default builds untouched.
// ---------------------------------------------------------------------------

/// Hard cap on a single CBOR frame (8 MiB). Mirrors the JS-side
/// `MAX_STDIN_BUFFER` so a malformed length prefix can't pin
/// arbitrary RAM in the reader thread waiting for `body_len` bytes
/// that will never arrive. Anything beyond this is a framing bug or
/// an attack and we kill the IPC channel by returning an error.
#[cfg(feature = "binary-ipc")]
pub(crate) const MAX_CBOR_FRAME: usize = 8 * 1024 * 1024;

/// Write a length-prefixed CBOR frame to `out`. The body is encoded
/// in a scratch buffer first so the prefix can carry the exact body
/// length — CBOR is self-delimiting but the wire reader prefers a
/// length prefix to avoid streaming-decode state across reads.
///
/// Currently only the writer-side coalescer in `bun.rs` needs this
/// (via [`push_cbor_frame_into`] which sidesteps the full writer
/// because the body is already encoded). Kept as a generic helper
/// for the property tests and any future direct-write call site.
#[cfg(feature = "binary-ipc")]
#[allow(dead_code)]
pub(crate) fn write_cbor_frame<W, T>(out: &mut W, value: &T) -> std::io::Result<()>
where
    W: Write,
    T: serde::Serialize,
{
    let mut body: Vec<u8> = Vec::with_capacity(256);
    ciborium::ser::into_writer(value, &mut body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    if body.len() > MAX_CBOR_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("CBOR frame body {} exceeds cap {}", body.len(), MAX_CBOR_FRAME),
        ));
    }
    let len = u32::try_from(body.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "frame body > u32"))?;
    out.write_all(&len.to_le_bytes())?;
    out.write_all(&body)?;
    Ok(())
}

/// Append an already-CBOR-encoded body as a length-prefixed frame to
/// `out`. Used by the burst coalescer in `bun.rs` so multiple frames
/// can be flushed in a single `write_all` syscall (B18 parity for
/// the binary path).
#[cfg(feature = "binary-ipc")]
pub(crate) fn push_cbor_frame_into(out: &mut Vec<u8>, body: &[u8]) {
    let len = body.len() as u32;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(body);
}

/// Read one length-prefixed CBOR frame from `reader`. Returns `Ok(None)`
/// on clean EOF (Bun closed stdout — same semantic as `read_until`
/// returning 0 on the JSON path). Errors on a truncated header or a
/// body that exceeds the per-frame cap.
#[cfg(feature = "binary-ipc")]
pub(crate) fn read_cbor_frame<R: Read>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let mut hdr = [0u8; 4];
    match read_exact_or_eof(reader, &mut hdr)? {
        0 => return Ok(None),
        4 => {},
        n => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("truncated CBOR frame header: {n}/4 bytes"),
            ));
        },
    }
    let body_len = u32::from_le_bytes(hdr) as usize;
    if body_len == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "CBOR frame body length is 0",
        ));
    }
    if body_len > MAX_CBOR_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("CBOR frame body {body_len} exceeds cap {MAX_CBOR_FRAME}"),
        ));
    }
    let mut body = vec![0u8; body_len];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

/// `read_exact` flavour that returns the number of bytes read on
/// short reads instead of an error. Returns `Ok(0)` on clean EOF
/// before the first byte (so the caller can distinguish "no frame
/// pending" from "truncated frame"). Anything in between is treated
/// as a truncated header by the caller.
#[cfg(feature = "binary-ipc")]
fn read_exact_or_eof<R: Read>(reader: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match reader.read(&mut buf[total..]) {
            Ok(0) => return Ok(total),
            Ok(n) => total += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

#[cfg(all(test, feature = "binary-ipc"))]
mod tests {
    use std::io::Cursor;

    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Probe {
        id: String,
        n: u32,
    }

    #[test]
    fn round_trip_single_frame() {
        let mut buf: Vec<u8> = Vec::new();
        let p = Probe {
            id: "abc".into(),
            n: 7,
        };
        write_cbor_frame(&mut buf, &p).unwrap();
        let mut cur = Cursor::new(&buf);
        let body = read_cbor_frame(&mut cur).unwrap().unwrap();
        let decoded: Probe = ciborium::de::from_reader(&body[..]).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn round_trip_burst() {
        let mut buf: Vec<u8> = Vec::new();
        for i in 0..5u32 {
            write_cbor_frame(&mut buf, &Probe {
                id: format!("id-{i}"),
                n: i,
            })
            .unwrap();
        }
        let mut cur = Cursor::new(&buf);
        for i in 0..5u32 {
            let body = read_cbor_frame(&mut cur).unwrap().unwrap();
            let p: Probe = ciborium::de::from_reader(&body[..]).unwrap();
            assert_eq!(p.n, i);
        }
        assert!(read_cbor_frame(&mut cur).unwrap().is_none());
    }

    #[test]
    fn rejects_zero_length_frame() {
        let buf = [0u8, 0, 0, 0];
        let mut cur = Cursor::new(&buf[..]);
        assert!(read_cbor_frame(&mut cur).is_err());
    }

    #[test]
    fn rejects_oversized_frame() {
        let mut buf: Vec<u8> = Vec::new();
        let huge = (MAX_CBOR_FRAME + 1) as u32;
        buf.extend_from_slice(&huge.to_le_bytes());
        let mut cur = Cursor::new(&buf[..]);
        assert!(read_cbor_frame(&mut cur).is_err());
    }

    #[test]
    fn truncated_header_errors() {
        let buf = [1u8, 2];
        let mut cur = Cursor::new(&buf[..]);
        assert!(read_cbor_frame(&mut cur).is_err());
    }

    /// Sanity: a frame produced by `cbor-x` (the JS Bun-side encoder)
    /// round-trips cleanly through `ciborium` here. Captured from a
    /// `bun -e "import { encodeCborFrame } from './ipc-frame.ts'; ..."`
    /// run on the matching `Call`-shape payload — keeps the two
    /// encoders' interop pinned by an actual on-the-wire byte string,
    /// not a self-test that would silently drift if both sides break
    /// in lockstep.
    #[test]
    fn cbor_x_call_frame_decodes_via_ciborium() {
        // [u32 LE: 37 body bytes][CBOR map { type, id, fn, args }]
        // Frame body produced by `cbor-x` Encoder({ useRecords: false,
        // tagUint8Array: false }) on
        // `{ type: "call", id: "x", fn: "h", args: [1, "hello", true] }`.
        let bytes: &[u8] = &[
            0x25, 0x00, 0x00, 0x00, // body_len = 37
            0xb9, 0x00, 0x04, // map(4)
            0x64, b't', b'y', b'p', b'e', // "type"
            0x64, b'c', b'a', b'l', b'l', // "call"
            0x62, b'i', b'd', // "id"
            0x61, b'x', // "x"
            0x62, b'f', b'n', // "fn"
            0x61, b'h', // "h"
            0x64, b'a', b'r', b'g', b's', // "args"
            0x83, 0x01, 0x65, b'h', b'e', b'l', b'l', b'o', 0xf5, // [1, "hello", true]
        ];
        let mut cur = Cursor::new(bytes);
        let body = read_cbor_frame(&mut cur).unwrap().unwrap();
        // Decode into a free-form ciborium::Value so the test doesn't
        // depend on the WireMsg shape (which uses #[serde(tag="type")]
        // — same on-wire shape but the assertion is cleaner with the
        // generic type).
        let v: ciborium::Value = ciborium::de::from_reader(&body[..]).unwrap();
        let map = v.as_map().expect("map");
        assert_eq!(map.len(), 4);
        // Spot-check a couple of fields — full structural equality is
        // covered by the round-trip tests above; this one only proves
        // the cross-language byte string parses.
        let kind = map
            .iter()
            .find(|(k, _)| k.as_text() == Some("type"))
            .and_then(|(_, v)| v.as_text())
            .unwrap();
        assert_eq!(kind, "call");
    }

    #[test]
    fn from_env_defaults_json_without_var() {
        // Snapshot whatever the test runner inherited and restore
        // afterwards so we don't leak state to sibling tests.
        let prev = std::env::var("TYND_IPC_FORMAT").ok();
        std::env::remove_var("TYND_IPC_FORMAT");
        assert_eq!(WireFormat::from_env(), WireFormat::Json);
        std::env::set_var("TYND_IPC_FORMAT", "cbor");
        assert_eq!(WireFormat::from_env(), WireFormat::Cbor);
        std::env::set_var("TYND_IPC_FORMAT", "garbage");
        assert_eq!(WireFormat::from_env(), WireFormat::Json);
        match prev {
            Some(v) => std::env::set_var("TYND_IPC_FORMAT", v),
            None => std::env::remove_var("TYND_IPC_FORMAT"),
        }
    }
}

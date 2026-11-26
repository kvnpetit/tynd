//! `compute` OS API: CSPRNG bytes over IPC, plus the hash helper behind the
//! `tynd-bin://` scheme.
//!
//! Hash / compress / decompress take multi-MB payloads, so they route
//! through the `tynd-bin://` custom protocol (`crate::webview::scheme_bin`) rather
//! than the JSON dispatch table.

use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::api::HostApi;
use crate::ipc::error::IpcError;
use crate::tynd;

/// Stateless dispatch marker.
#[derive(Debug, Default)]
pub(crate) struct ComputeApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("compute", |method, args| dispatch(method, &args));

#[tynd::handler(api = "compute")]
impl ComputeApi {
    /// Generate `n` random bytes from the OS CSPRNG, base64-encoded.
    /// `n` is bounded to `1..=1_048_576` to keep the response under the
    /// IPC frame ceiling (the host IPC bridge caps at
    /// ~2 MB per message after base64 expansion).
    pub fn random_bytes(n: u64) -> Result<String, String> {
        let buf = random_bytes_raw(n as usize)?;
        Ok(STANDARD.encode(&buf))
    }
}

/// Fill `n` bytes from the OS CSPRNG. Used by both the frontend dispatch
/// path and backend `crypto.getRandomValues`.
pub(crate) fn random_bytes_raw(n: usize) -> Result<Vec<u8>, String> {
    if n == 0 || n > 1_048_576 {
        return Err(IpcError::new(
            "compute",
            "randomBytes",
            format!("'n' must be 1..=1_048_576 (got {n})"),
        )
        .into());
    }
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf)
        .map_err(|e| IpcError::new("compute", "randomBytes", format!("OS CSPRNG: {e}")))?;
    Ok(buf)
}

/// Hash raw bytes with `algo` (blake3 / sha256 / sha384 / sha512) and encode the digest
/// with `encoding` (hex / base64). Called by the `tynd-bin://` scheme handler.
pub(crate) fn hash_raw(data: &[u8], algo: &str, encoding: &str) -> Result<String, String> {
    let digest: Vec<u8> = match algo {
        "blake3" => blake3::hash(data).as_bytes().to_vec(),
        "sha256" => Sha256::digest(data).to_vec(),
        "sha384" => Sha384::digest(data).to_vec(),
        "sha512" => Sha512::digest(data).to_vec(),
        _ => {
            return Err(
                IpcError::new("compute", "hash", format!("unsupported algo '{algo}'")).into(),
            )
        },
    };
    match encoding {
        "hex" => Ok(crate::util::hex_lower(&digest)),
        "base64" => Ok(STANDARD.encode(&digest)),
        _ => Err(IpcError::new(
            "compute",
            "hash",
            format!("unsupported encoding '{encoding}'"),
        )
        .into()),
    }
}

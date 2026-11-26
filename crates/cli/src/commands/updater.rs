//! `tynd updater`: signing keys, signatures, signed manifests and the update
//! server.

mod keygen;
mod publish;
mod release;
mod server;
mod sign;

pub(super) use keygen::keygen;
pub(super) use publish::publish;
pub(super) use release::release;
pub(super) use server::{update_server, UpdateServerRequest};
pub(super) use sign::{sign, SignRequest};

use anyhow::{bail, Context, Result};
use ed25519_dalek::SigningKey;
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) const PKCS8_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

pub(super) fn read_private_key(path: &Path) -> Result<SigningKey> {
    let bytes = fs::read(path).with_context(|| format!("read private key {}", path.display()))?;
    let secret: [u8; 32] = if bytes.len() == 32 {
        bytes.as_slice().try_into().expect("length checked")
    } else if bytes.len() == 48 && bytes.starts_with(&PKCS8_PREFIX) {
        bytes[16..].try_into().expect("length checked")
    } else {
        bail!("{} is not a supported Ed25519 PKCS#8 key", path.display());
    };
    Ok(SigningKey::from_bytes(&secret))
}

/// Seconds since the Unix epoch; also stamps `tynd report` archives.
pub(in crate::commands) fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

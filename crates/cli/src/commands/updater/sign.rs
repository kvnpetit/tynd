//! `tynd updater sign`.

use anyhow::{bail, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::Signer as _;
use std::{fs, path::PathBuf};

use super::read_private_key;
use crate::fsx::atomic_write;
use crate::output::{display_path, success, write_line};

#[derive(Debug)]
pub(in crate::commands) struct SignRequest {
    pub(in crate::commands) file: PathBuf,
    pub(in crate::commands) key: PathBuf,
    pub(in crate::commands) out: Option<PathBuf>,
}

pub(in crate::commands) fn sign(request: &SignRequest) -> Result<String> {
    if !request.file.is_file() {
        bail!("file not found: {}", request.file.display());
    }
    let signing = read_private_key(&request.key)?;
    let payload = fs::read(&request.file)?;
    let signature = BASE64.encode(signing.sign(&payload).to_bytes());
    if let Some(path) = &request.out {
        atomic_write(path, format!("{signature}\n"))?;
        success(format!("Signature -> {}", display_path(path)));
    } else {
        write_line(format_args!("{signature}"))?;
    }
    Ok(signature)
}

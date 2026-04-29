//! Native bridge for `crypto.*` — the Web-standard subset only.
//!
//! Lite deliberately stays on the Web standard surface: digest + HMAC
//! sign/verify + getRandomValues. Anything more exotic (AES-GCM, RSA,
//! ECDSA, argon2, scrypt, PBKDF2, HKDF) is intentionally left to
//! userland pure-JS libs (`@noble/ciphers`, `@noble/hashes`,
//! `hash-wasm`).
//!
//! All three bridge fns forward to `tynd_host::os::compute` so hash
//! and random are implemented once in the workspace.

use base64::{engine::general_purpose::STANDARD, Engine};
use hmac::{Hmac, Mac};
use rquickjs::{Ctx, Function, Object};
use sha2::{Sha256, Sha384, Sha512};
use tynd_host::os::compute;

pub(crate) fn register<'js>(ctx: &Ctx<'js>, g: &Object<'js>) -> rquickjs::Result<()> {
    g.set(
        "__tynd_crypto_random__",
        Function::new(ctx.clone(), |n: u32| -> String {
            // WebCrypto caps getRandomValues at 64 KiB per call.
            let capped = (n as usize).min(65_536);
            compute::random_bytes_raw(capped)
                .map(|b| STANDARD.encode(&b))
                .unwrap_or_default()
        })?,
    )?;

    // UUIDv7 (RFC 9562): time-ordered + globally unique. Backs
    // `crypto.randomUUID()` in the lite polyfill so IPC call IDs minted on
    // the QuickJS side are sortable by creation time. The browser-side shim
    // (host-rs/src/ipc.rs JS_SHIM) still uses the platform's v4
    // `crypto.randomUUID()` until a tiny v7 polyfill is worth shipping.
    g.set(
        "__tynd_random_uuid__",
        Function::new(ctx.clone(), || -> String { compute::uuid_v7_string() })?,
    )?;

    g.set(
        "__tynd_crypto_digest__",
        Function::new(ctx.clone(), |algo: String, input_b64: String| -> String {
            let bytes = STANDARD.decode(input_b64.as_bytes()).unwrap_or_default();
            compute::hash_raw(&bytes, &algo, "base64").unwrap_or_default()
        })?,
    )?;

    g.set(
        "__tynd_crypto_hmac_sign__",
        Function::new(
            ctx.clone(),
            |algo: String, key_b64: String, data_b64: String| -> String {
                let key = STANDARD.decode(key_b64.as_bytes()).unwrap_or_default();
                let data = STANDARD.decode(data_b64.as_bytes()).unwrap_or_default();
                hmac_sign(&algo, &key, &data)
                    .map(|t| STANDARD.encode(&t))
                    .unwrap_or_default()
            },
        )?,
    )?;

    g.set(
        "__tynd_crypto_hmac_verify__",
        Function::new(
            ctx.clone(),
            |algo: String, key_b64: String, sig_b64: String, data_b64: String| -> bool {
                let key = STANDARD.decode(key_b64.as_bytes()).unwrap_or_default();
                let sig = STANDARD.decode(sig_b64.as_bytes()).unwrap_or_default();
                let data = STANDARD.decode(data_b64.as_bytes()).unwrap_or_default();
                hmac_verify(&algo, &key, &sig, &data)
            },
        )?,
    )?;

    Ok(())
}

/// Compute an HMAC tag for one of the WebCrypto algorithm strings, or
/// `None` if the algorithm isn't recognised. The closure body for all
/// three SHA arms is identical except for the digest type, so a tiny
/// `match` on `algo.as_str()` dispatches to the right `Hmac<H>` once
/// instead of repeating the tag-building dance per arm.
fn hmac_sign(algo: &str, key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    macro_rules! run {
        ($H:ty) => {
            Hmac::<$H>::new_from_slice(key).ok().map(|mut m| {
                m.update(data);
                m.finalize().into_bytes().to_vec()
            })
        };
    }
    match algo {
        "sha256" => run!(Sha256),
        "sha384" => run!(Sha384),
        "sha512" => run!(Sha512),
        _ => None,
    }
}

/// Constant-time HMAC verification mirroring [`hmac_sign`]. Returns
/// `false` on unknown algorithm so the JS side can treat that as a
/// regular "tag mismatch" without a separate error path.
fn hmac_verify(algo: &str, key: &[u8], sig: &[u8], data: &[u8]) -> bool {
    macro_rules! run {
        ($H:ty) => {
            Hmac::<$H>::new_from_slice(key)
                .ok()
                .and_then(|mut m| {
                    m.update(data);
                    m.verify_slice(sig).ok()
                })
                .is_some()
        };
    }
    match algo {
        "sha256" => run!(Sha256),
        "sha384" => run!(Sha384),
        "sha512" => run!(Sha512),
        _ => false,
    }
}

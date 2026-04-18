//! Native bridge for `crypto.*` — the Web-standard subset only.
//!
//! Lite deliberately stays on the Web standard surface: digest + HMAC
//! sign/verify + getRandomValues. Anything more exotic (AES-GCM, RSA,
//! ECDSA, argon2, scrypt, PBKDF2, HKDF) is intentionally left to
//! userland pure-JS libs (`@noble/ciphers`, `@noble/hashes`,
//! `hash-wasm`) — see ALTERNATIVES.md.
//!
//! All three bridge fns forward to `tynd_host::os::compute` so hash
//! and random are implemented once in the workspace.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
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
                let tag = match algo.as_str() {
                    "sha256" => Hmac::<Sha256>::new_from_slice(&key).ok().map(|mut m| {
                        m.update(&data);
                        m.finalize().into_bytes().to_vec()
                    }),
                    "sha384" => Hmac::<Sha384>::new_from_slice(&key).ok().map(|mut m| {
                        m.update(&data);
                        m.finalize().into_bytes().to_vec()
                    }),
                    "sha512" => Hmac::<Sha512>::new_from_slice(&key).ok().map(|mut m| {
                        m.update(&data);
                        m.finalize().into_bytes().to_vec()
                    }),
                    _ => None,
                };
                tag.map(|t| STANDARD.encode(&t)).unwrap_or_default()
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
                match algo.as_str() {
                    "sha256" => Hmac::<Sha256>::new_from_slice(&key)
                        .ok()
                        .and_then(|mut m| {
                            m.update(&data);
                            m.verify_slice(&sig).ok()
                        })
                        .is_some(),
                    "sha384" => Hmac::<Sha384>::new_from_slice(&key)
                        .ok()
                        .and_then(|mut m| {
                            m.update(&data);
                            m.verify_slice(&sig).ok()
                        })
                        .is_some(),
                    "sha512" => Hmac::<Sha512>::new_from_slice(&key)
                        .ok()
                        .and_then(|mut m| {
                            m.update(&data);
                            m.verify_slice(&sig).ok()
                        })
                        .is_some(),
                    _ => false,
                }
            },
        )?,
    )?;

    Ok(())
}

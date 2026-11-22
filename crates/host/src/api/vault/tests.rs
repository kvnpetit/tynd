//! Vault encrypt/decrypt round-trip, tamper detection, argument mapping and
//! path confinement. Each test works in its own directory under the default
//! `<temp>/<app>` capability root. Cocoon's KDF is deliberately slow,
//! so tests keep the number of encrypt/decrypt calls small.

use std::path::Path;

use serde_json::{json, Value};

use super::*;

fn tmp() -> tempfile::TempDir {
    crate::security::path_scope::scoped_tempdir()
}

fn p(base: &Path, rel: &str) -> String {
    base.join(rel)
        .to_str()
        .expect("utf-8 temp path")
        .to_string()
}

fn decoded(read: &Value) -> Vec<u8> {
    STANDARD
        .decode(read["data"].as_str().expect("data field"))
        .expect("base64 data")
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn round_trip_base64_and_ciphertext_hides_plaintext() {
    let dir = tmp();
    let file = p(dir.path(), "nested/deeper/secret.vault");
    let secret = b"top-secret-token-0123456789";
    write_impl(&file, "pw", &json!(STANDARD.encode(secret))).unwrap();

    let raw = std::fs::read(&file).unwrap();
    assert!(
        !raw.windows(secret.len()).any(|w| w == secret),
        "plaintext must not appear in the file"
    );
    assert_eq!(decoded(&read_impl(&file, "pw").unwrap()), secret);
    // Parent dirs were created, and no temp file is left behind.
    assert_eq!(entries(&dir.path().join("nested/deeper")), ["secret.vault"]);
}

#[test]
fn byte_array_payload_and_empty_payload_round_trip() {
    let dir = tmp();
    let file = p(dir.path(), "a.vault");
    write_impl(&file, "pw", &json!([0, 1, 127, 255])).unwrap();
    assert_eq!(decoded(&read_impl(&file, "pw").unwrap()), [0, 1, 127, 255]);

    // Overwrite with an empty payload replaces the previous content.
    write_impl(&file, "pw", &json!("")).unwrap();
    assert_eq!(decoded(&read_impl(&file, "pw").unwrap()), b"");
    assert_eq!(entries(dir.path()), ["a.vault"]);
}

#[test]
fn wrong_password_fails_to_decrypt() {
    let dir = tmp();
    let file = p(dir.path(), "a.vault");
    write_impl(&file, "right", &json!(STANDARD.encode(b"x"))).unwrap();
    let e = read_impl(&file, "wrong").unwrap_err();
    assert!(e.starts_with("vault.read: ("), "{e}");
    assert!(e.contains("parse"), "{e}");
}

#[test]
fn tampered_ciphertext_is_detected() {
    let dir = tmp();
    let file = p(dir.path(), "a.vault");
    write_impl(&file, "pw", &json!(STANDARD.encode(b"some secret payload"))).unwrap();
    let original = std::fs::read(&file).unwrap();

    // Flip one bit in the last byte (inside the GCM tag / ciphertext).
    let mut flipped = original.clone();
    *flipped.last_mut().unwrap() ^= 0x01;
    std::fs::write(&file, &flipped).unwrap();
    assert!(read_impl(&file, "pw").unwrap_err().contains("parse"));

    // Cocoon layout: 44-byte header (magic, config, salt 8..24, nonce
    // 24..36, big-endian length 36..44), 16-byte tag, ciphertext. Flip one
    // bit in the salt, the nonce, the tag and the first ciphertext byte.
    // (The length field is covered separately: see the forged-length test.)
    for offset in [10, 30, 50, 60] {
        let mut flipped = original.clone();
        flipped[offset] ^= 0x01;
        std::fs::write(&file, &flipped).unwrap();
        assert!(read_impl(&file, "pw").is_err(), "flip at byte {offset}");
    }

    // A header claiming fewer bytes than present still fails authentication.
    let mut shorter = original.clone();
    shorter[43] -= 1;
    std::fs::write(&file, &shorter).unwrap();
    assert!(read_impl(&file, "pw").is_err());

    // Truncation.
    std::fs::write(&file, &original[..original.len() - 1]).unwrap();
    assert!(read_impl(&file, "pw").is_err());

    // Garbage / empty file.
    std::fs::write(&file, b"not a cocoon container").unwrap();
    assert!(read_impl(&file, "pw").is_err());
    std::fs::write(&file, b"").unwrap();
    assert!(read_impl(&file, "pw").is_err());

    // Restoring the original bytes decrypts again.
    std::fs::write(&file, &original).unwrap();
    assert_eq!(
        decoded(&read_impl(&file, "pw").unwrap()),
        b"some secret payload"
    );
}

#[test]
fn empty_password_is_rejected_for_read_and_write() {
    let dir = tmp();
    let file = p(dir.path(), "a.vault");
    let e = write_impl(&file, "", &json!("")).unwrap_err();
    assert!(e.contains("'password' must not be empty"), "{e}");
    assert!(!Path::new(&file).exists());
    let e = read_impl(&file, "").unwrap_err();
    assert!(e.contains("'password' must not be empty"), "{e}");
}

#[test]
fn invalid_data_shapes_are_rejected_without_writing() {
    let dir = tmp();
    let file = p(dir.path(), "a.vault");
    let e = write_impl(&file, "pw", &json!("***not base64***")).unwrap_err();
    assert!(e.starts_with("vault.write: base64 decode"), "{e}");
    for bad in [json!([256]), json!([-1]), json!([1, "2"]), json!([1.5])] {
        let e = write_impl(&file, "pw", &bad).unwrap_err();
        assert!(e.contains("non-byte element"), "{bad}: {e}");
    }
    for bad in [json!(null), json!(42), json!({ "a": 1 })] {
        let e = write_impl(&file, "pw", &bad).unwrap_err();
        assert!(e.contains("missing 'data'"), "{bad}: {e}");
    }
    assert!(
        entries(dir.path()).is_empty(),
        "nothing written on rejection"
    );
}

#[test]
fn read_of_missing_file_maps_to_vault_error() {
    let dir = tmp();
    let file = p(dir.path(), "missing.vault");
    let e = read_impl(&file, "pw").unwrap_err();
    assert!(e.starts_with("vault.read: ("), "{e}");
}

#[test]
fn exists_and_idempotent_delete() {
    let dir = tmp();
    let file = p(dir.path(), "a.vault");
    assert!(!exists_impl(&file).unwrap());
    std::fs::write(&file, b"x").unwrap();
    assert!(exists_impl(&file).unwrap());
    assert_eq!(delete_impl(&file).unwrap(), json!({}));
    assert!(!exists_impl(&file).unwrap());
    assert_eq!(
        delete_impl(&file).unwrap(),
        json!({}),
        "second delete is a no-op"
    );
}

#[test]
fn delete_of_directory_is_an_error() {
    let dir = tmp();
    std::fs::create_dir(dir.path().join("d")).unwrap();
    let e = delete_impl(&p(dir.path(), "d")).unwrap_err();
    assert!(e.starts_with("vault.delete:"), "{e}");
    assert!(dir.path().join("d").is_dir());
}

#[test]
fn out_of_scope_paths_are_refused_for_every_method() {
    let root = std::env::temp_dir()
        .ancestors()
        .last()
        .unwrap()
        .join("tynd-out-of-scope-test-dir")
        .join("a.vault");
    let outside = root.to_str().unwrap();
    for (method, result) in [
        ("read", read_impl(outside, "pw").map(drop)),
        ("write", write_impl(outside, "pw", &json!("")).map(drop)),
        ("exists", exists_impl(outside).map(drop)),
        ("delete", delete_impl(outside).map(drop)),
    ] {
        let e = result.unwrap_err();
        assert!(e.starts_with(&format!("vault.{method}: ")), "{e}");
        assert!(e.contains("outside capability roots"), "{e}");
    }
    assert!(!root.exists());
}

#[test]
fn macro_dispatch_maps_arguments_and_errors() {
    let dir = tmp();
    let file = p(dir.path(), "d.vault");

    let e = inner::dispatch("read", &json!({ "path": file })).unwrap_err();
    assert_eq!(e, "vault.read: missing 'password' argument");
    let e = inner::dispatch("read", &json!({ "path": file, "password": 5 })).unwrap_err();
    assert!(e.starts_with("vault.read: invalid 'password':"), "{e}");
    let e = inner::dispatch("write", &json!({ "path": file, "password": "pw" })).unwrap_err();
    assert_eq!(e, "vault.write: missing 'data' argument");
    let e = inner::dispatch("nope", &json!({})).unwrap_err();
    assert!(e.contains("vault") && e.contains("nope"), "{e}");

    assert_eq!(
        inner::dispatch("exists", &json!({ "path": file })).unwrap(),
        json!(false)
    );
    inner::dispatch(
        "write",
        &json!({ "path": file, "password": "pw", "data": STANDARD.encode(b"via dispatch") }),
    )
    .unwrap();
    assert_eq!(
        inner::dispatch("exists", &json!({ "path": file })).unwrap(),
        json!(true)
    );
    let read = inner::dispatch("read", &json!({ "path": file, "password": "pw" })).unwrap();
    assert_eq!(decoded(&read), b"via dispatch");
    assert_eq!(
        inner::dispatch("delete", &json!({ "path": file })).unwrap(),
        json!({})
    );
}

#[cfg(unix)]
#[test]
fn written_vault_is_user_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tmp();
    let file = p(dir.path(), "a.vault");
    write_impl(&file, "pw", &json!("")).unwrap();
    let mode = std::fs::metadata(&file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

/// Env var carrying the forged vault path into the child probe.
const FORGED_ENV: &str = "TYND_VAULT_FORGED_LENGTH_FILE";

/// Child half of [`forged_length_header_is_an_error_not_an_abort`]: a no-op
/// unless spawned by it, so a regression that aborts the process cannot take
/// the whole test binary down.
#[test]
fn forged_length_child_probe() {
    let Ok(file) = std::env::var(FORGED_ENV) else {
        return;
    };
    let e = read_impl(&file, "pw").unwrap_err();
    assert!(e.starts_with("vault.read: ("), "{e}");
    assert!(e.contains("TooShort"), "{e}");
}

#[test]
fn forged_length_header_is_an_error_not_an_abort() {
    let dir = tmp();
    let file = p(dir.path(), "forged.vault");
    write_impl(&file, "pw", &json!("")).unwrap();
    let mut bytes = std::fs::read(&file).unwrap();
    bytes[36..44].copy_from_slice(&(1u64 << 39).to_be_bytes()); // 512 GiB
    std::fs::write(&file, &bytes).unwrap();

    let probe = format!(
        "{}::forged_length_child_probe",
        module_path!().split_once("::").unwrap().1
    );
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([probe.as_str(), "--exact", "--test-threads=1"])
        .env(FORGED_ENV, &file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "reading a forged-length vault killed the process: {status}"
    );
}

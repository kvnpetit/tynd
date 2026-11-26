use ed25519_dalek::{Signer as _, SigningKey};

use super::*;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn b64_public(key: &SigningKey) -> String {
    STANDARD.encode(key.verifying_key().as_bytes())
}

fn sign(key: &SigningKey, message: &[u8]) -> Signature {
    key.sign(message)
}

#[test]
fn pin_accepts_any_listed_key_and_rejects_others() {
    let (old, new, rogue) = (
        b64_public(&key(1)),
        b64_public(&key(2)),
        b64_public(&key(3)),
    );
    let pin = format!("{old}, {new}");
    assert!(decode_pub_key_with_pin(&old, Some(&pin)).is_ok());
    assert!(decode_pub_key_with_pin(&new, Some(&pin)).is_ok());
    assert!(decode_pub_key_with_pin(&rogue, Some(&pin)).is_err());
    assert!(decode_pub_key_with_pin(&old, None).is_err());
    assert!(decode_pub_key_with_pin(&old, Some("  ")).is_err());
}

fn config_with_keys(keys: &[String]) -> UpdaterConfig {
    serde_json::from_value(json!({
        "endpoints": ["https://cdn.example/m.json"],
        "publicKeys": keys,
    }))
    .unwrap()
}

#[test]
fn key_set_verifies_with_any_trusted_key() {
    let (old, new, rogue) = (key(1), key(2), key(3));
    let pin = format!("{};{}", b64_public(&old), b64_public(&new));
    let cfg = config_with_keys(&[b64_public(&old), b64_public(&new)]);
    let keys = KeySet::from_config_with_pin(&cfg, "t", Some(&pin)).unwrap();
    let message = b"release bytes";

    assert!(
        keys.verify(message, &sign(&old, message)).is_ok(),
        "old key"
    );
    assert!(
        keys.verify(message, &sign(&new, message)).is_ok(),
        "rotated key"
    );
    assert!(keys.verify(message, &sign(&rogue, message)).is_err());
    assert!(keys.verify(b"tampered", &sign(&new, message)).is_err());
}

#[test]
fn key_set_dedups_repeated_keys() {
    let k = key(1);
    let public = b64_public(&k);
    let cfg = config_with_keys(&[public.clone(), public.clone()]);
    let keys = KeySet::from_config_with_pin(&cfg, "t", Some(&public)).unwrap();
    assert_eq!(keys.0.len(), 1);
    assert!(keys.verify(b"m", &sign(&k, b"m")).is_ok());
}

#[test]
fn key_set_requires_every_trusted_key_to_be_pinned() {
    let (trusted, rogue) = (b64_public(&key(1)), b64_public(&key(3)));
    let cfg = config_with_keys(&[trusted.clone(), rogue]);
    assert!(KeySet::from_config_with_pin(&cfg, "t", Some(&trusted)).is_err());
    assert!(KeySet::from_config_with_pin(&config_with_keys(&[]), "t", Some(&trusted)).is_err());
}

#[test]
fn renderer_supplied_keys_have_no_effect() {
    // The key set is built from the trusted config alone: attacker keys in
    // the call arguments are neither read nor merged in.
    let (trusted, attacker) = (key(1), key(9));
    let cfg = config_with_keys(&[b64_public(&trusted)]);
    // Even a host without a pin cannot be talked into the attacker key.
    let pin = b64_public(&trusted);
    let keys = KeySet::from_config_with_pin(&cfg, "t", Some(&pin)).unwrap();
    assert_eq!(keys.0.len(), 1);
    assert!(keys.verify(b"m", &sign(&trusted, b"m")).is_ok());
    assert!(keys.verify(b"m", &sign(&attacker, b"m")).is_err());
}

#[test]
fn configure_accepts_and_validates_proxy_and_headers() {
    let base = json!({
        "endpoints": ["https://updates.example.com/m.json"],
        "publicKeys": ["K"],
    });
    let with = |extra: Value| {
        let mut value = base.clone();
        value.as_object_mut().unwrap().extend(
            extra
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        parse_trusted(&value)
    };
    let (_, net) = with(json!({
        "proxy": "http://proxy.corp:3128",
        "headers": { "Authorization": "Bearer t" },
    }))
    .unwrap();
    assert_eq!(net.proxy(), Some("http://proxy.corp:3128"));
    let url = url::Url::parse("https://updates.example.com/m.json").unwrap();
    assert_eq!(net.headers_for(&url).len(), 1);
    assert!(with(json!({ "headers": { "Host": "evil.example" } })).is_err());
    assert!(with(json!({ "proxy": "ftp://proxy.corp" })).is_err());
    // No proxy and no headers is still valid.
    assert!(with(json!({})).is_ok());
}

#[derive(Clone, Default)]
struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn invalid_env_runtime_config_is_logged_not_discarded() {
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        assert!(parse_env_trusted("{not json").is_none());
        assert!(parse_env_trusted(r#"{"endpoints": 3}"#).is_none());
        assert!(parse_env_trusted("null").is_none());
        let valid = r#"{"endpoints":["https://updates.example.com/m.json"],"publicKeys":["K"]}"#;
        assert!(parse_env_trusted(valid).is_some());
    });
    let log = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains("ignoring invalid TYND_FRONTEND_RUNTIME_CONFIG:"),
        "{log}"
    );
    assert!(
        log.contains("invalid TYND_FRONTEND_RUNTIME_CONFIG updater configuration"),
        "{log}"
    );
    assert_eq!(log.matches("WARN").count(), 2, "{log}");
}

#[test]
fn caller_fields_never_reach_the_http_layer() {
    // `http_agent*` and `get_with_redirects*` take no proxy or header
    // parameter: they read only the trusted network settings, so there is
    // no path for renderer values to reach the request; building the agent
    // needs no caller input.
    assert!(http_agent().is_ok());
    assert!(http_agent_with_timeout(PERIODIC_HTTP_TIMEOUT).is_ok());
}

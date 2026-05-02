//! Biometric authentication — Touch ID / Face ID / Windows Hello /
//! polkit (Linux). Promoted to a first-class API now that
//! `robius-authentication` is mature enough to abstract the per-OS
//! plumbing without dragging in Apple/MS SDK build deps directly.
//!
//! `isAvailable()` is best-effort: there's no cross-platform "would
//! authenticate succeed?" probe, so we report `true` whenever the
//! platform support is compiled in. The actual `authenticate()` call
//! is the canonical signal — it fails fast with a typed error if
//! biometrics aren't enrolled.

use robius_authentication::{
    AndroidText, BiometricStrength, Context, PolicyBuilder, Text, WindowsText,
};
use serde_json::{json, Value};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "isAvailable" => Ok(Value::Bool(true)),
        "authenticate" => authenticate(args),
        _ => Err(format!("biometric.{method}: unknown method")),
    }
}

fn authenticate(args: &Value) -> Result<Value, String> {
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .ok_or_else(|| "biometric.authenticate: missing 'reason'".to_string())?;
    let fallback_title = args
        .get("fallback_title")
        .or_else(|| args.get("fallbackTitle"))
        .and_then(Value::as_str)
        .unwrap_or("Authenticate");

    // `Policy` is built lazily via the const builder; biometrics-or-password
    // matches the most common UX (offer Touch ID, fall back to system PIN).
    let policy = PolicyBuilder::new()
        .biometrics(Some(BiometricStrength::Strong))
        .password(true)
        .build()
        .ok_or_else(|| {
            "biometric.authenticate: platform refused the requested policy".to_string()
        })?;

    let title_owned = fallback_title.to_string();
    let reason_owned = reason.to_string();

    // `Text` borrows; build it inside the same scope as the strings so
    // the references stay live across the blocking call.
    let text = Text {
        android: AndroidText {
            title: title_owned.as_str(),
            subtitle: None,
            description: Some(reason_owned.as_str()),
        },
        apple: reason_owned.as_str(),
        // `WindowsText::new` returns `Option` (length limits on the
        // credential UI). `new_truncated` is the infallible variant —
        // safer when the caller hands us long captions.
        windows: WindowsText::new_truncated(title_owned.as_str(), reason_owned.as_str()),
    };

    // `blocking_authenticate` waits for the user response on the calling
    // thread — fine here since `dispatch()` already runs off the main
    // event loop. A success returns `Ok(())`; any auth failure becomes
    // `Ok(false)` rather than propagating, since "user pressed Cancel"
    // is a normal flow, not an error.
    match Context::new(()).blocking_authenticate(text, &policy) {
        Ok(()) => Ok(json!(true)),
        Err(_) => Ok(json!(false)),
    }
}

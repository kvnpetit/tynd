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

use crate::ipc_error::IpcError;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    // W3.2 — ACL gate. `isAvailable` is left ungated because it returns
    // a static `true` (no side-effects, no secret data) and admin
    // tooling needs to introspect availability before the user grants
    // the capability. `authenticate` is the actual gated mutator.
    #[cfg(feature = "api-acl")]
    {
        if method == "authenticate"
            && !crate::os::acl::authority().is_allowed("biometric:authenticate", None, None, None)
        {
            return Err(
                IpcError::new("biometric", "authenticate", "permission denied by ACL")
                    .into_string(),
            );
        }
    }
    match method {
        "isAvailable" => Ok(Value::Bool(true)),
        "authenticate" => authenticate(args),
        _ => Err(IpcError::unknown_method("biometric", method).into_string()),
    }
}

fn authenticate(args: &Value) -> Result<Value, String> {
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from(IpcError::missing_arg("biometric", "authenticate", "reason")))?;
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
            String::from(IpcError::new(
                "biometric",
                "authenticate",
                "platform refused the requested policy",
            ))
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

//! Biometric authentication — Touch/Face ID and Windows Hello.
//! Wraps `robius-authentication` on macOS and Windows. Linux keeps the
//! stable API surface but reports unavailable because the upstream polkit
//! backend is incomplete and does not compile.
//!
//! `isAvailable()` is best-effort (no cross-platform "would auth succeed?"
//! probe). The canonical signal is `authenticate()`, which fails fast with
//! a typed error if biometrics aren't enrolled.
//!
//! `authenticate` is ACL-gated.

#[cfg(any(target_os = "macos", target_os = "windows"))]
use robius_authentication::{
    AndroidText, BiometricStrength, Context, Error as AuthError, PolicyBuilder, Text, WindowsText,
};
use serde_json::Value;

use crate::api::HostApi;
use crate::ipc::error::IpcError;

// Public methods are macro inputs; keeping the module crate-private prevents ACL bypass.
#[allow(unreachable_pub)]
pub(crate) mod inner {
    use super::authenticate_impl;
    use crate::tynd;

    /// Stateless dispatch marker.
    #[derive(Debug, Default)]
    pub(crate) struct BiometricApi;

    #[tynd::handler(api = "biometric")]
    impl BiometricApi {
        /// Best-effort availability check. Returns `true` whenever the
        /// platform backend is compiled in — there's no cross-platform
        /// "would `authenticate()` succeed right now?" probe, so this
        /// stays static. Use `authenticate()` itself as the canonical
        /// signal (it fails fast when biometrics aren't enrolled).
        pub fn is_available() -> bool {
            cfg!(any(target_os = "macos", target_os = "windows"))
        }

        /// Prompt the user for biometric (or password fallback)
        /// authentication. Returns `true` on success and `false` on
        /// user cancel — only true platform errors propagate as `Err`.
        pub fn authenticate(
            reason: String,
            fallback_title: Option<String>,
        ) -> Result<bool, String> {
            let fallback = fallback_title.unwrap_or_else(|| "Authenticate".into());
            authenticate_impl(&reason, &fallback)
        }
    }
}

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("biometric", |method, args| dispatch(method, &args));

/// Public dispatch wrapper — interposes the ACL gate on `authenticate`
/// before delegating to the macro-generated table inside [`inner`].
pub(crate) fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    // ACL gate. `isAvailable` is ungated (no side effects, admin tooling
    // needs introspection); `authenticate` is the gated mutator.
    if method == "authenticate"
        && !crate::security::acl::authority().is_allowed("biometric:authenticate", None, None, None)
    {
        return Err(
            IpcError::new("biometric", "authenticate", "permission denied by ACL").into_string(),
        );
    }
    inner::dispatch(method, args)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn authenticate_impl(reason: &str, fallback_title: &str) -> Result<bool, String> {
    // Biometrics-or-password matches the most common UX (offer Touch ID,
    // fall back to the system PIN).
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

    let text = Text {
        android: AndroidText {
            title: fallback_title,
            subtitle: None,
            description: Some(reason),
        },
        apple: reason,
        // `WindowsText::new` returns `Option` (length limits on the
        // credential UI). `new_truncated` is the infallible variant —
        // safer when the caller hands us long captions.
        windows: WindowsText::new_truncated(fallback_title, reason),
    };

    // `authenticate` only starts the prompt; the verdict arrives through the
    // callback, possibly on another thread. Park on a channel for it — fine
    // here since `dispatch()` already runs off the main event loop. We split
    // the error space carefully:
    //   * `Ok(true)`  — the user authenticated successfully.
    //   * `Ok(false)` — the user cancelled or failed the prompt. These are normal interactive
    //     flows; surfacing them as `Err` would force every JS caller to disambiguate "user said no"
    //     from "the sensor is dead".
    //   * `Err(...)`  — true platform / configuration faults (no enrollment, sensor disabled, group
    //     policy, etc.). Caller cannot retry by re-prompting the user.
    let (verdict_tx, verdict_rx) = std::sync::mpsc::channel();
    let context = Context::new(());
    let started = context.authenticate(text, &policy, move |result| {
        let _ = verdict_tx.send(result);
    });
    let verdict = match started {
        Ok(()) => verdict_rx.recv().map_err(|_| {
            String::from(IpcError::new(
                "biometric",
                "authenticate",
                "authentication prompt ended without a result",
            ))
        })?,
        Err(e) => Err(e),
    };
    match verdict {
        Ok(()) => Ok(true),
        Err(
            AuthError::UserCanceled
            | AuthError::AppCanceled
            | AuthError::SystemCanceled
            | AuthError::Authentication
            | AuthError::Exhausted,
        ) => Ok(false),
        Err(e) => Err(IpcError::new("biometric", "authenticate", format!("{e:?}")).into_string()),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn authenticate_impl(_reason: &str, _fallback_title: &str) -> Result<bool, String> {
    Err(IpcError::new(
        "biometric",
        "authenticate",
        "biometric authentication is unavailable on this platform",
    )
    .into_string())
}

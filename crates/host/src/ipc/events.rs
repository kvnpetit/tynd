//! Process-wide OS event emitter. Background threads emit without holding the
//! `EventLoopProxy`. `app::run_app` wires `set_emitter` once at startup.

use std::sync::OnceLock;

use serde_json::Value;

pub(crate) type EmitFn = Box<dyn Fn(&str, &Value) + Send + Sync + 'static>;

/// Which webviews receive an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EventScope {
    /// Every webview, remote-content ones included. Only for events that carry
    /// nothing sensitive; the host treats a plain [`emit`] as [`Self::Local`].
    All,
    /// Only webviews showing app-local content (bundled `tynd://` or a
    /// loopback dev server); never remote-content webviews.
    Local,
    /// The webview with this label when it shows app-local content, otherwise
    /// every app-local webview. For events that describe one webview's
    /// activity and may carry sensitive data (popup URLs, download paths).
    Origin(String),
    /// Exactly the webview with this label (the owner of the resource the
    /// event describes: a watch, a process, a socket), and nobody else, and
    /// only while the label is still on this document generation (the one the
    /// resource was created in). The host drops the event once the label has
    /// committed another document.
    Webview(String, u64),
}

pub(crate) type ScopedEmitFn = Box<dyn Fn(&str, &Value, EventScope) + Send + Sync + 'static>;

static EMITTER: OnceLock<EmitFn> = OnceLock::new();
static SCOPED_EMITTER: OnceLock<ScopedEmitFn> = OnceLock::new();

pub(crate) fn set_emitter(f: EmitFn) -> Result<(), &'static str> {
    EMITTER
        .set(f)
        .map_err(|_| "event emitter already registered")
}

/// Register the scoped emitter behind [`emit_scoped`].
pub(crate) fn set_scoped_emitter(f: ScopedEmitFn) -> Result<(), &'static str> {
    SCOPED_EMITTER
        .set(f)
        .map_err(|_| "scoped event emitter already registered")
}

/// Emit with the host's default scope: app-local webviews only. Use
/// [`emit_scoped`] to widen it ([`EventScope::All`]) or to target the owner
/// webview of a resource ([`EventScope::Webview`]).
pub(crate) fn emit(name: &str, data: &Value) {
    if let Some(f) = EMITTER.get() {
        f(name, data);
    }
}

/// Emit to a subset of webviews. Without a registered scoped emitter the event
/// is dropped: falling back to a broadcast would leak it to remote content.
pub(crate) fn emit_scoped(name: &str, data: &Value, scope: EventScope) {
    if let Some(f) = SCOPED_EMITTER.get() {
        f(name, data, scope);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_emit_without_an_emitter_is_a_silent_drop() {
        // No emitter is registered in this crate's tests; must not panic or broadcast.
        emit_scoped("x", &Value::Null, EventScope::Local);
        emit_scoped("x", &Value::Null, EventScope::Origin("main".into()));
        emit_scoped("x", &Value::Null, EventScope::Webview("main".into(), 0));
        emit_scoped("x", &Value::Null, EventScope::All);
    }
}

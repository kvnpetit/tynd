//! Owner of id-addressed resources (fs watches and file handles, spawned
//! processes, sockets, keep-awake requests, shortcuts, scheduled notifications,
//! staged updates).
//!
//! A renderer call is dispatched synchronously on one pool thread, so
//! [`with_caller_at`] makes the calling identity readable through [`current`]
//! while the OS handler runs. The identity is an [`Owner`]: the webview label,
//! the label's document [generation](generation) and the origin of the calling
//! frame (`None` for app-local content). The generation is read when the IPC
//! message is received, not when the pool thread runs it. A handler that creates
//! a resource captures that identity and emits the resource's events with
//! [`emit_for`], which delivers them to that webview only and only while the
//! label is still on the captured generation. Calls from the backend carry no
//! identity and keep the host's default scope.
//!
//! Ownership is per document, not per label: when a webview commits a new
//! document or closes, `release_webview` frees what that document could have
//! balanced, re-owns what must survive it (processes, scheduled notifications,
//! staged updates) to the backend, and bumps the label's generation so events
//! already queued for the old document are dropped instead of reaching the new
//! one. A resource whose creation call was still running at that moment
//! registers after the release: it checks [`Owner::is_current`] once it is
//! visible and is released right away when the generation has moved on. A
//! resource is reachable only by a caller with the same label, generation and
//! origin, so a remote iframe inside a local page cannot use the page's
//! resources. A backend-owned resource is reachable only from the backend and
//! from local-trust webviews.

// `current` / `emit_for` have callers only in the stream-resource APIs.
#![cfg_attr(
    not(any(feature = "api-fs", feature = "api-process", feature = "api-websocket")),
    allow(dead_code)
)]

use std::{cell::RefCell, sync::LazyLock};

use dashmap::DashMap;
use serde_json::Value;

use crate::ipc::events::{self, EventScope};

/// The identity of a renderer call, captured when the IPC message arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Owner {
    label: String,
    /// Document generation of `label` when the call was received.
    generation: u64,
    /// Effective origin of the calling frame; `None` for app-local content.
    origin: Option<String>,
}

impl Owner {
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    /// Whether the label is still on the document generation this identity was
    /// captured in.
    pub(crate) fn is_current(&self) -> bool {
        generation(&self.label) == self.generation
    }
}

thread_local! {
    static CALLER: RefCell<Option<Owner>> = const { RefCell::new(None) };
}

/// Restores the previous caller when dropped, also on unwind.
struct CallerGuard(Option<Owner>);

impl Drop for CallerGuard {
    fn drop(&mut self) {
        CALLER.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

/// Run `f` with an app-local `webview` on its current generation recorded as
/// the calling identity of this thread.
#[cfg(test)]
pub(crate) fn with_caller<R>(webview: &str, f: impl FnOnce() -> R) -> R {
    with_caller_trust(webview, true, f)
}

/// Like [`with_caller`] for a caller that is local or (a made-up) remote origin.
#[cfg(test)]
pub(crate) fn with_caller_trust<R>(webview: &str, local: bool, f: impl FnOnce() -> R) -> R {
    let origin = (!local).then_some("https://remote.invalid");
    with_caller_at(webview, generation(webview), origin, f)
}

/// Run `f` with the calling identity of this thread set to `webview`, its
/// document `generation` read when the call was received, and the effective
/// `origin` of the calling frame (`None` for app-local content). A remote
/// document is refused backend-owned resources and resources of another
/// origin (see [`permits`]).
pub(crate) fn with_caller_at<R>(
    webview: &str,
    generation: u64,
    origin: Option<&str>,
    f: impl FnOnce() -> R,
) -> R {
    let caller = Owner {
        label: webview.to_owned(),
        generation,
        origin: origin.map(str::to_owned),
    };
    let previous = CALLER.with(|slot| slot.replace(Some(caller)));
    let _guard = CallerGuard(previous);
    f()
}

/// Identity of the call being dispatched on this thread, if any: what a handler
/// stores as the owner of a resource it creates.
pub(crate) fn current() -> Option<Owner> {
    CALLER.with(|slot| slot.borrow().clone())
}

/// Whether the calling webview may operate on a resource owned by `owner`.
///
/// A resource with an owner is reachable only from a caller with the same
/// label, the same origin and the label's current generation (or from the
/// backend, which carries no identity). A backend-owned resource (`None`) is
/// reachable from the backend and from webviews showing app-local content, never
/// from a webview showing remote content. Callers must answer a refusal exactly
/// like an unknown id so a webview cannot probe for other resources.
pub(crate) fn permits(owner: Option<&Owner>) -> bool {
    CALLER.with(|slot| match (owner, slot.borrow().as_ref()) {
        (Some(owner), Some(caller)) => {
            owner.label == caller.label
                && owner.origin == caller.origin
                && owner.generation == caller.generation
                && owner.is_current()
        },
        (None, Some(caller)) => caller.origin.is_none(),
        (_, None) => true,
    })
}

/// Document generation of each label that ever released a document.
static GENERATIONS: LazyLock<DashMap<String, u64>> = LazyLock::new(DashMap::new);

/// Current document generation of `label` (0 until its first release). An event
/// of [`EventScope::Webview`] carries its owner's generation and is dropped at
/// delivery when the label has moved on.
pub(crate) fn generation(label: &str) -> u64 {
    GENERATIONS.get(label).map_or(0, |entry| *entry.value())
}

/// End the current document of `label`: events queued so far for it are stale.
pub(crate) fn bump_generation(label: &str) {
    *GENERATIONS.entry(label.to_owned()).or_insert(0) += 1;
}

/// The scope for an event of a resource owned by `owner`: that webview alone
/// while it is on the owner's document generation, or `None` (the host's
/// default scope) for a backend-created resource.
pub(crate) fn scope_for(owner: Option<&Owner>) -> Option<EventScope> {
    owner.map(|owner| EventScope::Webview(owner.label.clone(), owner.generation))
}

/// Emit an event of a resource owned by `owner`.
pub(crate) fn emit_for(owner: Option<&Owner>, name: &str, data: &Value) {
    match scope_for(owner) {
        Some(scope) => events::emit_scoped(name, data, scope),
        None => events::emit(name, data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label_of_current() -> Option<String> {
        current().map(|owner| owner.label)
    }

    fn owned(label: &str, origin: Option<&str>) -> Owner {
        Owner {
            label: label.to_owned(),
            generation: generation(label),
            origin: origin.map(str::to_owned),
        }
    }

    #[test]
    fn caller_is_visible_only_inside_the_call_and_restored_after() {
        assert_eq!(label_of_current(), None);
        with_caller("outer", || {
            assert_eq!(label_of_current().as_deref(), Some("outer"));
            with_caller("inner", || {
                assert_eq!(label_of_current().as_deref(), Some("inner"));
            });
            assert_eq!(label_of_current().as_deref(), Some("outer"));
        });
        assert_eq!(label_of_current(), None);
    }

    #[test]
    fn caller_is_restored_after_a_panic() {
        let result = std::panic::catch_unwind(|| with_caller("w", || panic!("boom")));
        assert!(result.is_err());
        assert_eq!(label_of_current(), None);
    }

    #[test]
    fn caller_does_not_leak_to_other_threads() {
        with_caller("w", || {
            assert_eq!(std::thread::spawn(current).join().unwrap(), None);
        });
    }

    #[test]
    fn owned_resources_are_reachable_only_by_their_webview_or_the_backend() {
        let panel = owned("panel", None);
        // Backend caller (no label) reaches everything.
        assert!(permits(Some(&panel)));
        assert!(permits(None));
        with_caller("panel", || {
            assert!(permits(Some(&panel)));
            assert!(!permits(Some(&owned("other", None))));
            assert!(permits(None));
        });
    }

    #[test]
    fn remote_callers_are_refused_backend_owned_resources() {
        let remote = owned("remote", Some("https://remote.invalid"));
        with_caller_trust("remote", false, || {
            assert!(!permits(None));
            assert!(permits(Some(&remote)));
            assert!(!permits(Some(&owned("panel", None))));
        });
        with_caller_trust("local", true, || assert!(permits(None)));
        // The backend carries no caller and reaches everything.
        assert!(permits(None));
    }

    #[test]
    fn remote_iframe_in_a_local_page_is_refused_the_pages_resources() {
        let label = "owner-iframe";
        let page = owned(label, None);
        // Same label, but the calling frame has a remote origin.
        with_caller_trust(label, false, || assert!(!permits(Some(&page))));
        with_caller_trust(label, true, || assert!(permits(Some(&page))));

        // The reverse: a resource opened by a remote frame is not reachable
        // from the local page or from another remote origin.
        let frame = owned(label, Some("https://a.invalid"));
        let at = |origin: Option<&str>| {
            with_caller_at(label, generation(label), origin, || permits(Some(&frame)))
        };
        assert!(!at(None));
        assert!(!at(Some("https://b.invalid")));
        assert!(at(Some("https://a.invalid")));
    }

    #[test]
    fn resource_of_an_earlier_document_is_not_reachable_after_a_commit() {
        let label = "owner-stale";
        let old = owned(label, None);
        let old_generation = generation(label);
        assert!(old.is_current());
        bump_generation(label);
        assert!(!old.is_current());
        // A call of the new document and a late call of the old one are both
        // refused.
        with_caller(label, || assert!(!permits(Some(&old))));
        with_caller_at(label, old_generation, None, || {
            assert!(!permits(Some(&old)));
        });
    }

    #[test]
    fn generation_advances_per_label() {
        let (a, b) = ("owner-gen-a", "owner-gen-b");
        assert_eq!(generation(a), 0);
        bump_generation(a);
        bump_generation(a);
        assert_eq!(generation(a), 2);
        assert_eq!(generation(b), 0);
    }

    #[test]
    fn owned_resources_target_their_webview_and_backend_ones_use_the_default() {
        assert_eq!(
            scope_for(Some(&owned("panel", None))),
            Some(EventScope::Webview("panel".into(), 0))
        );
        assert_eq!(scope_for(None), None);
    }

    #[test]
    fn event_scope_keeps_the_generation_captured_at_creation() {
        let label = "owner-scope";
        let old = owned(label, None);
        bump_generation(label);
        assert_eq!(
            scope_for(Some(&old)),
            Some(EventScope::Webview(label.into(), 0))
        );
    }
}

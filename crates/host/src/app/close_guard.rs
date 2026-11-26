//! Preventable close, per window: a close request opens a short grace period
//! during which any `onCloseRequested` handler (renderer or backend) can call
//! `cancelClose()` for that window. The event loop then closes it only when
//! nobody did.

use std::sync::LazyLock;

use dashmap::DashMap;

/// Windows with a pending close request, mapped to "a handler cancelled it".
static PENDING: LazyLock<DashMap<String, bool>> = LazyLock::new(DashMap::new);

/// Open a grace period for `label`. Returns `false` when one is already
/// running (repeated clicks on X), so the caller does not start a second one.
pub(super) fn begin(label: &str) -> bool {
    if PENDING.contains_key(label) {
        return false;
    }
    PENDING.insert(label.to_owned(), false);
    true
}

/// `cancelClose()` for `label`. A no-op outside a grace period: a handler
/// that answers too late cannot block a later, unrelated close.
pub(super) fn cancel(label: &str) {
    if let Some(mut cancelled) = PENDING.get_mut(label) {
        *cancelled = true;
    }
}

/// End the grace period of `label`: `true` when the window should close now.
pub(super) fn finish(label: &str) -> bool {
    PENDING
        .remove(label)
        .is_some_and(|(_, cancelled)| !cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closes_unless_cancelled_during_the_grace_period() {
        assert!(begin("cg-a"));
        assert!(finish("cg-a"));

        assert!(begin("cg-b"));
        cancel("cg-b");
        assert!(!finish("cg-b"));
    }

    #[test]
    fn repeated_requests_share_one_grace_period() {
        assert!(begin("cg-c"));
        assert!(!begin("cg-c"));
        assert!(finish("cg-c"));
        assert!(!finish("cg-c"), "a second timer must not close twice");
    }

    #[test]
    fn a_late_cancel_does_not_block_the_next_close() {
        cancel("cg-d");
        assert!(begin("cg-d"));
        assert!(finish("cg-d"));
    }

    #[test]
    fn windows_are_independent() {
        assert!(begin("cg-e"));
        assert!(begin("cg-f"));
        cancel("cg-e");
        assert!(!finish("cg-e"));
        assert!(finish("cg-f"));
    }
}

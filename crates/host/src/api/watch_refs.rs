//! Per-owner reference counts for process-wide singleton watchers
//! (`clipboard` monitor, `theme` watcher).
//!
//! A watcher stays alive while any owner holds a reference. Owners are
//! webview labels, so a webview that reloads or closes without calling
//! `stop*` can have its references dropped in one go ([`WatchRefs::release_all`])
//! instead of leaking the watcher for the lifetime of the process.

use std::collections::HashMap;

/// Owner recorded for calls that do not come from a webview (backend code,
/// native plugins).
pub(crate) const BACKEND_OWNER: &str = "<backend>";

#[derive(Debug, Default)]
pub(crate) struct WatchRefs {
    by_owner: HashMap<String, usize>,
}

impl WatchRefs {
    /// Take one reference for `owner`.
    pub(crate) fn acquire(&mut self, owner: &str) {
        *self.by_owner.entry(owner.to_owned()).or_insert(0) += 1;
    }

    /// Drop one reference held by `owner`. Returns `false` (and changes
    /// nothing) when `owner` holds none, so one webview cannot release
    /// another's subscription with an unbalanced stop.
    pub(crate) fn release_one(&mut self, owner: &str) -> bool {
        let Some(count) = self.by_owner.get_mut(owner) else {
            return false;
        };
        *count -= 1;
        if *count == 0 {
            self.by_owner.remove(owner);
        }
        true
    }

    /// Drop every reference held by `owner`; returns how many were held.
    pub(crate) fn release_all(&mut self, owner: &str) -> usize {
        self.by_owner.remove(owner).unwrap_or(0)
    }

    /// References held across all owners.
    pub(crate) fn total(&self) -> usize {
        self.by_owner.values().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_tracked_per_owner() {
        let mut refs = WatchRefs::default();
        refs.acquire("a");
        refs.acquire("a");
        refs.acquire("b");
        assert_eq!(refs.total(), 3);
        assert!(refs.release_one("a"));
        assert_eq!(refs.total(), 2);
    }

    #[test]
    fn unbalanced_release_cannot_steal_another_owners_ref() {
        let mut refs = WatchRefs::default();
        refs.acquire("a");
        assert!(!refs.release_one("b"));
        assert_eq!(refs.total(), 1);
        assert!(refs.release_one("a"));
        assert!(!refs.release_one("a"));
        assert_eq!(refs.total(), 0);
    }

    #[test]
    fn release_all_drops_only_that_owner() {
        let mut refs = WatchRefs::default();
        refs.acquire("a");
        refs.acquire("a");
        refs.acquire("b");
        assert_eq!(refs.release_all("a"), 2);
        assert_eq!(refs.release_all("a"), 0);
        assert_eq!(refs.total(), 1);
    }
}

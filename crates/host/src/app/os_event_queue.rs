use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use parking_lot::Mutex;

use crate::ipc::events::EventScope;
use crate::ipc::owner;

pub(super) const CAPACITY: usize = 1_024;
pub(super) const BATCH_SIZE: usize = 64;
const LOSSY_CAPACITY: usize = 960;

struct QueuedEvent {
    script: String,
    scope: EventScope,
    lossy: bool,
}

impl QueuedEvent {
    /// An [`EventScope::Webview`] event carries the document generation its
    /// resource was created in. It is dropped at delivery if the label has
    /// committed or closed a document since, so it never reaches the next one.
    fn is_stale(&self) -> bool {
        match &self.scope {
            EventScope::Webview(label, created_in) => owner::generation(label) != *created_in,
            _ => false,
        }
    }
}

struct QueueState {
    events: VecDeque<QueuedEvent>,
    lossy_len: usize,
}

pub(super) struct OsEventQueue {
    queue: Mutex<QueueState>,
    scheduled: AtomicBool,
    dropped: AtomicUsize,
}

pub(super) struct EventBatch {
    pub(super) scripts: Vec<(String, EventScope)>,
    pub(super) dropped: usize,
    pub(super) reschedule: bool,
}

impl OsEventQueue {
    pub(super) fn new() -> Self {
        Self {
            queue: Mutex::new(QueueState {
                events: VecDeque::with_capacity(CAPACITY),
                lossy_len: 0,
            }),
            scheduled: AtomicBool::new(false),
            dropped: AtomicUsize::new(0),
        }
    }

    /// Queue one pre-serialized event emitted through the plain emitter. The
    /// queue is deliberately loss-bounded: a stalled renderer cannot turn
    /// high-rate native events into unbounded process memory.
    ///
    /// Plain events (file paths, process output, socket payloads, native-plugin
    /// data, ...) reach app-local webviews only, unless the event is on
    /// [`REMOTE_SAFE_EVENTS`]. An emitter that wants a wider or narrower
    /// audience uses [`Self::enqueue_scoped`].
    pub(super) fn enqueue(&self, event: &str, script: String) -> bool {
        self.enqueue_scoped(event, script, default_scope(event))
    }

    /// Like [`Self::enqueue`] for an event restricted to some webviews.
    pub(super) fn enqueue_scoped(&self, event: &str, script: String, scope: EventScope) -> bool {
        let lossy = is_high_rate_event(event);
        let mut queue = self.queue.lock();
        if queue.events.len() >= CAPACITY || (lossy && queue.lossy_len >= LOSSY_CAPACITY) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        queue.events.push_back(QueuedEvent {
            script,
            scope,
            lossy,
        });
        if lossy {
            queue.lossy_len += 1;
        }
        drop(queue);
        !self.scheduled.swap(true, Ordering::AcqRel)
    }

    /// Consume one event-loop wakeup and drain a bounded batch. Producers that
    /// race with this method either schedule their own wakeup or observe the
    /// reschedule latch set here; no queued event can lose its wakeup.
    pub(super) fn drain_batch(&self) -> EventBatch {
        self.scheduled.store(false, Ordering::Release);
        let mut queue = self.queue.lock();
        let take = queue.events.len().min(BATCH_SIZE);
        let drained: Vec<QueuedEvent> = queue.events.drain(..take).collect();
        queue.lossy_len -= drained.iter().filter(|event| event.lossy).count();
        let scripts = drained
            .into_iter()
            .filter(|event| !event.is_stale())
            .map(|event| (event.script, event.scope))
            .collect();
        let has_more = !queue.events.is_empty();
        drop(queue);
        let reschedule = has_more && !self.scheduled.swap(true, Ordering::AcqRel);
        EventBatch {
            scripts,
            dropped: self.dropped.swap(0, Ordering::AcqRel),
            reschedule,
        }
    }

    pub(super) fn cancel_schedule(&self) {
        self.scheduled.store(false, Ordering::Release);
    }
}

fn is_high_rate_event(event: &str) -> bool {
    matches!(
        event,
        "audio:samples"
            | "http:request-chunk"
            | "terminal:data"
            | "websocket:message"
            | "ws:message"
    )
}

/// Plain events that may reach webviews showing remote content. Empty: every
/// OS and native-plugin event can carry local data (paths, process output,
/// socket payloads, deep-link URLs, plugin streams), and a remote page can
/// define the `window.__tynd_os_event__` hook that receives them. Add a name
/// here only with a written reason, or emit it with `EventScope::All`.
const REMOTE_SAFE_EVENTS: &[&str] = &[];

/// Scope of an event emitted through the plain emitter.
fn default_scope(event: &str) -> EventScope {
    if REMOTE_SAFE_EVENTS.contains(&event) {
        EventScope::All
    } else {
        EventScope::Local
    }
}

/// Labels, among `all`, that must receive an event of `scope`. `is_local`
/// tells whether a webview currently shows app-local content.
pub(super) fn scope_targets<'a>(
    scope: &EventScope,
    all: &[&'a str],
    is_local: impl Fn(&str) -> bool,
) -> Vec<&'a str> {
    match scope {
        EventScope::All => all.to_vec(),
        EventScope::Local => all
            .iter()
            .copied()
            .filter(|label| is_local(label))
            .collect(),
        // Activity of one webview (navigation, popups, downloads, drops) is
        // delivered to every app-local webview whether the originator is local
        // or remote: a parent window must see its local child's events, and
        // remote content never receives them.
        EventScope::Origin(_) => all
            .iter()
            .copied()
            .filter(|label| is_local(label))
            .collect(),
        // The owner of a resource gets its events even when it shows remote
        // content (it passed the capability check that created the resource).
        EventScope::Webview(owner, _) => {
            all.iter().copied().filter(|label| label == owner).collect()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABELS: [&str; 3] = ["main", "panel", "remote"];

    fn is_local(label: &str) -> bool {
        label != "remote"
    }

    #[test]
    fn all_scope_reaches_every_webview() {
        assert_eq!(
            scope_targets(&EventScope::All, &LABELS, is_local),
            LABELS.to_vec()
        );
    }

    #[test]
    fn local_scope_skips_remote_webviews() {
        assert_eq!(
            scope_targets(&EventScope::Local, &LABELS, is_local),
            vec!["main", "panel"]
        );
    }

    #[test]
    fn origin_scope_from_a_local_webview_reaches_every_local_webview() {
        assert_eq!(
            scope_targets(&EventScope::Origin("panel".into()), &LABELS, is_local),
            vec!["main", "panel"]
        );
    }

    #[test]
    fn origin_scope_from_a_remote_webview_goes_to_local_ones_only() {
        assert_eq!(
            scope_targets(&EventScope::Origin("remote".into()), &LABELS, is_local),
            vec!["main", "panel"]
        );
    }

    #[test]
    fn webview_scope_reaches_only_its_owner() {
        assert_eq!(
            scope_targets(&EventScope::Webview("remote".into(), 0), &LABELS, is_local),
            vec!["remote"]
        );
        assert_eq!(
            scope_targets(&EventScope::Webview("panel".into(), 0), &LABELS, is_local),
            vec!["panel"]
        );
        assert_eq!(
            scope_targets(&EventScope::Webview("gone".into(), 0), &LABELS, is_local),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn plain_events_default_to_local_webviews() {
        for name in [
            "fs:change",
            "process:stdout:42",
            "process:stderr:42",
            "process:exit:42",
            "websocket:message",
            "keys:set",
            "shortcut:triggered",
            "notification:action",
            "terminal:data",
            "clipboard:change",
            "app:second-launch",
            "app:open-url",
            "deep-link:url",
            "window:drag-enter",
            "theme:change",
        ] {
            let queue = OsEventQueue::new();
            queue.enqueue(name, "S".into());
            assert_eq!(
                queue.drain_batch().scripts,
                vec![("S".to_owned(), EventScope::Local)],
                "{name}"
            );
        }
    }

    #[test]
    fn no_event_is_remote_safe_without_a_reason() {
        assert_eq!(REMOTE_SAFE_EVENTS, &[] as &[&str]);
    }

    #[test]
    fn explicit_scopes_survive_the_queue() {
        let queue = OsEventQueue::new();
        queue.enqueue_scoped("a", "A".into(), EventScope::All);
        queue.enqueue_scoped("b", "B".into(), EventScope::Webview("w".into(), 0));
        assert_eq!(
            queue.drain_batch().scripts,
            vec![
                ("A".to_owned(), EventScope::All),
                ("B".to_owned(), EventScope::Webview("w".into(), 0))
            ]
        );
    }

    #[test]
    fn webview_events_of_an_earlier_document_are_dropped_after_a_commit() {
        let queue = OsEventQueue::new();
        // Generations are captured when the resource is created, not queued.
        let old = owner::generation("oq-gen");
        let other = owner::generation("oq-other");
        queue.enqueue_scoped("a", "old".into(), EventScope::Webview("oq-gen".into(), old));
        queue.enqueue_scoped(
            "b",
            "other".into(),
            EventScope::Webview("oq-other".into(), other),
        );
        queue.enqueue_scoped("c", "local".into(), EventScope::Local);
        // The label commits a new document (release_webview bumps it).
        owner::bump_generation("oq-gen");
        // A frame from the old document's socket that was still in flight, and
        // one from a resource of the new document.
        queue.enqueue_scoped(
            "websocket:message",
            "late".into(),
            EventScope::Webview("oq-gen".into(), old),
        );
        let new = owner::generation("oq-gen");
        queue.enqueue_scoped("d", "new".into(), EventScope::Webview("oq-gen".into(), new));

        assert_eq!(
            queue.drain_batch().scripts,
            vec![
                (
                    "other".to_owned(),
                    EventScope::Webview("oq-other".into(), other)
                ),
                ("local".to_owned(), EventScope::Local),
                ("new".to_owned(), EventScope::Webview("oq-gen".into(), new)),
            ]
        );
    }

    #[test]
    fn scoped_events_keep_their_scope_through_the_queue() {
        let queue = OsEventQueue::new();
        assert!(queue.enqueue("a", "A".into()));
        assert!(!queue.enqueue_scoped("b", "B".into(), EventScope::Local));
        let batch = queue.drain_batch();
        assert_eq!(
            batch.scripts,
            vec![
                ("A".to_owned(), EventScope::Local),
                ("B".to_owned(), EventScope::Local)
            ]
        );
    }
}

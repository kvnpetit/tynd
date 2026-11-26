//! `fs.watch` / `fs.unwatch`: debounced filesystem watchers that emit
//! `fs:change` events to the webview (or backend) that registered them.

use std::{
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        LazyLock,
    },
};

// `notify` is only reached through the debouncer's re-export, so our types
// and the debouncer's `Watcher` bound always name the same crate version.
use notify_debouncer_full::notify::{EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use parking_lot::Mutex;
use serde_json::{json, Value};

use super::ops::{bool_arg, utf8_path_arg};
use crate::ipc::owner::{self, Owner};

/// Watcher handle stored per `fs.watch` registration. The
/// `notify_debouncer_full::Debouncer` owns the `notify` recommended
/// watcher *plus* a cache of file IDs and a debounce/sort thread —
/// dropping it joins the thread and unregisters every path. It gives us:
///
/// - dedup of multi-event renames (a Windows save emits `Remove` + `Create` on the same path; the
///   debouncer collapses them into a single `Modify`/`Rename`),
/// - tracked rename `from` → `to` paths so events buffered before the rename land with the
///   post-rename path,
/// - exactly-once `Remove` for a recursive directory delete.
///
/// One `{ id, kind, path }` event is emitted per affected path.
type WatchHandle = Debouncer<RecommendedWatcher, RecommendedCache>;

/// A registered watch and the webview identity that opened it (`None`: the
/// backend).
struct Watch {
    _handle: WatchHandle,
    owner: Option<Owner>,
}

static WATCHERS: LazyLock<Mutex<ahash::HashMap<u64, Watch>>> =
    LazyLock::new(|| Mutex::new(ahash::HashMap::default()));
static NEXT_WATCH_ID: AtomicU64 = AtomicU64::new(1);

fn kind_label(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Create(_) => "create",
        EventKind::Modify(notify_debouncer_full::notify::event::ModifyKind::Name(_)) => "rename",
        EventKind::Modify(_) => "modify",
        EventKind::Remove(_) => "delete",
        _ => "other",
    }
}

/// Debounce window — short enough that interactive saves still feel
/// instant, long enough that the OS coalescing windows on each platform
/// catch the multi-event bursts they emit per save (Windows
/// ReadDirectoryChangesW fires three to four events, FSEvents on macOS
/// can fan out renames as Remove + Create).
const WATCH_DEBOUNCE_MS: u64 = 100;

pub(super) fn watch(args: &Value) -> Result<Value, String> {
    let path = utf8_path_arg(args, "path")?;
    let resolved = crate::security::policy::authorize_fs_path(path.as_str())?;
    // Notify wraps the OS watcher (inotify / FSEvents / ReadDirectoryChangesW);
    // it expects an absolute path it can pass straight to the kernel and
    // doesn't have a `Dir`-relative variant. We can't sandbox the watcher
    // itself, but we *can* refuse to register one for paths outside an
    // allowed root — `authorize_fs_path()` does exactly that and gives us
    // the resolved spelling that must be passed to the ambient watcher.
    let recursive = bool_arg(args, "recursive", false);

    let id = NEXT_WATCH_ID.fetch_add(1, Ordering::Relaxed);
    let owner = owner::current();
    let event_owner = owner.clone();

    let mut debouncer = new_debouncer(
        std::time::Duration::from_millis(WATCH_DEBOUNCE_MS),
        None,
        move |result: DebounceEventResult| emit_events(event_owner.as_ref(), id, result),
    )
    .map_err(|e| format!("fs.watch: {e}"))?;

    let mode = if recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    debouncer
        .watch(&resolved, mode)
        .map_err(|e| format!("fs.watch({path}): {e}"))?;

    register(id, debouncer, owner)
}

/// Emit one `fs:change` event per affected path of a debounced batch (or per
/// error) to `owner`.
fn emit_events(owner: Option<&Owner>, id: u64, result: DebounceEventResult) {
    match result {
        Ok(events_batch) => {
            for evt in events_batch {
                let kind = kind_label(evt.kind);
                for authorized_path in evt.paths.iter().filter_map(|p| authorized_event_path(p)) {
                    owner::emit_for(
                        owner,
                        "fs:change",
                        &json!({
                            "id": id,
                            "kind": kind,
                            "path": authorized_path,
                        }),
                    );
                }
            }
        },
        Err(errs) => {
            for e in errs {
                owner::emit_for(
                    owner,
                    "fs:change",
                    &json!({ "id": id, "kind": "error", "error": e.to_string() }),
                );
            }
        },
    }
}

/// Event path as the FS policy resolves it, or `None` when it falls outside
/// the allow-list (or isn't UTF-8).
///
/// SECURITY: notify follows symlinks at the OS layer and surfaces the
/// symlink TARGET path. If a watched dir contains a symlink pointing outside
/// the allow-list, the watcher would otherwise leak that external absolute
/// path to the frontend, so every event path is re-checked.
fn authorized_event_path(path: &Path) -> Option<String> {
    let authorized = crate::security::policy::authorize_fs_path(path.to_str()?).ok()?;
    authorized.to_str().map(str::to_owned)
}

/// Make the watch visible under `id`. The call may have taken long enough to
/// get here (recursive watch of a large tree) for the calling webview to commit
/// another document. A release that ran before the insert did not see the
/// watch, so it is dropped here instead of streaming paths to the next
/// document. Checked once the watch is visible: a later release finds it in the
/// map.
fn register(id: u64, handle: WatchHandle, owner: Option<Owner>) -> Result<Value, String> {
    let captured = owner.clone();
    WATCHERS.lock().insert(
        id,
        Watch {
            _handle: handle,
            owner,
        },
    );
    if captured.is_some_and(|owner| !owner.is_current()) {
        unwatch_stale(id);
        return Err("fs.watch: the document that requested it is gone".to_owned());
    }
    Ok(json!({ "id": id }))
}

pub(super) fn unwatch(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "fs.unwatch: missing 'id'".to_string())?;
    Ok(Value::Bool(unwatch_id(id)))
}

/// Remove watch `id` if the caller may operate on it. A watch owned by another
/// webview answers like an unknown id. The debouncer is dropped after the map
/// lock is released: dropping it joins its worker thread.
fn unwatch_id(id: u64) -> bool {
    let removed = {
        let mut watchers = WATCHERS.lock();
        if watchers
            .get(&id)
            .is_some_and(|watch| owner::permits(watch.owner.as_ref()))
        {
            watchers.remove(&id)
        } else {
            None
        }
    };
    removed.is_some()
}

/// Drop watch `id` without a caller check (the debouncer is dropped after the
/// map lock is released).
fn unwatch_stale(id: u64) {
    let removed = WATCHERS.lock().remove(&id);
    drop(removed);
}

/// Drop every watch opened by `label`. The debouncers are dropped after the
/// map lock is released: dropping one joins its worker thread.
pub(super) fn release_owner(label: &str) {
    let released: Vec<Watch> = {
        let mut watchers = WATCHERS.lock();
        let ids: Vec<u64> = watchers
            .iter()
            .filter(|(_, watch)| {
                watch
                    .owner
                    .as_ref()
                    .is_some_and(|owner| owner.label() == label)
            })
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter()
            .filter_map(|id| watchers.remove(&id))
            .collect()
    };
    drop(released);
}

pub(super) fn shutdown_all() {
    // Dropping each debouncer unregisters its OS paths and joins the
    // debounce worker before this function returns.
    WATCHERS.lock().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::owner::with_caller;

    fn debouncer() -> WatchHandle {
        new_debouncer(
            std::time::Duration::from_millis(WATCH_DEBOUNCE_MS),
            None,
            |_: DebounceEventResult| {},
        )
        .expect("debouncer")
    }

    fn owner_of(label: &str) -> Option<Owner> {
        with_caller(label, owner::current)
    }

    fn insert_watch(owner: Option<Owner>) -> u64 {
        let id = NEXT_WATCH_ID.fetch_add(1, Ordering::Relaxed);
        register(id, debouncer(), owner).expect("register");
        id
    }

    #[test]
    fn unwatch_by_another_webview_is_indistinguishable_from_an_unknown_id() {
        let id = insert_watch(owner_of("watch-a"));
        let refused = with_caller("watch-b", || unwatch_id(id));
        let unknown = with_caller("watch-b", || unwatch_id(u64::MAX));
        assert_eq!(refused, unknown);
        assert!(!refused);
        assert!(WATCHERS.lock().contains_key(&id));

        assert!(with_caller("watch-a", || unwatch_id(id)));
        assert!(!WATCHERS.lock().contains_key(&id));
    }

    #[test]
    fn backend_may_unwatch_a_webview_watch() {
        let id = insert_watch(owner_of("watch-c"));
        assert!(unwatch_id(id));
        assert!(!unwatch_id(id));
    }

    #[test]
    fn registration_after_the_document_was_replaced_is_released() {
        let label = "watch-late";
        // The create call is received (identity captured) before the commit.
        let late = owner_of(label);
        owner::bump_generation(label);
        let id = NEXT_WATCH_ID.fetch_add(1, Ordering::Relaxed);
        assert!(register(id, debouncer(), late).is_err());
        assert!(!WATCHERS.lock().contains_key(&id));
    }

    #[test]
    fn remote_iframe_cannot_unwatch_the_local_pages_watch() {
        let id = insert_watch(owner_of("watch-frame"));
        assert!(!owner::with_caller_trust("watch-frame", false, || {
            unwatch_id(id)
        }));
        assert!(WATCHERS.lock().contains_key(&id));
        assert!(with_caller("watch-frame", || unwatch_id(id)));
    }
}

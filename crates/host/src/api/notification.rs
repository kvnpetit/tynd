//! Cross-platform desktop notifications + scheduling + click actions.
//!
//! Backends:
//! - **Windows**: `tauri-winrt-notification` (Toast XML via WinRT).
//! - **macOS**: `mac-notification-sys` (UNNotificationCenter wrapper).
//! - **Linux**: `notify-rust` (D-Bus `org.freedesktop.Notifications`).

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, LazyLock,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, TrySendError};
use nutype::nutype;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};

#[cfg(target_os = "windows")]
use crate::ipc::error::IpcError;

use crate::api::HostApi;
use crate::ipc::events;
use crate::tynd;

/// Validated identifier for a notification action button. Constructed
/// via `NotifActionId::try_new(...)` (or auto-validated through serde
/// `Deserialize`) so malformed payloads are rejected at the IPC edge
/// rather than surfacing as opaque OS-side failures further down.
fn valid_action_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[nutype(
    validate(predicate = valid_action_id),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        Display,
        Serialize,
        Deserialize,
        AsRef
    )
)]
pub(crate) struct NotifActionId(String);

/// Click-action button shape — `id` is what the host emits on
/// `notification:action`; `label` is what the user sees.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct NotifAction {
    pub(crate) id: NotifActionId,
    pub(crate) label: String,
}

/// Stateless dispatch marker.
#[derive(Debug, Default)]
pub(crate) struct NotificationApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    shutdown: Some(shutdown),
    release_webview: Some(release_webview),
    ..HostApi::new("notification", |method, args| dispatch(method, &args))
};

#[tynd::handler(api = "notification")]
impl NotificationApi {
    /// Send a notification immediately. Each action button fires
    /// `notification:action { action: <id> }` when clicked.
    pub fn send(
        title: Option<String>,
        body: Option<String>,
        icon: Option<String>,
        sound: Option<String>,
        actions: Option<Vec<NotifAction>>,
    ) -> Result<(), String> {
        let icon = authorize_icon(icon.as_deref())?;
        let actions = actions.unwrap_or_default();
        let actions_pairs: Vec<(String, String)> = actions
            .into_iter()
            .map(|a| (a.id.into_inner(), a.label))
            .collect();
        send_platform(
            title.as_deref().unwrap_or(""),
            body.as_deref().unwrap_or(""),
            icon.as_deref(),
            sound.as_deref(),
            actions_pairs,
        )
        .map(|_| ())
    }

    /// Schedule a notification to fire after `delayMs`. Returns
    /// `{ id }` — pass that id back to `cancel` to abort.
    pub fn schedule(
        delay_ms: u64,
        title: Option<String>,
        body: Option<String>,
        icon: Option<String>,
        sound: Option<String>,
        actions: Option<Vec<NotifAction>>,
    ) -> Result<Value, String> {
        let icon = authorize_icon(icon.as_deref())?;
        let payload = json!({
            "title": title,
            "body": body,
            "icon": icon,
            "sound": sound,
            "actions": actions
                .unwrap_or_default()
                .into_iter()
                .map(|a| json!({ "id": a.id, "label": a.label }))
                .collect::<Vec<_>>(),
        });
        schedule_with_delay(delay_ms, payload)
    }

    /// Cancel a scheduled notification by id. Returns `true` when an
    /// entry was found and aborted, `false` when no matching id was
    /// pending.
    pub fn cancel(id: u64) -> bool {
        let mut state = SCHEDULER.lock();
        // A notification scheduled by another webview answers like an
        // unknown id.
        let permitted = state
            .pending
            .get(&id)
            .is_some_and(|owner| crate::ipc::owner::permits(owner.as_ref()));
        if !permitted {
            return false;
        }
        state.pending.remove(&id);
        if let Some(scheduler) = state.scheduler.as_ref() {
            // The shared pending set is authoritative: cancellation remains
            // effective even when the bounded command queue is already full.
            let _ = scheduler.tx.try_send(SchedulerCommand::Cancel(id));
        }
        true
    }

    /// Permission probe — every supported OS treats this as
    /// always-granted (the user-facing prompt is OS-side and out of
    /// our control).
    pub fn check_permission() -> String {
        "granted".to_string()
    }

    /// Permission request — every supported OS treats this as
    /// always-granted (see `checkPermission`).
    pub fn request_permission() -> String {
        "granted".to_string()
    }
}

const MAX_SCHEDULED_NOTIFICATIONS: usize = 1024;
const SCHEDULER_QUEUE_CAPACITY: usize = MAX_SCHEDULED_NOTIFICATIONS * 2 + 1;

enum SchedulerCommand {
    Schedule {
        id: u64,
        deadline: Instant,
        payload: Value,
    },
    Cancel(u64),
    Shutdown,
}

struct SchedulerTask {
    tx: Sender<SchedulerCommand>,
    stopping: Arc<AtomicBool>,
    join: JoinHandle<()>,
}

#[derive(Default)]
struct SchedulerState {
    shutting_down: bool,
    /// Pending schedule id → scheduling webview (`None` for the backend).
    pending: ahash::HashMap<u64, Option<crate::ipc::owner::Owner>>,
    scheduler: Option<SchedulerTask>,
}

static SCHEDULER: LazyLock<Mutex<SchedulerState>> =
    LazyLock::new(|| Mutex::new(SchedulerState::default()));
static NEXT_SCHEDULE_ID: AtomicU64 = AtomicU64::new(1);

fn start_scheduler() -> Result<SchedulerTask, String> {
    let (tx, rx) = crossbeam_channel::bounded(SCHEDULER_QUEUE_CAPACITY);
    let stopping = Arc::new(AtomicBool::new(false));
    let worker_stopping = Arc::clone(&stopping);
    let join = std::thread::Builder::new()
        .name("tynd-notification-scheduler".into())
        .spawn(move || run_scheduler(&rx, &worker_stopping))
        .map_err(|error| format!("failed to spawn notification scheduler: {error}"))?;
    Ok(SchedulerTask { tx, stopping, join })
}

fn schedule_with_delay(delay_ms: u64, args_owned: Value) -> Result<Value, String> {
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(delay_ms))
        .ok_or("notification.schedule: delayMs is too large")?;
    let mut state = SCHEDULER.lock();
    if state.shutting_down {
        return Err("notification.schedule: host is shutting down".into());
    }
    if state.pending.len() >= MAX_SCHEDULED_NOTIFICATIONS {
        return Err(format!(
            "notification.schedule: at most {MAX_SCHEDULED_NOTIFICATIONS} notifications may be pending"
        ));
    }

    if state.scheduler.is_none() {
        state.scheduler = Some(start_scheduler()?);
    }
    // Counter: Relaxed is enough for monotonic uniqueness.
    let id = NEXT_SCHEDULE_ID.fetch_add(1, Ordering::Relaxed);
    // Registered under the scheduler lock, which `release_webview` also takes:
    // a call that was still running when its webview committed a new document
    // (the generation has moved on) is backend-owned from the start, exactly as
    // if the release had seen it.
    let owner = crate::ipc::owner::current().filter(crate::ipc::owner::Owner::is_current);
    state.pending.insert(id, owner);
    let send_result = state
        .scheduler
        .as_ref()
        .expect("scheduler initialized above")
        .tx
        .try_send(SchedulerCommand::Schedule {
            id,
            deadline,
            payload: args_owned,
        });
    if let Err(error) = send_result {
        state.pending.remove(&id);
        return Err(match error {
            TrySendError::Full(_) => "notification.schedule: scheduler queue is full".into(),
            TrySendError::Disconnected(_) => {
                "notification.schedule: scheduler stopped unexpectedly".into()
            },
        });
    }
    Ok(json!({ "id": id }))
}

fn run_scheduler(rx: &Receiver<SchedulerCommand>, stopping: &AtomicBool) {
    let mut scheduled = ahash::HashMap::<u64, (Instant, Value)>::default();
    while !stopping.load(Ordering::Acquire) {
        let next_wait = scheduled
            .values()
            .map(|(deadline, _)| deadline.saturating_duration_since(Instant::now()))
            .min();
        let command = match next_wait {
            Some(wait) => match rx.recv_timeout(wait) {
                Ok(command) => Some(command),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => None,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            },
        };

        match command {
            Some(SchedulerCommand::Schedule {
                id,
                deadline,
                payload,
            }) => {
                let still_pending = {
                    let state = SCHEDULER.lock();
                    !state.shutting_down && state.pending.contains_key(&id)
                };
                if still_pending {
                    scheduled.insert(id, (deadline, payload));
                }
            },
            Some(SchedulerCommand::Cancel(id)) => {
                scheduled.remove(&id);
            },
            Some(SchedulerCommand::Shutdown) => break,
            None => fire_due(&mut scheduled),
        }
    }
    scheduled.clear();
}

/// Send every scheduled notification whose deadline has passed and that is
/// still pending (not cancelled, host not shutting down).
fn fire_due(scheduled: &mut ahash::HashMap<u64, (Instant, Value)>) {
    let now = Instant::now();
    let due: Vec<u64> = scheduled
        .iter()
        .filter_map(|(id, (deadline, _))| (*deadline <= now).then_some(*id))
        .collect();
    for id in due {
        let Some((_, payload)) = scheduled.remove(&id) else {
            continue;
        };
        let should_send = {
            let mut state = SCHEDULER.lock();
            !state.shutting_down && state.pending.remove(&id).is_some()
        };
        if should_send {
            if let Err(error) = send_from_value(&payload) {
                tracing::error!("scheduled notification {id} failed: {error}");
            }
        }
    }
}

/// The document of webview `label` is gone: its pending notifications still
/// fire but are re-owned to the backend, so a later document of the label
/// cannot cancel them (unless it is app-local, like any backend resource).
pub(crate) fn release_webview(label: &str) {
    let mut state = SCHEDULER.lock();
    for owner in state.pending.values_mut() {
        if owner.as_ref().is_some_and(|owner| owner.label() == label) {
            *owner = None;
        }
    }
}

pub(crate) fn shutdown() {
    let scheduler = {
        let mut state = SCHEDULER.lock();
        state.shutting_down = true;
        state.pending.clear();
        state.scheduler.take()
    };
    if let Some(scheduler) = scheduler {
        scheduler.stopping.store(true, Ordering::Release);
        let _ = scheduler.tx.try_send(SchedulerCommand::Shutdown);
        let _ = scheduler.join.join();
    }
}

fn send_from_value(args: &Value) -> Result<Value, String> {
    let title = args.get("title").and_then(Value::as_str).unwrap_or("");
    let body = args.get("body").and_then(Value::as_str).unwrap_or("");
    let icon = authorize_icon(args.get("icon").and_then(Value::as_str))?;
    let sound = args.get("sound").and_then(Value::as_str);
    let actions: Vec<(String, String)> = args
        .get("actions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| {
                    let id = v.get("id").and_then(Value::as_str)?;
                    let label = v.get("label").and_then(Value::as_str)?;
                    Some((id.to_string(), label.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    send_platform(title, body, icon.as_deref(), sound, actions)
}

/// Resolve notification image paths both when scheduling and immediately
/// before delivery. Scheduled notifications can outlive filesystem changes;
/// the second check closes symlink escapes introduced while they wait.
fn authorize_icon(icon: Option<&str>) -> Result<Option<String>, String> {
    icon.map(|path| {
        let resolved = crate::security::policy::authorize_fs_path(path)?;
        resolved.into_os_string().into_string().map_err(|path| {
            format!(
                "notification: resolved icon path '{}' is not valid UTF-8",
                path.to_string_lossy()
            )
        })
    })
    .transpose()
}

fn emit_action(id: &str) {
    events::emit("notification:action", &json!({ "action": id }));
}

/// Cap concurrent `wait_for_action` threads so a runaway caller can't spawn
/// thousands of blocked OS threads. Drops new waits once we're at the cap —
/// the notification still appears but its clicks go unhandled.
#[cfg(all(unix, not(target_os = "macos")))]
const MAX_ACTION_WAITERS: usize = 16;
#[cfg(all(unix, not(target_os = "macos")))]
use std::sync::atomic::AtomicUsize;
#[cfg(all(unix, not(target_os = "macos")))]
static ACTION_WAITERS: AtomicUsize = AtomicUsize::new(0);

#[cfg(target_os = "macos")]
const MAX_ACTION_WAITERS_MACOS: usize = 16;
#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicUsize;
#[cfg(target_os = "macos")]
static ACTION_WAITERS_MACOS: AtomicUsize = AtomicUsize::new(0);

#[cfg(all(unix, not(target_os = "macos")))]
fn send_platform(
    title: &str,
    body: &str,
    icon: Option<&str>,
    sound: Option<&str>,
    actions: Vec<(String, String)>,
) -> Result<Value, String> {
    use notify_rust::Notification;

    let app_name = exe_stem();
    let mut n = Notification::new();
    n.appname(&app_name).summary(title).body(body);
    if let Some(p) = icon {
        n.icon(p);
    }
    if let Some(s) = sound {
        n.sound_name(s);
    }
    for (id, label) in &actions {
        n.action(id, label);
    }

    let handle = n.show().map_err(|e| e.to_string())?;
    if !actions.is_empty() {
        let prev = ACTION_WAITERS.fetch_add(1, Ordering::AcqRel);
        if prev >= MAX_ACTION_WAITERS {
            ACTION_WAITERS.fetch_sub(1, Ordering::AcqRel);
        } else if let Err(error) = std::thread::Builder::new()
            .name("tynd-notification-action".into())
            .spawn(move || {
                handle.wait_for_action(emit_action);
                ACTION_WAITERS.fetch_sub(1, Ordering::AcqRel);
            })
        {
            ACTION_WAITERS.fetch_sub(1, Ordering::AcqRel);
            return Err(format!(
                "failed to spawn notification action waiter: {error}"
            ));
        }
    }
    Ok(Value::Null)
}

#[cfg(target_os = "windows")]
fn send_platform(
    title: &str,
    body: &str,
    icon: Option<&str>,
    sound: Option<&str>,
    actions: Vec<(String, String)>,
) -> Result<Value, String> {
    use std::str::FromStr as _;

    use tauri_winrt_notification::{IconCrop, Sound, Toast};

    let app_id = exe_stem();
    let mut toast = Toast::new(&app_id).title(title).text1(body);

    if let Some(p) = icon {
        toast = toast.icon(std::path::Path::new(p), IconCrop::Square, "");
    }
    if let Some(s) = sound {
        if let Ok(snd) = Sound::from_str(s) {
            toast = toast.sound(Some(snd));
        }
    }
    for (id, label) in &actions {
        // `action` arg on the button is what `on_activated` receives when
        // the user clicks — use our public id as the payload.
        toast = toast.add_button(label, id);
    }
    if !actions.is_empty() {
        toast = toast.on_activated(move |arg| {
            if let Some(action) = arg {
                emit_action(&action);
            }
            Ok(())
        });
    }

    toast
        .show()
        .map_err(|e| IpcError::new("notification", "send", format!("{e:?}")).into_string())?;
    Ok(Value::Null)
}

#[cfg(target_os = "macos")]
fn send_platform(
    title: &str,
    body: &str,
    icon: Option<&str>,
    sound: Option<&str>,
    actions: Vec<(String, String)>,
) -> Result<Value, String> {
    use mac_notification_sys::{MainButton, Notification, NotificationResponse};

    // Owned strings so we can move them into the action-waiting thread.
    let title_owned = title.to_string();
    let body_owned = body.to_string();
    let icon_owned = icon.map(str::to_string);
    let sound_owned = sound.map(str::to_string);

    let build = move |actions: &[(String, String)]| -> Result<NotificationResponse, String> {
        let labels: Vec<&str> = actions.iter().map(|(_, l)| l.as_str()).collect();
        let mut n = Notification::new();
        n.title(&title_owned).message(&body_owned);
        if let Some(ref s) = sound_owned {
            n.sound(s);
        }
        if let Some(ref p) = icon_owned {
            n.content_image(p);
        }
        match labels.len() {
            0 => {},
            1 => {
                n.main_button(MainButton::SingleAction(labels[0]));
            },
            _ => {
                n.main_button(MainButton::DropdownActions("Actions", &labels));
            },
        }
        n.send().map_err(|e| e.to_string())
    };

    if actions.is_empty() {
        let _ = build(&[])?;
    } else {
        // `send()` blocks until the user clicks or the banner is dismissed.
        // Off-thread so the call pool keeps serving unrelated requests.
        // Cap concurrent waiters to bound OS thread spawn under runaway callers.
        let prev = ACTION_WAITERS_MACOS.fetch_add(1, Ordering::AcqRel);
        if prev >= MAX_ACTION_WAITERS_MACOS {
            ACTION_WAITERS_MACOS.fetch_sub(1, Ordering::AcqRel);
            let _ = build(&actions)?;
        } else {
            if let Err(error) = std::thread::Builder::new()
                .name("tynd-notification-action".into())
                .spawn(move || {
                    if let Ok(NotificationResponse::ActionButton(label)) = build(&actions) {
                        if let Some((id, _)) = actions.iter().find(|(_, l)| *l == label) {
                            emit_action(id);
                        }
                    }
                    ACTION_WAITERS_MACOS.fetch_sub(1, Ordering::AcqRel);
                })
            {
                ACTION_WAITERS_MACOS.fetch_sub(1, Ordering::AcqRel);
                return Err(format!(
                    "failed to spawn notification action waiter: {error}"
                ));
            }
        }
    }
    Ok(Value::Null)
}

#[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
fn exe_stem() -> String {
    // Only Windows + Linux platform branches use this helper — macOS reaches
    // identity via mac-notification-sys' `set_application` instead. Gating the
    // function prevents dead_code lints on the unused macOS build.
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "tynd".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::owner::with_caller;

    #[test]
    fn cancel_by_another_webview_answers_like_an_unknown_id() {
        let scheduled = with_caller("nt-a", || schedule_with_delay(3_600_000, json!({}))).unwrap();
        let id = scheduled["id"].as_u64().unwrap();

        assert!(!with_caller("nt-b", || NotificationApi::cancel(id)));
        assert!(!with_caller("nt-b", || NotificationApi::cancel(u64::MAX)));
        assert!(SCHEDULER.lock().pending.contains_key(&id));

        assert!(with_caller("nt-a", || NotificationApi::cancel(id)));
        assert!(!SCHEDULER.lock().pending.contains_key(&id));
    }

    #[test]
    fn release_reowns_pending_notifications_to_the_backend() {
        use crate::ipc::owner::with_caller_trust;

        let scheduled = with_caller("nt-d", || schedule_with_delay(3_600_000, json!({}))).unwrap();
        let id = scheduled["id"].as_u64().unwrap();

        release_webview("nt-d");
        assert_eq!(SCHEDULER.lock().pending.get(&id), Some(&None));

        // The next document of the label is remote: it cannot cancel the
        // backend-owned notification, exactly like an unknown id.
        assert!(!with_caller_trust("nt-d", false, || {
            NotificationApi::cancel(id)
        }));
        assert!(SCHEDULER.lock().pending.contains_key(&id));
        assert!(with_caller_trust("nt-d", true, || NotificationApi::cancel(
            id
        )));
    }

    #[test]
    fn backend_may_cancel_a_webview_notification() {
        let scheduled = with_caller("nt-c", || schedule_with_delay(3_600_000, json!({}))).unwrap();
        let id = scheduled["id"].as_u64().unwrap();
        assert!(NotificationApi::cancel(id));
    }

    #[test]
    fn schedule_after_the_document_was_replaced_is_backend_owned() {
        let label = "nt-late";
        let scheduled = with_caller(label, || {
            // The commit lands while the schedule call is still running.
            crate::ipc::owner::bump_generation(label);
            schedule_with_delay(3_600_000, json!({}))
        })
        .unwrap();
        let id = scheduled["id"].as_u64().unwrap();
        assert_eq!(SCHEDULER.lock().pending.get(&id), Some(&None));
        assert!(NotificationApi::cancel(id));
    }
}

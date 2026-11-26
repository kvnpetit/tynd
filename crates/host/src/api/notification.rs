//! Cross-platform desktop notifications: send / schedule, action buttons and
//! registered action categories, inline reply, click / close events, the
//! list of delivered notifications and the permission state.
//!
//! Backends (`notification/<os>.rs`):
//! - **Windows**: WinRT toasts (`Windows.UI.Notifications`) built as toast XML.
//! - **macOS**: `mac-notification-sys` (`NSUserNotification`).
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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::HostApi;
use crate::ipc::owner::Owner;
use crate::tynd;

#[cfg(all(unix, not(target_os = "macos")))]
#[path = "notification/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "notification/macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "notification/windows.rs"]
mod platform;

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NotifAction {
    pub(crate) id: NotifActionId,
    pub(crate) label: String,
}

/// Inline reply field (`reply` option).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotifReply {
    /// Hint shown in the empty field.
    #[serde(default)]
    pub(crate) placeholder: Option<String>,
    /// Label of the send button.
    #[serde(default)]
    pub(crate) button_label: Option<String>,
}

/// `urgency` option.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

/// `timeout` option: `"default"`, `"never"` or milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum NotifTimeout {
    #[default]
    Default,
    Never,
    Millis(u32),
}

/// Most action buttons per notification (the Windows toast limit, applied on
/// every OS so a notification looks the same everywhere).
const MAX_ACTIONS: usize = 5;
/// Longest title / body / label / placeholder accepted.
const MAX_TEXT: usize = 4096;
const MAX_LABEL: usize = 128;
/// Registered categories per process.
const MAX_CATEGORIES: usize = 64;
/// Delivered notifications tracked for `getDelivered` / `remove`.
const MAX_DELIVERED: usize = 256;

/// Everything needed to show one notification.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NotifSpec {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) icon: Option<String>,
    pub(crate) image: Option<String>,
    pub(crate) sound: Option<String>,
    pub(crate) silent: bool,
    pub(crate) urgency: Urgency,
    pub(crate) timeout: NotifTimeout,
    pub(crate) actions: Vec<NotifAction>,
    pub(crate) reply: Option<NotifReply>,
}

fn invalid(op: &str, msg: impl std::fmt::Display) -> String {
    format!("notification.{op}: {msg}")
}

/// `timeout` wire value → [`NotifTimeout`].
fn parse_timeout(value: Option<&Value>, op: &str) -> Result<NotifTimeout, String> {
    match value {
        None | Some(Value::Null) => Ok(NotifTimeout::Default),
        Some(Value::String(s)) if s == "default" => Ok(NotifTimeout::Default),
        Some(Value::String(s)) if s == "never" => Ok(NotifTimeout::Never),
        Some(Value::Number(n)) => n
            .as_u64()
            .filter(|&ms| ms > 0)
            .and_then(|ms| u32::try_from(ms).ok())
            .map(NotifTimeout::Millis)
            .ok_or_else(|| invalid(op, "timeout must be a positive number of milliseconds")),
        Some(_) => Err(invalid(
            op,
            "timeout must be \"default\", \"never\" or a number of milliseconds",
        )),
    }
}

fn check_text(op: &str, what: &str, value: &str, max: usize) -> Result<(), String> {
    if value.len() > max {
        return Err(invalid(op, format!("{what} is longer than {max} bytes")));
    }
    Ok(())
}

fn check_actions(op: &str, actions: &[NotifAction]) -> Result<(), String> {
    if actions.len() > MAX_ACTIONS {
        return Err(invalid(
            op,
            format!(
                "at most {MAX_ACTIONS} actions are allowed, got {}",
                actions.len()
            ),
        ));
    }
    for (index, action) in actions.iter().enumerate() {
        if action.label.trim().is_empty() {
            return Err(invalid(
                op,
                format!("action '{}' has an empty label", action.id),
            ));
        }
        check_text(op, "action label", &action.label, MAX_LABEL)?;
        if actions[..index].iter().any(|other| other.id == action.id) {
            return Err(invalid(op, format!("duplicate action id '{}'", action.id)));
        }
    }
    Ok(())
}

/// Sound names are platform identifiers (`default`, `Mail`, `Glass`...).
fn check_sound(op: &str, sound: &str) -> Result<(), String> {
    let valid = !sound.is_empty()
        && sound.len() <= 64
        && sound
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(invalid(op, format!("invalid sound name '{sound}'")))
    }
}

/// Wire options shared by `send` and `schedule`.
struct SpecArgs {
    title: Option<String>,
    body: Option<String>,
    icon: Option<String>,
    image: Option<String>,
    sound: Option<String>,
    silent: Option<bool>,
    urgency: Option<Urgency>,
    timeout: Option<Value>,
    actions: Option<Vec<NotifAction>>,
    category: Option<String>,
    reply: Option<NotifReply>,
}

impl SpecArgs {
    /// Validate the options into a [`NotifSpec`], resolving `category`.
    fn into_spec(self, op: &str) -> Result<NotifSpec, String> {
        let actions = match (self.actions, self.category) {
            (Some(_), Some(_)) => {
                return Err(invalid(op, "pass either actions or category, not both"));
            },
            (Some(actions), None) => actions,
            (None, Some(category)) => CATEGORIES.lock().get(&category).cloned().ok_or_else(|| {
                invalid(
                    op,
                    format!(
                        "unknown category '{category}'; register it with notification.registerActions first"
                    ),
                )
            })?,
            (None, None) => Vec::new(),
        };
        check_actions(op, &actions)?;
        let title = self.title.unwrap_or_default();
        let body = self.body.unwrap_or_default();
        check_text(op, "title", &title, MAX_TEXT)?;
        check_text(op, "body", &body, MAX_TEXT)?;
        if let Some(sound) = self.sound.as_deref() {
            check_sound(op, sound)?;
        }
        if let Some(reply) = &self.reply {
            for text in [&reply.placeholder, &reply.button_label]
                .into_iter()
                .flatten()
            {
                check_text(op, "reply text", text, MAX_LABEL)?;
            }
        }
        let spec = NotifSpec {
            title,
            body,
            icon: authorize_image(op, self.icon.as_deref())?,
            image: authorize_image(op, self.image.as_deref())?,
            sound: self.sound,
            silent: self.silent.unwrap_or(false),
            urgency: self.urgency.unwrap_or_default(),
            timeout: parse_timeout(self.timeout.as_ref(), op)?,
            actions,
            reply: self.reply,
        };
        platform::check_supported(&spec, op)?;
        Ok(spec)
    }
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
    /// Show a notification now and return `{ id }`. Clicks, action buttons,
    /// replies and dismissals fire `notification:click`, `:action`, `:reply`
    /// and `:close` with that id.
    #[allow(
        clippy::too_many_arguments,
        reason = "one parameter per wire option of notification.send"
    )]
    pub fn send(
        title: Option<String>,
        body: Option<String>,
        icon: Option<String>,
        image: Option<String>,
        sound: Option<String>,
        silent: Option<bool>,
        urgency: Option<Urgency>,
        timeout: Option<Value>,
        actions: Option<Vec<NotifAction>>,
        category: Option<String>,
        reply: Option<NotifReply>,
    ) -> Result<Value, String> {
        let spec = SpecArgs {
            title,
            body,
            icon,
            image,
            sound,
            silent,
            urgency,
            timeout,
            actions,
            category,
            reply,
        }
        .into_spec("send")?;
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let owner = crate::ipc::owner::current().filter(Owner::is_current);
        deliver(id, &spec, owner)?;
        Ok(json!({ "id": id }))
    }

    /// Schedule a notification to fire after `delayMs`. Returns `{ id }`:
    /// `cancel(id)` aborts it while pending, and once shown it is the id of
    /// its events and of `remove`.
    #[allow(
        clippy::too_many_arguments,
        reason = "one parameter per wire option of notification.schedule"
    )]
    pub fn schedule(
        delay_ms: u64,
        title: Option<String>,
        body: Option<String>,
        icon: Option<String>,
        image: Option<String>,
        sound: Option<String>,
        silent: Option<bool>,
        urgency: Option<Urgency>,
        timeout: Option<Value>,
        actions: Option<Vec<NotifAction>>,
        category: Option<String>,
        reply: Option<NotifReply>,
    ) -> Result<Value, String> {
        let spec = SpecArgs {
            title,
            body,
            icon,
            image,
            sound,
            silent,
            urgency,
            timeout,
            actions,
            category,
            reply,
        }
        .into_spec("schedule")?;
        schedule_with_delay(delay_ms, spec)
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

    /// Register (or replace) the action buttons of `category`, so `send` /
    /// `schedule` can pass `category` instead of repeating `actions`.
    pub fn register_actions(category: String, actions: Vec<NotifAction>) -> Result<(), String> {
        let valid_name = !category.is_empty()
            && category.len() <= 64
            && category
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
        if !valid_name {
            return Err(invalid(
                "registerActions",
                format!(
                    "invalid category '{category}' (1-64 chars: letters, digits, '_', '-', '.')"
                ),
            ));
        }
        if actions.is_empty() {
            return Err(invalid("registerActions", "pass at least one action"));
        }
        check_actions("registerActions", &actions)?;
        let mut categories = CATEGORIES.lock();
        if !categories.contains_key(&category) && categories.len() >= MAX_CATEGORIES {
            return Err(invalid(
                "registerActions",
                format!("at most {MAX_CATEGORIES} categories may be registered"),
            ));
        }
        categories.insert(category, actions);
        Ok(())
    }

    /// Notifications this app showed that are still on screen or in the
    /// notification center: `[{ id, title, body }]`. Windows and Linux.
    pub fn get_delivered() -> Result<Value, String> {
        platform::check_delivered_supported("getDelivered")?;
        let live = platform::live_ids();
        let delivered = DELIVERED.lock();
        let list: Vec<Value> = delivered
            .iter()
            .filter(|(id, entry)| {
                crate::ipc::owner::permits(entry.owner.as_ref())
                    && live.as_ref().is_none_or(|live| live.contains(id))
            })
            .map(|(id, entry)| json!({ "id": id, "title": entry.title, "body": entry.body }))
            .collect();
        Ok(Value::Array(list))
    }

    /// Remove a delivered notification from screen and notification center.
    /// Returns `false` for an unknown id. Windows and Linux.
    pub fn remove(id: u64) -> Result<bool, String> {
        platform::check_delivered_supported("remove")?;
        let entry = {
            let mut delivered = DELIVERED.lock();
            let permitted = delivered
                .get(&id)
                .is_some_and(|entry| crate::ipc::owner::permits(entry.owner.as_ref()));
            if !permitted {
                return Ok(false);
            }
            delivered.remove(&id)
        };
        if let Some(entry) = entry {
            platform::remove(id, &entry.handle)?;
        }
        Ok(true)
    }

    /// Remove every delivered notification the caller may see. Returns how
    /// many were removed. Windows and Linux.
    pub fn remove_all() -> Result<u64, String> {
        platform::check_delivered_supported("removeAll")?;
        let removed: Vec<(u64, Delivered)> = {
            let mut delivered = DELIVERED.lock();
            let ids: Vec<u64> = delivered
                .iter()
                .filter(|(_, entry)| crate::ipc::owner::permits(entry.owner.as_ref()))
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| delivered.remove(&id).map(|entry| (id, entry)))
                .collect()
        };
        let mut count = 0;
        for (id, entry) in &removed {
            platform::remove(*id, &entry.handle)?;
            count += 1;
        }
        Ok(count)
    }

    /// Permission state: `"granted"` or `"denied"` (Windows reads the
    /// user / policy setting; macOS and Linux have no queryable state and
    /// answer `"granted"`).
    pub fn check_permission() -> String {
        platform::permission().to_string()
    }

    /// No OS backend can prompt: answers the current state, like
    /// `checkPermission`.
    pub fn request_permission() -> String {
        platform::permission().to_string()
    }
}

/// Registered action categories.
static CATEGORIES: LazyLock<Mutex<ahash::HashMap<String, Vec<NotifAction>>>> =
    LazyLock::new(|| Mutex::new(ahash::HashMap::default()));

/// A notification on screen or in the notification center.
pub(crate) struct Delivered {
    title: String,
    body: String,
    /// Webview that showed it (`None` for the backend); receives its events.
    owner: Option<Owner>,
    handle: platform::Handle,
}

/// Delivered notifications by id (ordered so the oldest is evicted first).
static DELIVERED: LazyLock<Mutex<std::collections::BTreeMap<u64, Delivered>>> =
    LazyLock::new(|| Mutex::new(std::collections::BTreeMap::new()));

/// Ids of `send` notifications and `schedule` entries (shared counter).
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Authorize `spec`'s images again, show it and track it as delivered.
fn deliver(id: u64, spec: &NotifSpec, owner: Option<Owner>) -> Result<(), String> {
    // Scheduled notifications can outlive filesystem changes: re-resolve
    // the images right before delivery to close symlink escapes.
    let spec = NotifSpec {
        icon: authorize_image("send", spec.icon.as_deref())?,
        image: authorize_image("send", spec.image.as_deref())?,
        ..spec.clone()
    };
    // Tracked before showing: an instant click must find its owner.
    let mut delivered = DELIVERED.lock();
    while delivered.len() >= MAX_DELIVERED {
        delivered.pop_first();
    }
    drop(delivered);
    let handle = platform::show(id, &spec)?;
    DELIVERED.lock().insert(
        id,
        Delivered {
            title: spec.title,
            body: spec.body,
            owner,
            handle,
        },
    );
    Ok(())
}

/// What the user did with a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotifEvent {
    Click,
    Action(String),
    Reply(String),
    Close(&'static str),
}

/// Emit `event` of notification `id` to its owner. A click, an action, a
/// reply and a user dismissal end the notification; a timeout leaves it in
/// the notification center (Windows) and keeps it delivered.
pub(crate) fn emit_event(id: u64, event: &NotifEvent) {
    let ends = !matches!(event, NotifEvent::Close("timeout"));
    let owner = {
        let mut delivered = DELIVERED.lock();
        let owner = delivered.get(&id).map(|entry| entry.owner.clone());
        if ends {
            delivered.remove(&id);
        }
        owner
    };
    // A notification evicted from the table is backend-owned.
    let owner = owner.flatten();
    let (name, payload) = match event {
        NotifEvent::Click => ("notification:click", json!({ "id": id })),
        NotifEvent::Action(action) => {
            ("notification:action", json!({ "id": id, "action": action }))
        },
        NotifEvent::Reply(text) => ("notification:reply", json!({ "id": id, "text": text })),
        NotifEvent::Close(reason) => ("notification:close", json!({ "id": id, "reason": reason })),
    };
    crate::ipc::owner::emit_for(owner.as_ref(), name, &payload);
}

const MAX_SCHEDULED_NOTIFICATIONS: usize = 1024;
const SCHEDULER_QUEUE_CAPACITY: usize = MAX_SCHEDULED_NOTIFICATIONS * 2 + 1;

enum SchedulerCommand {
    Schedule {
        id: u64,
        deadline: Instant,
        spec: Box<NotifSpec>,
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
    pending: ahash::HashMap<u64, Option<Owner>>,
    scheduler: Option<SchedulerTask>,
}

static SCHEDULER: LazyLock<Mutex<SchedulerState>> =
    LazyLock::new(|| Mutex::new(SchedulerState::default()));

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

fn schedule_with_delay(delay_ms: u64, spec: NotifSpec) -> Result<Value, String> {
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
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    // Registered under the scheduler lock, which `release_webview` also takes:
    // a call that was still running when its webview committed a new document
    // (the generation has moved on) is backend-owned from the start, exactly as
    // if the release had seen it.
    let owner = crate::ipc::owner::current().filter(Owner::is_current);
    state.pending.insert(id, owner);
    let send_result = state
        .scheduler
        .as_ref()
        .expect("scheduler initialized above")
        .tx
        .try_send(SchedulerCommand::Schedule {
            id,
            deadline,
            spec: Box::new(spec),
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
    let mut scheduled = ahash::HashMap::<u64, (Instant, Box<NotifSpec>)>::default();
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
            Some(SchedulerCommand::Schedule { id, deadline, spec }) => {
                let still_pending = {
                    let state = SCHEDULER.lock();
                    !state.shutting_down && state.pending.contains_key(&id)
                };
                if still_pending {
                    scheduled.insert(id, (deadline, spec));
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
fn fire_due(scheduled: &mut ahash::HashMap<u64, (Instant, Box<NotifSpec>)>) {
    let now = Instant::now();
    let due: Vec<u64> = scheduled
        .iter()
        .filter_map(|(id, (deadline, _))| (*deadline <= now).then_some(*id))
        .collect();
    for id in due {
        let Some((_, spec)) = scheduled.remove(&id) else {
            continue;
        };
        let owner = {
            let mut state = SCHEDULER.lock();
            if state.shutting_down {
                None
            } else {
                state.pending.remove(&id)
            }
        };
        if let Some(owner) = owner {
            if let Err(error) = deliver(id, &spec, owner) {
                tracing::error!("scheduled notification {id} failed: {error}");
            }
        }
    }
}

/// The document of webview `label` is gone: its pending and delivered
/// notifications are re-owned to the backend, so a later document of the
/// label cannot cancel or remove them (unless it is app-local, like any
/// backend resource).
pub(crate) fn release_webview(label: &str) {
    let mut state = SCHEDULER.lock();
    for owner in state.pending.values_mut() {
        if owner.as_ref().is_some_and(|owner| owner.label() == label) {
            *owner = None;
        }
    }
    drop(state);
    for entry in DELIVERED.lock().values_mut() {
        if entry
            .owner
            .as_ref()
            .is_some_and(|owner| owner.label() == label)
        {
            entry.owner = None;
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

/// Resolve a notification image path through the `fs` scope.
fn authorize_image(op: &str, path: Option<&str>) -> Result<Option<String>, String> {
    path.map(|path| {
        let resolved = crate::security::policy::authorize_fs_path(path)?;
        resolved.into_os_string().into_string().map_err(|path| {
            invalid(
                op,
                format!(
                    "resolved image path '{}' is not valid UTF-8",
                    path.to_string_lossy()
                ),
            )
        })
    })
    .transpose()
}

/// Cap on threads blocked waiting for a notification's response (Linux and
/// macOS backends): past it, notifications still show but fire no events.
#[cfg(unix)]
pub(crate) const MAX_RESPONSE_WAITERS: usize = 16;

#[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
pub(crate) fn exe_stem() -> String {
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

    fn spec_args() -> SpecArgs {
        SpecArgs {
            title: Some("t".into()),
            body: None,
            icon: None,
            image: None,
            sound: None,
            silent: None,
            urgency: None,
            timeout: None,
            actions: None,
            category: None,
            reply: None,
        }
    }

    fn action(id: &str, label: &str) -> NotifAction {
        NotifAction {
            id: NotifActionId::try_new(id.to_owned()).unwrap(),
            label: label.to_owned(),
        }
    }

    #[test]
    fn timeout_values_are_parsed() {
        assert_eq!(parse_timeout(None, "send"), Ok(NotifTimeout::Default));
        assert_eq!(
            parse_timeout(Some(&json!("never")), "send"),
            Ok(NotifTimeout::Never)
        );
        assert_eq!(
            parse_timeout(Some(&json!(5000)), "send"),
            Ok(NotifTimeout::Millis(5000))
        );
        for bad in [json!(0), json!(-1), json!("soon"), json!(true), json!(1.5)] {
            assert!(parse_timeout(Some(&bad), "send").is_err(), "{bad}");
        }
    }

    #[test]
    fn actions_are_validated() {
        let mut args = spec_args();
        args.actions = Some((0..6).map(|i| action(&format!("a{i}"), "L")).collect());
        let e = args.into_spec("send").unwrap_err();
        assert!(e.contains("at most 5 actions"), "{e}");

        let mut args = spec_args();
        args.actions = Some(vec![action("a", "One"), action("a", "Two")]);
        assert!(args
            .into_spec("send")
            .unwrap_err()
            .contains("duplicate action id"));

        let mut args = spec_args();
        args.actions = Some(vec![action("a", "  ")]);
        assert!(args.into_spec("send").unwrap_err().contains("empty label"));

        let e = serde_json::from_value::<NotifAction>(json!({ "id": "bad id", "label": "x" }));
        assert!(e.is_err());
    }

    #[test]
    fn categories_resolve_to_registered_actions() {
        NotificationApi::register_actions(
            "test-cat".into(),
            vec![action("yes", "Yes"), action("no", "No")],
        )
        .unwrap();
        let mut args = spec_args();
        args.category = Some("test-cat".into());
        let spec = args.into_spec("send").unwrap();
        assert_eq!(spec.actions.len(), 2);

        let mut args = spec_args();
        args.category = Some("missing-cat".into());
        let e = args.into_spec("send").unwrap_err();
        assert!(e.contains("unknown category 'missing-cat'"), "{e}");

        let mut args = spec_args();
        args.category = Some("test-cat".into());
        args.actions = Some(vec![action("x", "X")]);
        assert!(args.into_spec("send").unwrap_err().contains("not both"));

        let e = NotificationApi::register_actions("bad name".into(), vec![action("a", "A")])
            .unwrap_err();
        assert!(e.contains("invalid category"), "{e}");
        let e = NotificationApi::register_actions("empty".into(), Vec::new()).unwrap_err();
        assert!(e.contains("at least one action"), "{e}");
    }

    #[test]
    fn sound_names_are_validated() {
        let mut args = spec_args();
        args.sound = Some("Mail".into());
        assert!(args.into_spec("send").is_ok());
        let mut args = spec_args();
        args.sound = Some("../x".into());
        assert!(args
            .into_spec("send")
            .unwrap_err()
            .contains("invalid sound name"));
    }

    #[test]
    fn events_reach_the_owner_and_end_the_notification() {
        let owner = with_caller("nt-ev", crate::ipc::owner::current);
        DELIVERED.lock().insert(
            900_001,
            Delivered {
                title: "t".into(),
                body: String::new(),
                owner,
                handle: platform::Handle::default(),
            },
        );
        emit_event(900_001, &NotifEvent::Close("timeout"));
        assert!(DELIVERED.lock().contains_key(&900_001));
        emit_event(900_001, &NotifEvent::Click);
        assert!(!DELIVERED.lock().contains_key(&900_001));
    }

    #[test]
    fn remove_by_another_webview_answers_like_an_unknown_id() {
        if platform::check_delivered_supported("remove").is_err() {
            return;
        }
        let owner = with_caller("nt-rm-a", crate::ipc::owner::current);
        DELIVERED.lock().insert(
            900_002,
            Delivered {
                title: "t".into(),
                body: String::new(),
                owner,
                handle: platform::Handle::default(),
            },
        );
        assert_eq!(
            with_caller("nt-rm-b", || NotificationApi::remove(900_002)),
            Ok(false)
        );
        assert!(DELIVERED.lock().contains_key(&900_002));
        let listed = with_caller("nt-rm-b", NotificationApi::get_delivered).unwrap();
        assert!(!listed.to_string().contains("900002"));
        release_webview("nt-rm-a");
        assert!(DELIVERED.lock()[&900_002].owner.is_none());
        DELIVERED.lock().remove(&900_002);
    }

    #[test]
    fn cancel_by_another_webview_answers_like_an_unknown_id() {
        let scheduled = with_caller("nt-a", || {
            schedule_with_delay(3_600_000, NotifSpec::default())
        })
        .unwrap();
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

        let scheduled = with_caller("nt-d", || {
            schedule_with_delay(3_600_000, NotifSpec::default())
        })
        .unwrap();
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
        let scheduled = with_caller("nt-c", || {
            schedule_with_delay(3_600_000, NotifSpec::default())
        })
        .unwrap();
        let id = scheduled["id"].as_u64().unwrap();
        assert!(NotificationApi::cancel(id));
    }

    #[test]
    fn schedule_after_the_document_was_replaced_is_backend_owned() {
        let label = "nt-late";
        let scheduled = with_caller(label, || {
            // The commit lands while the schedule call is still running.
            crate::ipc::owner::bump_generation(label);
            schedule_with_delay(3_600_000, NotifSpec::default())
        })
        .unwrap();
        let id = scheduled["id"].as_u64().unwrap();
        assert_eq!(SCHEDULER.lock().pending.get(&id), Some(&None));
        assert!(NotificationApi::cancel(id));
    }
}

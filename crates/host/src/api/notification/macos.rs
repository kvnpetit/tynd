//! macOS backend: `mac-notification-sys` (`NSUserNotification`).
//!
//! `send()` blocks until the user responds, so every notification gets a
//! waiter thread (capped at [`MAX_RESPONSE_WAITERS`]; past it the
//! notification shows without events). The API offers no list or removal of
//! delivered notifications, so those calls reject.

use std::{
    collections::HashSet,
    sync::atomic::{AtomicUsize, Ordering},
};

use mac_notification_sys::{MainButton, Notification, NotificationResponse};

use super::{emit_event, NotifEvent, NotifSpec, MAX_RESPONSE_WAITERS};

/// Nothing to keep: delivered notifications cannot be removed on macOS.
#[derive(Default)]
pub(super) struct Handle;

static WAITERS: AtomicUsize = AtomicUsize::new(0);

pub(super) fn check_supported(spec: &NotifSpec, op: &str) -> Result<(), String> {
    if spec.reply.is_some() && !spec.actions.is_empty() {
        return Err(format!(
            "notification.{op}: reply and actions cannot be combined on macos"
        ));
    }
    Ok(())
}

pub(super) fn check_delivered_supported(op: &str) -> Result<(), String> {
    Err(format!(
        "notification.{op}: listing and removing delivered notifications is not supported on macos"
    ))
}

pub(super) fn live_ids() -> Option<HashSet<u64>> {
    None
}

pub(super) fn remove(_id: u64, _handle: &Handle) -> Result<(), String> {
    check_delivered_supported("remove")
}

/// `NSUserNotification` has no queryable authorization state.
pub(super) fn permission() -> &'static str {
    "granted"
}

/// Show `spec`; `wait` blocks until the user responds.
fn send(spec: &NotifSpec, wait: bool) -> Result<NotificationResponse, String> {
    let labels: Vec<&str> = spec.actions.iter().map(|a| a.label.as_str()).collect();
    let mut n = Notification::new();
    n.title(&spec.title).message(&spec.body);
    if !spec.silent {
        match spec.sound.as_deref() {
            Some("default") => {
                n.default_sound();
            },
            Some(sound) => {
                n.sound(sound);
            },
            None => {},
        }
    }
    if let Some(image) = spec.image.as_deref().or(spec.icon.as_deref()) {
        n.content_image(image);
    }
    if let Some(reply) = &spec.reply {
        n.main_button(MainButton::Response(
            reply.placeholder.as_deref().unwrap_or(""),
        ));
    } else {
        match labels.len() {
            0 => {},
            1 => {
                n.main_button(MainButton::SingleAction(labels[0]));
            },
            _ => {
                n.main_button(MainButton::DropdownActions("Actions", &labels));
            },
        }
    }
    n.wait_for_click(wait);
    n.send().map_err(|e| format!("notification.send: {e}"))
}

fn response_event(spec: &NotifSpec, response: NotificationResponse) -> NotifEvent {
    match response {
        NotificationResponse::ActionButton(label) => spec
            .actions
            .iter()
            .find(|action| action.label == label)
            .map_or(NotifEvent::Click, |action| {
                NotifEvent::Action(action.id.as_ref().to_owned())
            }),
        NotificationResponse::Reply(text) => NotifEvent::Reply(text),
        NotificationResponse::Click => NotifEvent::Click,
        NotificationResponse::CloseButton(_) => NotifEvent::Close("user"),
        NotificationResponse::None => NotifEvent::Close("unknown"),
    }
}

pub(super) fn show(id: u64, spec: &NotifSpec) -> Result<Handle, String> {
    let previous = WAITERS.fetch_add(1, Ordering::AcqRel);
    if previous >= MAX_RESPONSE_WAITERS {
        WAITERS.fetch_sub(1, Ordering::AcqRel);
        send(spec, false)?;
        return Ok(Handle);
    }
    let owned = spec.clone();
    let spawned = std::thread::Builder::new()
        .name("tynd-notification-response".into())
        .spawn(move || {
            match send(&owned, true) {
                Ok(response) => emit_event(id, &response_event(&owned, response)),
                Err(error) => tracing::error!("notification {id} failed: {error}"),
            }
            WAITERS.fetch_sub(1, Ordering::AcqRel);
        });
    if let Err(error) = spawned {
        WAITERS.fetch_sub(1, Ordering::AcqRel);
        return Err(format!(
            "notification.send: failed to spawn the response waiter: {error}"
        ));
    }
    Ok(Handle)
}

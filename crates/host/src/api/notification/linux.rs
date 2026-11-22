//! Linux backend: `notify-rust` over D-Bus `org.freedesktop.Notifications`.
//!
//! Every notification gets a waiter thread for its response (click, action,
//! close), capped at [`MAX_RESPONSE_WAITERS`]. Removal calls
//! `CloseNotification` with the server's id.

use std::{
    collections::HashSet,
    sync::atomic::{AtomicUsize, Ordering},
};

use notify_rust::{CloseReason, Hint, Notification, NotificationResponse, Timeout};

use super::{
    emit_event, exe_stem, NotifEvent, NotifSpec, NotifTimeout, Urgency, MAX_RESPONSE_WAITERS,
};

/// D-Bus id the notification server gave the notification.
#[derive(Default)]
pub(super) struct Handle(u32);

static WAITERS: AtomicUsize = AtomicUsize::new(0);

pub(super) fn check_supported(spec: &NotifSpec, op: &str) -> Result<(), String> {
    if spec.reply.is_some() {
        return Err(format!(
            "notification.{op}: inline reply is not supported on linux"
        ));
    }
    Ok(())
}

pub(super) fn check_delivered_supported(_op: &str) -> Result<(), String> {
    Ok(())
}

/// The server keeps no queryable history: the host's own tracking is used.
pub(super) fn live_ids() -> Option<HashSet<u64>> {
    None
}

/// Desktop notifications have no permission model.
pub(super) fn permission() -> &'static str {
    "granted"
}

pub(super) fn remove(_id: u64, handle: &Handle) -> Result<(), String> {
    let error = |e: zbus::Error| format!("notification.remove: {e}");
    let connection = zbus::blocking::Connection::session().map_err(error)?;
    connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "CloseNotification",
            &(handle.0,),
        )
        .map(|_| ())
        .map_err(error)
}

fn response_event(response: &NotificationResponse) -> NotifEvent {
    match response {
        NotificationResponse::Default => NotifEvent::Click,
        NotificationResponse::Action(action) => NotifEvent::Action(action.clone()),
        NotificationResponse::Reply(text) => NotifEvent::Reply(text.clone()),
        NotificationResponse::Closed(reason) => NotifEvent::Close(match reason {
            CloseReason::Expired => "timeout",
            CloseReason::Dismissed => "user",
            CloseReason::CloseAction => "app",
            CloseReason::Other(_) => "unknown",
        }),
    }
}

pub(super) fn show(id: u64, spec: &NotifSpec) -> Result<Handle, String> {
    let app_name = exe_stem();
    let mut n = Notification::new();
    n.appname(&app_name).summary(&spec.title).body(&spec.body);
    if let Some(icon) = &spec.icon {
        n.icon(icon);
    }
    if let Some(image) = &spec.image {
        n.image_path(image);
    }
    if spec.silent {
        n.hint(Hint::SuppressSound(true));
    } else if let Some(sound) = &spec.sound {
        n.sound_name(sound);
    }
    n.urgency(match spec.urgency {
        Urgency::Low => notify_rust::Urgency::Low,
        Urgency::Normal => notify_rust::Urgency::Normal,
        Urgency::Critical => notify_rust::Urgency::Critical,
    });
    n.timeout(match spec.timeout {
        NotifTimeout::Default => Timeout::Default,
        NotifTimeout::Never => Timeout::Never,
        NotifTimeout::Millis(ms) => Timeout::Milliseconds(ms),
    });
    // The "default" action is what servers invoke on a body click.
    n.action("default", "");
    for action in &spec.actions {
        n.action(action.id.as_ref(), &action.label);
    }

    let handle = n.show().map_err(|e| format!("notification.send: {e}"))?;
    let dbus_id = handle.id();
    let previous = WAITERS.fetch_add(1, Ordering::AcqRel);
    if previous >= MAX_RESPONSE_WAITERS {
        WAITERS.fetch_sub(1, Ordering::AcqRel);
        return Ok(Handle(dbus_id));
    }
    let spawned = std::thread::Builder::new()
        .name("tynd-notification-response".into())
        .spawn(move || {
            let _ = handle.wait_for_response(|response: &NotificationResponse| {
                emit_event(id, &response_event(response));
            });
            WAITERS.fetch_sub(1, Ordering::AcqRel);
        });
    if let Err(error) = spawned {
        WAITERS.fetch_sub(1, Ordering::AcqRel);
        tracing::warn!("notification {id}: no response events ({error})");
    }
    Ok(Handle(dbus_id))
}

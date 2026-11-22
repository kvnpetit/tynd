//! `NSWorkspace.notificationCenter` observer blocks bridged into
//! `crate::ipc::events::emit`. Observer tokens remain on their owner thread and
//! are explicitly removed during central host shutdown.
use std::sync::mpsc;

use objc2_app_kit::{
    objc2::{rc::Retained, runtime::AnyObject},
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
    NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSNotificationName};
use serde_json::json;

use super::{join_workers, MONITOR_START_TIMEOUT};
use crate::ipc::events;

pub(super) struct Monitor {
    stop: mpsc::SyncSender<()>,
    join: std::thread::JoinHandle<()>,
}

impl Monitor {
    pub(super) fn shutdown(self) {
        let _ = self.stop.send(());
        join_workers(vec![self.join], "power[mac]");
    }
}

impl super::MonitorHandle for Monitor {
    fn shutdown(self) {
        Monitor::shutdown(self);
    }

    fn is_alive(&self) -> bool {
        !self.join.is_finished()
    }
}

pub(super) fn spawn_watcher() -> Result<Monitor, String> {
    let (stop_tx, stop_rx) = mpsc::sync_channel(1);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let join = std::thread::Builder::new()
        .name("tynd-power-mac".into())
        .spawn(move || run(stop_rx, ready_tx))
        .map_err(|error| format!("power[mac]: watcher thread spawn failed: {error}"))?;

    match ready_rx.recv_timeout(MONITOR_START_TIMEOUT) {
        Ok(()) => Ok(Monitor {
            stop: stop_tx,
            join,
        }),
        Err(error) => {
            let _ = stop_tx.send(());
            join_workers(vec![join], "power[mac]");
            Err(format!("power[mac]: watcher readiness failed: {error}"))
        },
    }
}

fn run(stop: mpsc::Receiver<()>, ready: mpsc::SyncSender<()>) {
    // SAFETY: documented singleton accessors with refcount tracked by
    // objc2. All observer tokens stay on this worker thread.
    let workspace = NSWorkspace::sharedWorkspace();
    let center = workspace.notificationCenter();
    let tokens = unsafe {
        vec![
            register(&center, NSWorkspaceWillSleepNotification, "power:suspend"),
            register(&center, NSWorkspaceDidWakeNotification, "power:resume"),
            register(
                &center,
                NSWorkspaceSessionDidResignActiveNotification,
                "power:lock",
            ),
            register(
                &center,
                NSWorkspaceSessionDidBecomeActiveNotification,
                "power:unlock",
            ),
        ]
    };

    if ready.send(()).is_ok() {
        let _ = stop.recv();
    }

    for token in &tokens {
        // SAFETY: each token came from this center's block observer API
        // and is removed before its final retained reference is dropped.
        unsafe { center.removeObserver(token) };
    }
}

/// # Safety
/// `center` must be valid and `name` one of NSWorkspace's notification
/// constants. Nil queue runs block on posting thread; nil object observes
/// every sender.
unsafe fn register(
    center: &NSNotificationCenter,
    name: &NSNotificationName,
    event: &'static str,
) -> Retained<AnyObject> {
    let block = block2::RcBlock::new(move |_notification: std::ptr::NonNull<NSNotification>| {
        events::emit(event, &json!({}));
    });
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
    };
    token.into()
}

//! Power-source poller for the macOS and Linux backends, which have no
//! cheap AC/battery notification wired: reads [`super::power_status`] every
//! [`POLL_EVERY`] and emits `power:source-change` on a flip. Windows gets the
//! same event from `GUID_ACDC_POWER_SOURCE` notifications instead.
use std::{sync::mpsc, thread::JoinHandle, time::Duration};

use serde_json::{json, Value};

use super::{join_workers, source_transition, MonitorHandle};
use crate::ipc::events;

const POLL_EVERY: Duration = Duration::from_secs(5);

/// An OS event watcher paired with the power-source poller; both stop
/// together.
pub(super) struct WithSourcePoll<T> {
    os: T,
    stop: mpsc::SyncSender<()>,
    join: JoinHandle<()>,
}

impl<T: MonitorHandle> WithSourcePoll<T> {
    pub(super) fn start(os: T) -> Result<Self, String> {
        let (stop, stop_rx) = mpsc::sync_channel(1);
        let join = match std::thread::Builder::new()
            .name("tynd-power-source".into())
            .spawn(move || poll(&stop_rx))
        {
            Ok(join) => join,
            Err(error) => {
                os.shutdown();
                return Err(format!("power: source poller spawn failed: {error}"));
            },
        };
        Ok(Self { os, stop, join })
    }
}

impl<T: MonitorHandle> MonitorHandle for WithSourcePoll<T> {
    fn shutdown(self) {
        let _ = self.stop.send(());
        join_workers(vec![self.join], "power[source]");
        self.os.shutdown();
    }

    fn is_alive(&self) -> bool {
        self.os.is_alive() && !self.join.is_finished()
    }
}

fn on_battery() -> Option<bool> {
    super::power_status()
        .get("onBattery")
        .and_then(Value::as_bool)
}

fn poll(stop: &mpsc::Receiver<()>) {
    let mut previous = on_battery();
    while let Err(mpsc::RecvTimeoutError::Timeout) = stop.recv_timeout(POLL_EVERY) {
        let now = on_battery();
        if let Some(on_battery) = source_transition(previous, now) {
            events::emit("power:source-change", &json!({ "onBattery": on_battery }));
        }
        if now.is_some() {
            previous = now;
        }
    }
}

//! Window lifecycle notifications: the Bun backend (`tynd:window` frame) and
//! native plugins (`HostEvent::WindowCreated` / `WindowClosed`) hear about the
//! same transitions.

use crossbeam_channel::Sender;

use tynd_protocol::runtime::{BackendCall, WindowEventKind};

/// Tell Bun and native plugins that window `label` now exists.
pub(super) fn window_created(call_tx: &Sender<BackendCall>, label: &str) {
    notify(call_tx, WindowEventKind::Created, label);
    #[cfg(feature = "plugins")]
    crate::plugins::broadcast_event(&crate::plugins::HostEvent::WindowCreated {
        label: label.to_owned(),
    });
}

/// Tell Bun and native plugins that window `label` was destroyed.
pub(super) fn window_closed(call_tx: &Sender<BackendCall>, label: &str) {
    notify(call_tx, WindowEventKind::Closed, label);
    #[cfg(feature = "plugins")]
    crate::plugins::broadcast_event(&crate::plugins::HostEvent::WindowClosed {
        label: label.to_owned(),
    });
}

fn notify(call_tx: &Sender<BackendCall>, kind: WindowEventKind, label: &str) {
    // A closed channel means the backend is already gone; nothing to tell.
    let _ = call_tx.send(BackendCall::window_event(kind, label));
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn window_event_args(call: BackendCall) -> (String, Vec<Value>) {
        match call {
            BackendCall::Typed { fn_name, args, .. } => (fn_name, args),
            other => panic!("expected a typed call, got {other:?}"),
        }
    }

    #[test]
    fn created_and_closed_send_window_event_frames_in_order() {
        let (tx, rx) = crossbeam_channel::unbounded();
        window_created(&tx, "settings");
        window_closed(&tx, "settings");

        let (name, args) = window_event_args(rx.try_recv().expect("created frame"));
        assert_eq!(name, tynd_protocol::runtime::WINDOW_EVENT_FN);
        assert_eq!(args, vec![Value::from("created"), Value::from("settings")]);

        let (_, args) = window_event_args(rx.try_recv().expect("closed frame"));
        assert_eq!(args, vec![Value::from("closed"), Value::from("settings")]);
        assert!(rx.try_recv().is_err(), "exactly two frames");
    }

    #[test]
    fn closed_backend_channel_is_not_an_error() {
        let (tx, rx) = crossbeam_channel::unbounded();
        drop(rx);
        window_created(&tx, "main");
        window_closed(&tx, "main");
    }
}

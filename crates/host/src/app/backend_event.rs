use serde_json::Value;
use tao::event_loop::ControlFlow;
use wry::WebView;

use crate::ipc::events::EventScope;

use super::{
    dispatch_support::{broadcast_script, broadcast_script_remote, broadcast_script_scoped},
    runtime::{BackendRuntimeState, WindowRegistry},
    UserEvent,
};
use crate::{ipc::dispatch, webview::shim};
use tynd_protocol::runtime::{BackendEvent, BackendReturnPayload};

/// What a backend `Return` id stands for. The renderer can never choose a
/// reserved id (`dispatch::valid_call_id`), so these only come from the host's
/// own lifecycle calls.
#[derive(Debug, PartialEq, Eq)]
enum ReturnId {
    /// The backend finished its `on_close` hook: the host exits now.
    CloseAck,
    /// Another framework-internal call: nothing to deliver to a webview.
    Reserved,
    /// A renderer call: resolve its promise.
    Call,
}

fn classify_return_id(id: &str) -> ReturnId {
    if id == "__tynd_on_close__" {
        ReturnId::CloseAck
    } else if id.starts_with("__tynd_") {
        ReturnId::Reserved
    } else {
        ReturnId::Call
    }
}

/// Reason shown to remote-origin pages when the backend dies.
const REMOTE_BACKEND_DEAD_MESSAGE: &str = "backend exited";

pub(super) struct BackendEventContext<'a> {
    pub(super) backend: &'a mut BackendRuntimeState,
    pub(super) windows: &'a WindowRegistry,
    pub(super) control_flow: &'a mut ControlFlow,
}

impl BackendEventContext<'_> {
    fn handle_return(&mut self, id: String, payload: BackendReturnPayload) {
        #[cfg(feature = "debug-protocol")]
        if crate::debug::complete_ipc_replay(&id, &payload) {
            return;
        }
        // An `app.protocol(...)` request the runtime could not deliver.
        if let BackendReturnPayload::Err(error) = &payload {
            if crate::webview::protocols::fail_backend_protocol(&id, error) {
                return;
            }
        }
        match classify_return_id(&id) {
            ReturnId::CloseAck => {
                if self.backend.begin_exit() {
                    crate::cleanup::run();
                    *self.control_flow = ControlFlow::Exit;
                }
                return;
            },
            ReturnId::Reserved => return,
            ReturnId::Call => {},
        }

        self.backend.flush_yields(self.windows);
        let Some((_, label)) = dispatch::CALL_LABELS.remove(&id) else {
            crate::tynd_warn!("dropping backend return for unknown call id={id}");
            return;
        };
        let label = label.to_string();
        let Some(webview) = self.webview_for(&label) else {
            crate::tynd_warn!("dropping backend return for closed webview id={id} label={label:?}");
            return;
        };
        let (ok, value) = match &payload {
            BackendReturnPayload::Ok(value) => (true, value.as_str()),
            BackendReturnPayload::Err(value) => (false, value.as_str()),
        };
        #[cfg(feature = "debug-protocol")]
        crate::debug::record_ipc_response(&id, ok, value);
        if let Err(error) = webview.evaluate_script(&shim::eval_resolve(&id, ok, value)) {
            crate::tynd_warn!("ipc reply eval failed for id={id} label={label}: {error}");
            let _ = self.backend.proxy().send_event(UserEvent::OsResult {
                label,
                id,
                ok: false,
                value: Value::String(format!("frontend eval failed: {error}")),
            });
        }
    }

    fn handle_yield(&mut self, id: String, value: String) {
        let Some(label) = dispatch::CALL_LABELS
            .get(&id)
            .map(|entry| entry.value().as_str().to_string())
        else {
            crate::tynd_warn!("dropping backend yield for unknown call id={id}");
            return;
        };
        if self.backend.push_yield(label, id, value) {
            self.backend.flush_yields(self.windows);
        } else {
            self.backend.schedule_yield_flush();
        }
    }

    fn handle_emit(&self, name: String, payload: String, to: Option<String>) {
        let script = shim::eval_dispatch(&name, &payload);
        if let Some(label) = to {
            if let Some(webview) = self.webview_for(&label) {
                let _ = webview.evaluate_script(&script);
            } else {
                crate::tynd_warn!("dropping backend event '{name}' for unknown webview '{label}'");
            }
        } else {
            self.broadcast_local(&script);
        }
    }

    fn handle_frontend_eval(&self, script: String, to: Option<String>) {
        if let Some(label) = to {
            if let Some(webview) = self.webview_for(&label) {
                let _ = webview.evaluate_script(&script);
            } else {
                crate::tynd_warn!("dropping frontend eval for unknown webview '{label}'");
            }
        } else {
            self.broadcast_local(&script);
        }
    }

    fn reload(&self) {
        self.broadcast(&shim::eval_hide_error_overlay());
        self.broadcast("window.location.reload();");
    }

    fn handle_error(&mut self, message: String) {
        dispatch::CALL_LABELS.clear();
        self.backend.clear_yields();
        // Crash text can carry paths and stack frames: never hand it to
        // remote-origin pages. They still get a message-free rejection so their
        // pending calls settle instead of hanging.
        self.broadcast_local(&shim::eval_backend_dead(&message));
        broadcast_script_remote(
            &shim::eval_backend_dead(REMOTE_BACKEND_DEAD_MESSAGE),
            &self.windows.webview,
            &self.windows.secondaries,
            &self.windows.child_webviews,
        );
        self.broadcast_local(&shim::eval_show_error_overlay(&message));
    }

    fn webview_for(&self, label: &str) -> Option<&WebView> {
        self.windows.webview_for(label)
    }

    /// Broadcast to app-local webviews only. Backend-originated events and
    /// errors are app-internal; a remote-origin webview only gets them when the
    /// backend targets it explicitly with `to`.
    fn broadcast_local(&self, script: &str) {
        broadcast_script_scoped(
            script,
            &EventScope::Local,
            &self.windows.webview,
            &self.windows.secondaries,
            &self.windows.child_webviews,
        );
    }

    fn broadcast(&self, script: &str) {
        broadcast_script(
            script,
            &self.windows.webview,
            &self.windows.secondaries,
            &self.windows.child_webviews,
        );
    }
}

pub(super) fn handle_backend_event(event: BackendEvent, mut context: BackendEventContext<'_>) {
    match event {
        BackendEvent::Return { id, payload } => context.handle_return(id, payload),
        BackendEvent::Yield { id, value } => context.handle_yield(id, value),
        BackendEvent::Emit { name, payload, to } => context.handle_emit(name, payload, to),
        BackendEvent::EvalInFrontend { script, to } => context.handle_frontend_eval(script, to),
        BackendEvent::Reload => context.reload(),
        BackendEvent::Error { message } => context.handle_error(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_ids_are_classified() {
        assert_eq!(classify_return_id("__tynd_on_close__"), ReturnId::CloseAck);
        assert_eq!(classify_return_id("__tynd_on_ready__"), ReturnId::Reserved);
        assert_eq!(classify_return_id("__tynd_"), ReturnId::Reserved);
        assert_eq!(classify_return_id("42"), ReturnId::Call);
        assert_eq!(classify_return_id("a__tynd_on_close__"), ReturnId::Call);
    }
}

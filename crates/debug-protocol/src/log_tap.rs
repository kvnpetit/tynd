//! Tracing layer that mirrors structured log events to debug subscribers.
//!
//! Activated only when the debug protocol is on. Frames are retained in a
//! bounded ring so `app.logs.tail` works before a live subscriber attaches.

use std::{
    collections::VecDeque,
    sync::{Arc, OnceLock},
};

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

/// One captured tracing event in a wire-stable shape.
#[derive(Debug, Clone, Serialize)]
pub struct LogFrame {
    pub ts: u64,
    pub level: &'static str,
    pub target: String,
    pub message: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub fields: Value,
}

#[derive(Debug)]
struct Tap {
    senders: Mutex<Vec<Sender<LogFrame>>>,
    recent: Mutex<VecDeque<LogFrame>>,
}

static TAP: OnceLock<Arc<Tap>> = OnceLock::new();
const RECENT_CAPACITY: usize = 1_000;

pub fn install() {
    let _ = TAP.set(Arc::new(Tap {
        senders: Mutex::new(Vec::new()),
        recent: Mutex::new(VecDeque::with_capacity(RECENT_CAPACITY)),
    }));
}

#[inline]
pub fn is_active() -> bool {
    TAP.get().is_some()
}

pub fn subscribe() -> Option<Receiver<LogFrame>> {
    let tap = TAP.get()?;
    let (tx, rx) = crossbeam_channel::bounded(1024);
    tap.senders.lock().push(tx);
    Some(rx)
}

pub fn broadcast(frame: LogFrame) {
    let Some(tap) = TAP.get() else { return };
    {
        let mut recent = tap.recent.lock();
        if recent.len() == RECENT_CAPACITY {
            recent.pop_front();
        }
        recent.push_back(frame.clone());
    }
    let mut senders = tap.senders.lock();
    senders.retain(|sender| {
        !matches!(
            sender.try_send(frame.clone()),
            Err(crossbeam_channel::TrySendError::Disconnected(_))
        )
    });
}

/// Return the newest `n` captured log frames in chronological order.
pub fn recent(n: usize) -> Vec<LogFrame> {
    let Some(tap) = TAP.get() else {
        return Vec::new();
    };
    let recent = tap.recent.lock();
    let take = n.min(recent.len());
    recent
        .iter()
        .skip(recent.len().saturating_sub(take))
        .cloned()
        .collect()
}

/// Tracing layer adapter — converts every event into a `LogFrame` and
/// hands it to the broadcast bus. Plug into a subscriber via
/// `tracing_subscriber::registry().with(DebugLogLayer)`.
#[derive(Debug)]
pub struct DebugLogLayer;

impl<S> tracing_subscriber::Layer<S> for DebugLogLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        // Skip serialization when the debug protocol was never installed.
        if !is_active() {
            return;
        }
        let metadata = event.metadata();
        let level = match *metadata.level() {
            tracing::Level::ERROR => "error",
            tracing::Level::WARN => "warn",
            tracing::Level::INFO => "info",
            tracing::Level::DEBUG => "debug",
            tracing::Level::TRACE => "trace",
        };

        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);

        broadcast(LogFrame {
            ts: super::ipc_tap::now_ms(),
            level,
            target: metadata.target().to_string(),
            message: visitor.message,
            fields: if visitor.fields.is_empty() {
                Value::Null
            } else {
                json!(visitor.fields)
            },
        });
    }
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: serde_json::Map<String, Value>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields
                .insert(field.name().to_string(), Value::String(value.to_string()));
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let formatted = format!("{value:?}");
        if field.name() == "message" {
            self.message = formatted;
        } else {
            self.fields
                .insert(field.name().to_string(), Value::String(formatted));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields
            .insert(field.name().to_string(), Value::from(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields
            .insert(field.name().to_string(), Value::from(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .insert(field.name().to_string(), Value::from(value));
    }
}

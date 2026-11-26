//! Broadcasts IPC envelopes to attached debug subscribers.
//!
//! Until [`install`] runs (the debug protocol is off), [`record`] is a single
//! `OnceLock` load: the host hot path pays for one branch, no allocation, no
//! work. Once installed, every envelope also lands in the bounded ring that
//! `ipc.recent` reads.

use std::sync::{Arc, OnceLock};

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;

/// Captured snapshot of one IPC envelope crossing the host boundary.
#[derive(Debug, Clone, Serialize)]
pub struct IpcFrame {
    pub ts: u64,
    pub direction: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub args: Value,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub result: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Bounded ring of recent envelopes — `ipc.recent` reads from this even
/// without an active subscription.
const RECENT_CAPACITY: usize = 256;

struct Tap {
    senders: Mutex<Vec<Sender<IpcFrame>>>,
    recent: Mutex<std::collections::VecDeque<IpcFrame>>,
}

impl Tap {
    fn new() -> Self {
        Self {
            senders: Mutex::new(Vec::new()),
            recent: Mutex::new(std::collections::VecDeque::with_capacity(RECENT_CAPACITY)),
        }
    }
}

/// Largest `args` / `result` payload kept per envelope (serialized JSON bytes).
/// Bigger values are replaced by a marker. The ring holds 256 frames and an
/// `ipc.recent` / `ipc.failed` reply carries up to about three such values per
/// frame (`payload` plus a repeated `error`), so 16 KiB keeps a full reply near
/// 12 MiB, below `MAX_FRAME_BYTES` (16 MiB) that clients enforce.
pub const MAX_TAP_VALUE_BYTES: usize = 16 * 1024;

/// `io::Write` that only counts bytes, aborting once `limit` is exceeded.
struct SizeProbe {
    written: usize,
    limit: usize,
}

impl std::io::Write for SizeProbe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.written += buf.len();
        if self.written > self.limit {
            return Err(std::io::Error::other("over limit"));
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Keep `value` when it serializes within [`MAX_TAP_VALUE_BYTES`]; otherwise
/// return a small `{"truncated": true, "limit_bytes": N}` marker.
pub(crate) fn cap_value(value: Value) -> Value {
    if value.is_null() {
        return value;
    }
    let mut probe = SizeProbe {
        written: 0,
        limit: MAX_TAP_VALUE_BYTES,
    };
    if serde_json::to_writer(&mut probe, &value).is_ok() {
        value
    } else {
        serde_json::json!({ "truncated": true, "limit_bytes": MAX_TAP_VALUE_BYTES })
    }
}

static TAP: OnceLock<Arc<Tap>> = OnceLock::new();

/// Initialize the tap. Idempotent — second call is a no-op.
pub fn install() {
    let _ = TAP.set(Arc::new(Tap::new()));
}

#[inline]
pub fn is_installed() -> bool {
    TAP.get().is_some()
}

/// Push an envelope into the ring + every active subscription. No-op
/// when the tap was never initialized.
pub fn record(mut frame: IpcFrame) {
    let Some(tap) = TAP.get() else { return };
    frame.args = cap_value(std::mem::take(&mut frame.args));
    frame.result = cap_value(std::mem::take(&mut frame.result));
    {
        let mut ring = tap.recent.lock();
        if ring.len() == RECENT_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(frame.clone());
    }
    tap.senders.lock().retain(|sender| {
        !matches!(
            sender.try_send(frame.clone()),
            Err(crossbeam_channel::TrySendError::Disconnected(_))
        )
    });
}

/// Subscribe to future envelopes. The receiver is dropped when the
/// caller stops polling.
pub fn subscribe() -> Option<Receiver<IpcFrame>> {
    let tap = TAP.get()?;
    let (tx, rx) = crossbeam_channel::bounded(1024);
    tap.senders.lock().push(tx);
    Some(rx)
}

/// Return the last `n` envelopes (newest last). Used by `ipc.recent`.
pub fn recent(n: usize) -> Vec<IpcFrame> {
    let Some(tap) = TAP.get() else {
        return Vec::new();
    };
    let ring = tap.recent.lock();
    let take = n.min(ring.len());
    ring.iter()
        .skip(ring.len().saturating_sub(take))
        .cloned()
        .collect()
}

/// Convenience: timestamp helper in milliseconds since UNIX epoch.
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

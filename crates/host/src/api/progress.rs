use std::time::{Duration, Instant};

use serde_json::json;

use crate::ipc::events;

const MIN_EMIT_INTERVAL: Duration = Duration::from_millis(50);

/// Shared byte-progress event emitter.
///
/// `emit_without_id` preserves the two existing wire contracts: updater
/// progress may be anonymous, while HTTP progress is opt-in via `progressId`.
pub(super) struct ProgressEmitter {
    event: &'static str,
    id: Option<String>,
    phase: &'static str,
    total: Option<u64>,
    loaded: u64,
    last_emit: Instant,
    emit_without_id: bool,
}

impl ProgressEmitter {
    pub(super) fn new(
        event: &'static str,
        id: Option<String>,
        phase: &'static str,
        total: Option<u64>,
        emit_without_id: bool,
    ) -> Self {
        Self {
            event,
            id,
            phase,
            total,
            loaded: 0,
            last_emit: Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(Instant::now),
            emit_without_id,
        }
    }

    pub(super) fn advance(&mut self, delta: usize, force: bool) {
        if !self.emit_without_id && self.id.is_none() {
            return;
        }
        self.loaded += delta as u64;
        if !force && self.last_emit.elapsed() < MIN_EMIT_INTERVAL {
            return;
        }
        self.last_emit = Instant::now();
        self.emit();
    }

    #[cfg(feature = "api-updater")]
    pub(super) fn emit_phase(&mut self, phase: &'static str) {
        self.phase = phase;
        self.emit();
    }

    fn emit(&self) {
        events::emit(
            self.event,
            &json!({
                "id": self.id,
                "phase": self.phase,
                "loaded": self.loaded,
                "total": self.total,
            }),
        );
    }
}

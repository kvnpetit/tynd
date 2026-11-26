use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
};

use cpal::Stream;
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use tynd_plugin_sdk::{EmitFn, PluginError, PluginResult};

use super::{capture::build_capture_stream, playback::build_playback_stream};
use crate::{CaptureOpts, PlaybackSlot};
/// Maximum number of control requests waiting behind a slow device
/// open/close operation.
const CONTROL_QUEUE_LIMIT: usize = 64;
const CONTROL_ENQUEUE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const START_ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Which kind of stream a `Stop` may remove: capture and playback share one id
/// space, so `stopCapture(playbackId)` must not end a playback.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StreamKind {
    Capture,
    Playback,
}

enum AudioCmd {
    StartCapture {
        id: u64,
        opts: CaptureOpts,
        emit: EmitFn,
        cancelled: Arc<AtomicBool>,
        ack: Sender<PluginResult<()>>,
    },
    StartPlayback {
        id: u64,
        samples: Arc<Vec<f32>>,
        sample_rate: u32,
        channels: u16,
        volume: f32,
        device: Option<String>,
        // Released when the stream is dropped (stop / EOF / error
        // unwinding the entry out of the registry below).
        slot: PlaybackSlot,
        cancelled: Arc<AtomicBool>,
        ack: Sender<PluginResult<()>>,
    },
    Stop {
        id: u64,
        kind: StreamKind,
        ack: Sender<bool>,
    },
    Shutdown {
        ack: Sender<()>,
    },
}

struct AudioControl {
    sender: Sender<AudioCmd>,
    join: parking_lot::Mutex<Option<JoinHandle<()>>>,
}

/// The live control thread, if any. `shutdown` clears it so a later start
/// spawns a fresh thread instead of failing on a dead queue until restart.
static CONTROL: parking_lot::Mutex<Option<Arc<AudioControl>>> = parking_lot::Mutex::new(None);

fn control() -> PluginResult<Arc<AudioControl>> {
    let mut slot = CONTROL.lock();
    if let Some(control) = slot.as_ref() {
        return Ok(Arc::clone(control));
    }
    let (tx, rx) = bounded::<AudioCmd>(CONTROL_QUEUE_LIMIT);
    let (playback_finished_tx, playback_finished_rx) = unbounded::<u64>();
    // EAGAIN here means the host is already at its thread limit.
    // Surface it as a plugin error instead of crashing the host; the next
    // call retries.
    let join = std::thread::Builder::new()
        .name("tynd-audio-control".into())
        .spawn(move || control_thread(rx, playback_finished_rx, playback_finished_tx))
        .map_err(|e| {
            tracing::error!(error = %e, "audio: failed to spawn control thread");
            PluginError::Custom(format!("audio: failed to spawn control thread: {e}"))
        })?;
    let control = Arc::new(AudioControl {
        sender: tx,
        join: parking_lot::Mutex::new(Some(join)),
    });
    *slot = Some(Arc::clone(&control));
    Ok(control)
}

fn cmd_sender() -> PluginResult<Sender<AudioCmd>> {
    Ok(control()?.sender.clone())
}

fn enqueue_control(command: AudioCmd, operation: &str) -> PluginResult<()> {
    cmd_sender()?
        .send_timeout(command, CONTROL_ENQUEUE_TIMEOUT)
        .map_err(|error| {
            PluginError::Custom(format!(
                "{operation}: control queue unavailable after {}s: {error}",
                CONTROL_ENQUEUE_TIMEOUT.as_secs()
            ))
        })
}

/// Owns every live stream — dropping a `Stream` runs cpal cleanup on
/// this same thread, satisfying the `!Send` contract. Playback
/// entries also own a `PlaybackSlot` so the playback-slot pool count
/// is released exactly when the entry is removed (`stop` / build
/// failure path drops the guard alongside the stream).
// Fields are write-only — held to keep the cpal stream + RAII guard
// alive on this thread; dropping the entry is the operational signal.
enum LiveStream {
    Capture {
        _stream: Stream,
    },
    Playback {
        _stream: Stream,
        _permit: PlaybackSlot,
    },
}

impl LiveStream {
    fn kind(&self) -> StreamKind {
        match self {
            Self::Capture { .. } => StreamKind::Capture,
            Self::Playback { .. } => StreamKind::Playback,
        }
    }
}

/// Live-stream table with the "start timed out" protocol: a start whose caller
/// gave up (`cancelled`) must never leave a stream nobody can stop.
struct Streams<T> {
    map: HashMap<u64, T>,
}

impl<T> Default for Streams<T> {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

impl<T> Streams<T> {
    /// Builds and inserts a stream unless the caller already cancelled.
    /// Re-checks after the (possibly slow) build, since the caller may time
    /// out while the device is opening. The built value is dropped on the
    /// control thread, which the `!Send` cpal streams require.
    fn start(
        &mut self,
        id: u64,
        cancelled: &AtomicBool,
        build: impl FnOnce() -> PluginResult<T>,
    ) -> PluginResult<()> {
        let cancelled_error = || PluginError::Custom(format!("audio: start of {id} was cancelled"));
        if cancelled.load(Ordering::Acquire) {
            return Err(cancelled_error());
        }
        let stream = build()?;
        if cancelled.load(Ordering::Acquire) {
            drop(stream);
            return Err(cancelled_error());
        }
        self.map.insert(id, stream);
        Ok(())
    }

    /// Removes `id` only when `matches` accepts the stored stream.
    fn remove_if(&mut self, id: u64, matches: impl FnOnce(&T) -> bool) -> bool {
        if self.map.get(&id).is_some_and(matches) {
            self.map.remove(&id).is_some()
        } else {
            false
        }
    }

    #[cfg(test)]
    fn remove(&mut self, id: u64) -> bool {
        self.map.remove(&id).is_some()
    }

    fn clear(&mut self) {
        self.map.clear();
    }
}

#[derive(Default)]
struct AudioState {
    streams: Streams<LiveStream>,
}

impl AudioState {
    fn handle(&mut self, command: AudioCmd, playback_finished: &Sender<u64>) -> bool {
        match command {
            AudioCmd::StartCapture {
                id,
                opts,
                emit,
                cancelled,
                ack,
            } => {
                let result = self.streams.start(id, &cancelled, || {
                    build_capture_stream(id, &opts, emit)
                        .map(|stream| LiveStream::Capture { _stream: stream })
                });
                let _ = ack.send(result);
                true
            },
            AudioCmd::StartPlayback {
                id,
                samples,
                sample_rate,
                channels,
                volume,
                device,
                slot,
                cancelled,
                ack,
            } => {
                // On failure or cancellation `slot` is dropped with the closure,
                // so a refused stream never leaks its playback slot.
                let result = self.streams.start(id, &cancelled, move || {
                    build_playback_stream(
                        id,
                        samples,
                        sample_rate,
                        channels,
                        volume,
                        device,
                        playback_finished.clone(),
                    )
                    .map(|stream| LiveStream::Playback {
                        _stream: stream,
                        _permit: slot,
                    })
                });
                let _ = ack.send(result);
                true
            },
            AudioCmd::Stop { id, kind, ack } => {
                // `LiveStream::Playback` drop runs the slot drop too.
                let removed = self.streams.remove_if(id, |stream| stream.kind() == kind);
                let _ = ack.send(removed);
                true
            },
            AudioCmd::Shutdown { ack } => {
                self.streams.clear();
                let _ = ack.send(());
                false
            },
        }
    }
}

fn control_thread(
    rx: Receiver<AudioCmd>,
    playback_finished_rx: Receiver<u64>,
    playback_finished_tx: Sender<u64>,
) {
    let mut state = AudioState::default();
    loop {
        crossbeam_channel::select! {
            recv(rx) -> command => {
                let Ok(command) = command else { break };
                if !state.handle(command, &playback_finished_tx) {
                    break;
                }
            }
            recv(playback_finished_rx) -> id => {
                if let Ok(id) = id {
                    state
                        .streams
                        .remove_if(id, |stream| stream.kind() == StreamKind::Playback);
                }
            }
        }
    }
}
pub(crate) fn start_capture(id: u64, opts: CaptureOpts, emit: EmitFn) -> PluginResult<u64> {
    let (ack_tx, ack_rx) = bounded::<PluginResult<()>>(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    enqueue_control(
        AudioCmd::StartCapture {
            id,
            opts,
            emit,
            cancelled: Arc::clone(&cancelled),
            ack: ack_tx,
        },
        "audio.startCapture",
    )?;
    await_start_ack(
        ack_rx,
        "audio.startCapture",
        id,
        StreamKind::Capture,
        &cancelled,
    )?;
    Ok(id)
}

pub(crate) fn stop_capture(id: u64) -> PluginResult<()> {
    let (ack_tx, ack_rx) = bounded::<bool>(1);
    enqueue_control(
        AudioCmd::Stop {
            id,
            kind: StreamKind::Capture,
            ack: ack_tx,
        },
        "audio.stopCapture",
    )?;
    match ack_rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::warn!(id, error = %e, "audio.stopCapture: control thread did not ack within 2s");
            Err(PluginError::Custom(format!(
                "audio.stopCapture: ack timeout for id {id} (stream may still be running)"
            )))
        },
    }
}

pub(super) struct PlaybackRequest {
    pub(super) id: u64,
    pub(super) samples: Vec<f32>,
    pub(super) sample_rate: u32,
    pub(super) channels: u16,
    pub(super) volume: f32,
    pub(super) device: Option<String>,
    pub(super) slot: PlaybackSlot,
}
pub(super) fn start_playback(request: PlaybackRequest, operation: &str) -> PluginResult<()> {
    let PlaybackRequest {
        id,
        samples,
        sample_rate,
        channels,
        volume,
        device,
        slot,
    } = request;
    let (ack_tx, ack_rx) = bounded::<PluginResult<()>>(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    enqueue_control(
        AudioCmd::StartPlayback {
            id,
            samples: Arc::new(samples),
            sample_rate,
            channels,
            volume,
            device,
            slot,
            cancelled: Arc::clone(&cancelled),
            ack: ack_tx,
        },
        operation,
    )?;
    await_start_ack(ack_rx, operation, id, StreamKind::Playback, &cancelled)
}

fn await_start_ack(
    receiver: Receiver<PluginResult<()>>,
    operation: &str,
    id: u64,
    kind: StreamKind,
    cancelled: &AtomicBool,
) -> PluginResult<()> {
    // A wedged hardware-open path must not hold dispatch permits forever.
    if let Ok(result) = receiver.recv_timeout(START_ACK_TIMEOUT) {
        return result;
    }
    // The caller is about to get an error and never learns `id`, so the queued
    // start must not later produce an unstoppable stream: flag it (checked
    // before and after the build) and also queue a Stop for the race where the
    // stream was inserted right as we gave up.
    cancelled.store(true, Ordering::Release);
    let (ack_tx, _ack_rx) = bounded::<bool>(1);
    let _ = cmd_sender().map(|sender| {
        sender.send_timeout(
            AudioCmd::Stop {
                id,
                kind,
                ack: ack_tx,
            },
            CONTROL_ENQUEUE_TIMEOUT,
        )
    });
    Err(PluginError::Custom(format!(
        "{operation}: ack timeout for id {id} (control thread wedged)"
    )))
}
pub(crate) fn stop_playback(id: u64) -> PluginResult<()> {
    let (ack_tx, ack_rx) = bounded::<bool>(1);
    enqueue_control(
        AudioCmd::Stop {
            id,
            kind: StreamKind::Playback,
            ack: ack_tx,
        },
        "audio.stopPlayback",
    )?;
    match ack_rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::warn!(id, error = %e, "audio.stopPlayback: control thread did not ack within 2s");
            Err(PluginError::Custom(format!(
                "audio.stopPlayback: ack timeout for id {id} (stream may still be running)"
            )))
        },
    }
}

pub(crate) fn shutdown() -> PluginResult<()> {
    // Clear the slot first: from here on a start spawns a fresh control thread
    // instead of queueing behind (or failing on) the one being stopped.
    let Some(control) = CONTROL.lock().take() else {
        return Ok(());
    };
    let Some(handle) = control.join.lock().take() else {
        return Ok(());
    };

    let (ack_tx, ack_rx) = bounded::<()>(1);
    control
        .sender
        .send_timeout(AudioCmd::Shutdown { ack: ack_tx }, CONTROL_ENQUEUE_TIMEOUT)
        .map_err(|error| {
            PluginError::Custom(format!(
                "audio.shutdown: control queue unavailable after {}s: {error}",
                CONTROL_ENQUEUE_TIMEOUT.as_secs()
            ))
        })?;
    ack_rx
        .recv_timeout(CONTROL_ENQUEUE_TIMEOUT)
        .map_err(|error| {
            PluginError::Custom(format!(
                "audio.shutdown: control thread did not acknowledge within {}s: {error}",
                CONTROL_ENQUEUE_TIMEOUT.as_secs()
            ))
        })?;
    handle
        .join()
        .map_err(|_| PluginError::Custom("audio.shutdown: control thread panicked".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_inserts_when_not_cancelled() {
        let mut streams = Streams::<u32>::default();
        let cancelled = AtomicBool::new(false);
        streams.start(1, &cancelled, || Ok(7)).unwrap();
        assert!(streams.remove(1));
        assert!(!streams.remove(1));
    }

    #[test]
    fn cancelled_before_build_never_builds() {
        let mut streams = Streams::<u32>::default();
        let cancelled = AtomicBool::new(true);
        let built = AtomicBool::new(false);
        let result = streams.start(1, &cancelled, || {
            built.store(true, Ordering::Release);
            Ok(7)
        });
        assert!(result.is_err());
        assert!(!built.load(Ordering::Acquire));
        assert!(!streams.remove(1));
    }

    #[test]
    fn cancelled_during_slow_build_drops_the_stream() {
        struct Tracked(Arc<AtomicBool>);
        impl Drop for Tracked {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let mut streams = Streams::<Tracked>::default();
        let cancelled = AtomicBool::new(false);
        let result = streams.start(1, &cancelled, || {
            // The caller times out while the device is still opening.
            cancelled.store(true, Ordering::Release);
            Ok(Tracked(Arc::clone(&dropped)))
        });
        assert!(result.is_err());
        assert!(dropped.load(Ordering::Acquire));
        assert!(!streams.remove(1));
    }

    #[test]
    fn build_failure_is_propagated_and_nothing_inserted() {
        let mut streams = Streams::<u32>::default();
        let cancelled = AtomicBool::new(false);
        let result = streams.start(1, &cancelled, || {
            Err(PluginError::Custom("no device".into()))
        });
        assert!(result.is_err());
        assert!(!streams.remove(1));
    }

    #[test]
    fn remove_if_ignores_a_stream_of_another_kind() {
        let mut streams = Streams::<u32>::default();
        let cancelled = AtomicBool::new(false);
        streams.start(1, &cancelled, || Ok(7)).unwrap();
        assert!(!streams.remove_if(1, |value| *value == 8));
        assert!(streams.remove_if(1, |value| *value == 7));
        assert!(!streams.remove_if(1, |_| true));
    }

    #[test]
    fn a_control_thread_is_recreated_after_shutdown() {
        let first = control().unwrap();
        shutdown().unwrap();
        let second = control().unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        // The fresh queue accepts commands: Stop of an unknown id is a no-op ack.
        stop_capture(u64::MAX).unwrap();
        shutdown().unwrap();
    }
}

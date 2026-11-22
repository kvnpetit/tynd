//! Drives `scap::Capturer` from a background thread, converts each
//! BGRA frame through the chosen encoder, and writes samples to disk.
//! `Codec::Mjpeg` → always available; `Codec::H264` / `Hevc` requires
//! `--features gpu-encode`.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use tynd_plugin_sdk::{join_with_timeout, JoinOutcome, PluginError, PluginResult};

use crate::{Codec, MonitorInfo, RecordingOpts, Sources, WindowInfo};

#[cfg(feature = "gpu-encode")]
#[path = "h264_backend.rs"]
mod h264_backend;
#[path = "mjpeg_backend.rs"]
mod mjpeg_backend;

/// Owned by `Recorder` so `Drop` stops + joins the worker.
pub(super) struct EncoderThread {
    pub(super) running: Arc<AtomicBool>,
    pub(super) paused: Arc<AtomicBool>,
    pub(super) join: Mutex<Option<JoinHandle<Result<(), String>>>>,
    /// Capture thread. `scap` cannot interrupt a blocked `get_next_frame`, so
    /// it may outlive `stop()` on a static screen; `stop()` reaps it with a
    /// bounded wait instead of ignoring it.
    capture: Mutex<Option<JoinHandle<()>>>,
    terminal: Arc<Mutex<Option<Result<(), String>>>>,
}

/// Outcome of waiting for the next captured frame.
pub(super) enum NextFrame {
    Frame(scap::frame::Frame),
    /// No frame within the timeout (static screen). The caller re-checks its
    /// stop flag and polls again.
    Idle,
    /// The capture source ended or failed.
    Closed(String),
}

/// Receiving end of the capture thread. `scap`'s `get_next_frame` has no
/// timeout and cannot be interrupted, so the capturer lives in its own thread
/// and the encoder polls this channel instead; that keeps `stop()` prompt (and
/// the output flushed) even when the screen is static and no frame arrives.
pub(super) struct FrameSource {
    rx: mpsc::Receiver<Result<scap::frame::Frame, String>>,
}

impl FrameSource {
    pub(super) fn next_frame(&self, timeout: Duration) -> NextFrame {
        match self.rx.recv_timeout(timeout) {
            Ok(Ok(frame)) => NextFrame::Frame(frame),
            Ok(Err(error)) => NextFrame::Closed(error),
            Err(mpsc::RecvTimeoutError::Timeout) => NextFrame::Idle,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                NextFrame::Closed("capture thread ended".into())
            },
        }
    }
}

/// How long the encoder waits for a frame before re-checking `running`.
pub(super) const FRAME_POLL: Duration = Duration::from_millis(100);
/// Frames buffered between the capture and encoder threads; extra frames are
/// dropped instead of growing memory behind a slow encoder.
const FRAME_QUEUE: usize = 4;
/// How long `stop()` waits for the capture thread after the encoder finished.
const CAPTURE_REAP: Duration = Duration::from_millis(500);

pub(super) fn spawn_encoder_thread<Worker>(
    thread_name: &'static str,
    encoder_name: &'static str,
    opts: &RecordingOpts,
    worker: Worker,
) -> PluginResult<EncoderThread>
where
    Worker: FnOnce(FrameSource, Arc<AtomicBool>, Arc<AtomicBool>) -> Result<(), String>
        + Send
        + 'static,
{
    let running = Arc::new(AtomicBool::new(true));
    let paused = Arc::new(AtomicBool::new(false));
    let worker_running = Arc::clone(&running);
    let worker_paused = Arc::clone(&paused);
    let capture_running = Arc::clone(&running);
    let worker_opts = opts.clone();
    let terminal = Arc::new(Mutex::new(None));
    let worker_terminal = Arc::clone(&terminal);
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let (frame_tx, frame_rx) = mpsc::sync_channel(FRAME_QUEUE);

    // The capturer is built and driven on this thread (Windows capture
    // handles are thread-affine). After `running` flips it stops at the next
    // delivered frame; on a static screen that can be later than `stop()`
    // because `scap` offers no way to interrupt a blocked `get_next_frame`.
    // `EncoderThread::stop` keeps this handle and reports a late exit.
    let capture = std::thread::Builder::new()
        .name(format!("{thread_name}-capture"))
        .spawn(move || {
            let mut capturer = match build_capturer(&worker_opts) {
                Ok(capturer) => capturer,
                Err(error) => {
                    let _ = startup_tx.send(Err(error.to_string()));
                    return;
                },
            };
            capturer.start_capture();
            if startup_tx.send(Ok(())).is_err() {
                capturer.stop_capture();
                return;
            }
            while capture_running.load(Ordering::Acquire) {
                match capturer.get_next_frame() {
                    Ok(frame) => match frame_tx.try_send(Ok(frame)) {
                        Ok(()) | Err(mpsc::TrySendError::Full(_)) => {},
                        Err(mpsc::TrySendError::Disconnected(_)) => break,
                    },
                    Err(error) => {
                        let _ = frame_tx.send(Err(format!("capture frame: {error}")));
                        break;
                    },
                }
            }
            capturer.stop_capture();
        })
        .map_err(|error| {
            PluginError::Custom(format!("screen-rec: spawn {encoder_name} capture: {error}"))
        })?;

    match startup_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => {},
        Ok(Err(error)) => {
            let _ = capture.join();
            return Err(PluginError::Custom(error));
        },
        Err(error) => {
            running.store(false, Ordering::Release);
            return Err(PluginError::Custom(format!(
                "screen-rec: capturer startup timed out: {error}"
            )));
        },
    }

    let join = std::thread::Builder::new()
        .name(thread_name.into())
        .spawn(move || {
            let result = worker(FrameSource { rx: frame_rx }, worker_running, worker_paused);
            *worker_terminal.lock() = Some(result.clone());
            result
        })
        .map_err(|error| {
            running.store(false, Ordering::Release);
            PluginError::Custom(format!("screen-rec: spawn {encoder_name} encoder: {error}"))
        })?;

    Ok(EncoderThread {
        running,
        paused,
        join: Mutex::new(Some(join)),
        capture: Mutex::new(Some(capture)),
        terminal,
    })
}

/// Preserve the primary capture/encode error while also surfacing a failed
/// flush/trailer. Cleanup must run on both success and failure paths.
pub(super) fn merge_worker_results(
    work: Result<(), String>,
    cleanup: Result<(), String>,
) -> Result<(), String> {
    match (work, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error}; encoder cleanup also failed: {cleanup_error}"
        )),
    }
}

impl EncoderThread {
    fn stop(&self) -> PluginResult<()> {
        self.running.store(false, Ordering::Release);
        let Some(handle) = self.join.lock().take() else {
            return self
                .terminal
                .lock()
                .take()
                .unwrap_or(Ok(()))
                .map_err(PluginError::Custom);
        };

        let worker_result = match join_with_timeout(handle, Duration::from_secs(5)) {
            JoinOutcome::Finished(result) => result,
            JoinOutcome::Panicked => {
                self.reap_capture();
                return Err(PluginError::Custom(
                    "screen-rec: capture worker panicked".to_string(),
                ));
            },
            JoinOutcome::TimedOut(handle) => {
                // Keep the handle so a later `stop` can still join it.
                *self.join.lock() = Some(handle);
                return Err(PluginError::Custom(
                    "screen-rec: capture worker did not stop within 5s".to_string(),
                ));
            },
        };
        self.terminal.lock().take();
        self.reap_capture();
        worker_result.map_err(PluginError::Custom)
    }

    /// Wait briefly for the capture thread. It exits at its next delivered
    /// frame, so on a static screen it can still hold the OS capture session
    /// (Windows capture border, macOS indicator) open: say so and detach it
    /// rather than block `stop()` or hide the leak.
    fn reap_capture(&self) {
        let Some(handle) = self.capture.lock().take() else {
            return;
        };
        match join_with_timeout(handle, CAPTURE_REAP) {
            JoinOutcome::Finished(()) => {},
            JoinOutcome::Panicked => {
                tracing::error!(target: "tynd::screen_rec", "capture thread panicked");
            },
            JoinOutcome::TimedOut(_) => {
                tracing::warn!(
                    target: "tynd::screen_rec",
                    "capture thread still waiting for a frame after stop; the OS capture session closes when the screen next changes"
                );
            },
        }
    }

    /// Give up on the worker: drop its join handle (detaching the thread) so
    /// a later `stop` - notably `Recorder::drop` - does not wait again.
    fn detach(&self) {
        drop(self.join.lock().take());
        drop(self.capture.lock().take());
    }

    fn failure(&self) -> Option<String> {
        match self.terminal.lock().as_ref() {
            Some(Err(error)) => return Some(error.clone()),
            Some(Ok(())) => return None,
            None => {},
        }
        let worker_finished = self
            .join
            .lock()
            .as_ref()
            .is_some_and(JoinHandle::is_finished);
        worker_finished.then(|| "screen-rec: capture worker panicked".to_string())
    }
}

/// Encoder thread reads frames from `Capturer::get_next_frame()` and
/// writes encoded samples to the output until `running` flips false.
pub(crate) struct Recorder {
    pub(crate) started_at: Instant,
    /// Wall-clock epoch ms snapshot taken at `start()` — `list()`
    /// reads this back instead of subtracting an `Instant` from
    /// `SystemTime::now()` (mixed-clock drift).
    pub(crate) started_at_ms: i64,
    pub(crate) output: String,
    pub(crate) paused_for: Duration,
    pub(crate) paused_at: Option<Instant>,
    encoder: EncoderThread,
}

impl Recorder {
    /// Frames received while paused are dropped, not written.
    pub(crate) fn set_paused(&self, paused: bool) {
        self.encoder.paused.store(paused, Ordering::SeqCst);
    }
}

pub(crate) fn stop(recorder: &Recorder) -> PluginResult<()> {
    recorder.encoder.stop()
}

/// Abandon a worker that did not stop in time (shutdown path), so dropping
/// the `Recorder` does not wait for it a second time.
pub(crate) fn detach(recorder: &Recorder) {
    recorder.encoder.detach();
}

pub(crate) fn failure(recorder: &Recorder) -> Option<String> {
    recorder.encoder.failure()
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Err(error) = self.encoder.stop() {
            tracing::error!(target: "tynd::screen_rec", "recording shutdown failed: {error}");
        }
    }
}

pub(crate) fn start(opts: RecordingOpts) -> PluginResult<Recorder> {
    if !scap::has_permission() {
        return Err(PluginError::Custom(
            "screen-rec: screen-capture permission denied — call `requestPermission()` first \
             (macOS) or grant access via Settings → Privacy → Screen Recording"
                .into(),
        ));
    }

    // Validate path *before* starting the capturer so a missing
    // parent surfaces synchronously, not in the encoder thread.
    let path = PathBuf::from(&opts.output);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(PluginError::Custom(format!(
                "screen-rec: output directory {} does not exist",
                parent.display()
            )));
        }
    }

    let encoder = match opts.codec {
        Codec::Mjpeg => mjpeg_backend::spawn(&opts)?,
        Codec::H264 | Codec::Hevc => spawn_h264_backend(&opts)?,
    };

    let started_at_ms = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis()),
    )
    .unwrap_or(0);
    Ok(Recorder {
        started_at: Instant::now(),
        started_at_ms,
        output: opts.output,
        paused_for: Duration::ZERO,
        paused_at: None,
        encoder,
    })
}

/// Wrapper isolating `cfg(feature = "gpu-encode")` from `start()`.
#[cfg(feature = "gpu-encode")]
fn spawn_h264_backend(opts: &RecordingOpts) -> PluginResult<EncoderThread> {
    h264_backend::spawn(opts)
}

#[cfg(not(feature = "gpu-encode"))]
fn spawn_h264_backend(_opts: &RecordingOpts) -> PluginResult<EncoderThread> {
    Err(PluginError::Custom(
        "screen-rec: H.264 / HEVC encoding requires the `gpu-encode` Cargo feature. Rebuild \
         with:\n  cd plugins/standalone/screen-rec/native && cargo build --features gpu-encode\nOr use \
         codec: 'mjpeg' (always available)."
            .into(),
    ))
}

fn enumerate_targets() -> PluginResult<Vec<scap::Target>> {
    std::panic::catch_unwind(scap::get_all_targets).map_err(|_| {
        PluginError::Custom("screen-rec: scap failed while enumerating capture sources".into())
    })
}

fn target_dimensions(target: &scap::Target) -> (u32, u32) {
    let options = scap::capturer::Options {
        target: Some(target.clone()),
        output_resolution: scap::capturer::Resolution::Captured,
        ..Default::default()
    };
    std::panic::catch_unwind(|| scap::capturer::get_output_frame_size(&options)).map_or_else(
        |_| {
            tracing::warn!(
                target: "tynd::screen_rec",
                "scap could not resolve capture-source dimensions"
            );
            (0, 0)
        },
        |[width, height]| (width, height),
    )
}

#[cfg(target_os = "windows")]
#[allow(clippy::cast_possible_truncation)] // Must match scap's HMONITOR-to-display-id truncation.
fn primary_display_id() -> Option<u32> {
    windows_capture::monitor::Monitor::primary()
        .ok()
        // Match scap 0.0.8's public display-id conversion exactly.
        .map(|monitor| monitor.as_raw_hmonitor() as usize as u32)
}

pub(crate) fn list_sources() -> PluginResult<Sources> {
    let targets = enumerate_targets()?;
    #[cfg(target_os = "windows")]
    let primary_display_id = primary_display_id();

    let mut monitors = Vec::new();
    let mut windows = Vec::new();
    for target in targets {
        match &target {
            scap::Target::Display(display) => {
                let (width, height) = target_dimensions(&target);
                let primary = {
                    #[cfg(target_os = "windows")]
                    {
                        primary_display_id == Some(display.id)
                    }
                    #[cfg(target_os = "macos")]
                    {
                        display.raw_handle.is_main()
                    }
                    #[cfg(target_os = "linux")]
                    {
                        false
                    }
                };
                monitors.push(MonitorInfo {
                    id: display.id,
                    name: display.title.clone(),
                    width,
                    height,
                    primary,
                });
            },
            scap::Target::Window(window) => windows.push(WindowInfo {
                id: window.id,
                title: window.title.clone(),
                // scap 0.0.8 does not expose the owning application.
                app_name: String::new(),
            }),
        }
    }
    Ok(Sources { monitors, windows })
}

fn display_target(id: u32) -> PluginResult<scap::Target> {
    enumerate_targets()?
        .into_iter()
        .find(|target| matches!(target, scap::Target::Display(display) if display.id == id))
        .ok_or_else(|| PluginError::InvalidArg {
            key: "opts.monitor".into(),
            detail: format!(
                "screen-rec.start: monitor {id} is not available; use an id returned by \
                 listSources() (omit monitor on Linux when the capture portal selects it)"
            ),
        })
}

pub(crate) fn has_permission() -> bool {
    scap::has_permission()
}

pub(crate) fn request_permission() -> bool {
    scap::request_permission()
}

/// Whether this build can encode `codec`; used for early-exit errors.
pub(crate) fn codec_supported(codec: Codec) -> bool {
    match codec {
        Codec::Mjpeg => true,
        Codec::H264 | Codec::Hevc => cfg!(feature = "gpu-encode"),
    }
}

pub(super) fn build_capturer(opts: &RecordingOpts) -> PluginResult<scap::capturer::Capturer> {
    let target = opts.monitor.map(display_target).transpose()?;
    let crop_area = opts.region.map(|region| scap::capturer::Area {
        origin: scap::capturer::Point {
            x: f64::from(region.x),
            y: f64::from(region.y),
        },
        size: scap::capturer::Size {
            width: f64::from(region.width),
            height: f64::from(region.height),
        },
    });
    scap::capturer::Capturer::build(scap::capturer::Options {
        fps: opts.fps,
        show_cursor: true,
        show_highlight: false,
        target,
        crop_area,
        excluded_targets: None,
        output_type: scap::frame::FrameType::BGRAFrame,
        output_resolution: scap::capturer::Resolution::Captured,
    })
    .map_err(|error| PluginError::Custom(format!("screen-rec: build capturer: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detach_makes_a_later_stop_return_without_waiting() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let join = std::thread::spawn(move || {
            let _ = release_rx.recv();
            Ok(())
        });
        let encoder = EncoderThread {
            running: Arc::new(AtomicBool::new(true)),
            paused: Arc::new(AtomicBool::new(false)),
            join: Mutex::new(Some(join)),
            capture: Mutex::new(None),
            terminal: Arc::new(Mutex::new(None)),
        };
        encoder.detach();
        let started = Instant::now();
        assert!(encoder.stop().is_ok());
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(release_tx);
    }

    #[test]
    fn stop_does_not_wait_past_the_bound_for_a_blocked_capture_thread() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let capture = std::thread::spawn(move || {
            let _ = release_rx.recv();
        });
        let encoder = EncoderThread {
            running: Arc::new(AtomicBool::new(true)),
            paused: Arc::new(AtomicBool::new(false)),
            join: Mutex::new(Some(std::thread::spawn(|| Ok(())))),
            capture: Mutex::new(Some(capture)),
            terminal: Arc::new(Mutex::new(None)),
        };
        let started = Instant::now();
        assert!(encoder.stop().is_ok());
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(encoder.capture.lock().is_none());
        drop(release_tx);
    }

    #[test]
    fn idle_source_times_out_instead_of_blocking_and_reports_closure() {
        let (tx, rx) = mpsc::sync_channel(1);
        let source = FrameSource { rx };
        let started = Instant::now();
        assert!(matches!(
            source.next_frame(Duration::from_millis(20)),
            NextFrame::Idle
        ));
        assert!(started.elapsed() < Duration::from_secs(1));
        tx.send(Err("boom".into())).unwrap();
        assert!(matches!(
            source.next_frame(Duration::from_millis(20)),
            NextFrame::Closed(message) if message == "boom"
        ));
        drop(tx);
        assert!(matches!(
            source.next_frame(Duration::from_millis(20)),
            NextFrame::Closed(_)
        ));
    }
}

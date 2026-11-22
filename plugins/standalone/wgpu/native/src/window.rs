//! Native `tao::Window` + `wgpu::Surface` lifecycle.
//!
//! ## Threading model
//!
//! `tao::Window` (and its underlying `winit`) wants the `EventLoop` on the
//! **process main thread**. The Tynd host owns that thread for its own
//! primary window and the plugin SDK does not yet hand plugins a handle on
//! it, so this plugin spawns a dedicated event-loop thread per
//! `createSurface` call and creates the `EventLoop` + `Window` **inside**
//! that thread (the EventLoop is `!Send` so it can't be moved across
//! thread boundaries):
//!
//! - **Windows** : works - Win32 accepts event loops on non-main threads (`with_any_thread`).
//! - **macOS** : unsupported - Cocoa's run loop hard-requires the main thread.
//! - **Linux** : unsupported - tao is GTK there, GTK is not thread-safe and the host already runs
//!   its own GTK loop on the main thread, so a second loop on a plugin thread is undefined
//!   behaviour. Supporting it needs a main-thread proxy (`PluginCx::with_event_loop_proxy`).
//!
//! On unsupported platforms `createSurface` returns an explanatory error and
//! `available()` reports `available: false`, so callers can fall back to a
//! webview canvas.
//!
//! The loop is driven with `run_return`, never `run`: `run` ends in
//! `std::process::exit`, which would take the whole host down as soon as one
//! surface window closes.
//!
//! ## Init handshake
//!
//! Because everything lives inside the spawned thread, `create()` opens a
//! `crossbeam-channel` and waits on the worker to send back the wgpu
//! adapter info (vendor / device / driver) - needed by the JS-facing
//! `SurfaceHandle` returned to the caller. Resize requests flow back to
//! the worker through a second channel, picked up at the next event-loop
//! tick. Submit / close use the same pattern.
//!
//! ## Error handling
//!
//! The wgpu default for uncaptured errors is to panic. A panic inside the
//! window procedure would cross FFI and abort the host, so every device gets an
//! `on_uncaptured_error` handler that emits `wgpu:error` instead, and every size
//! is validated / clamped against the device limits before it reaches `configure`.

#![cfg(feature = "native")]

use std::{
    cell::RefCell,
    sync::{atomic::AtomicBool, Arc},
    thread::JoinHandle,
    time::Duration,
};

use crossbeam_channel::{bounded, Sender};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use serde_json::json;
// Only Windows can build an `EventLoop` off the main thread (`with_any_thread`);
// see the module docs and the bail arm at the top of `SurfaceInner::create`.
#[cfg(windows)]
use tao::platform::windows::EventLoopBuilderExtWindows;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::run_return::EventLoopExtRunReturn,
    window::WindowBuilder,
};
use tynd_plugin_sdk::{
    join_with_timeout, sanitize_event_string, EmitFn, PluginError, PluginResult,
};
use wgpu::{
    CompositeAlphaMode, Device, Instance, InstanceDescriptor, Queue, RequestAdapterOptions,
    Surface, SurfaceConfiguration, TextureFormat, TextureUsages,
};

use super::{
    clamp_surface_size, validate_surface_size, AdapterInfo, Backend, PowerPreference, PresentMode,
    SurfaceFormat, SurfaceOpts, MAX_SURFACE_DIMENSION,
};

/// How long `Drop` waits for an event-loop thread to wind down before detaching it.
const JOIN_DEADLINE: Duration = Duration::from_secs(5);

/// Minimum delay before the same render error is emitted again.
#[cfg(any(windows, test))]
const ERROR_REPEAT_INTERVAL: Duration = Duration::from_secs(1);

/// Rate-limits `wgpu:error` for the per-frame render path: a persistent error
/// would otherwise emit at the display refresh rate. A different message is
/// always let through; an identical one at most once per interval.
#[cfg(any(windows, test))]
#[derive(Default)]
struct ErrorThrottle {
    last: Option<(String, std::time::Instant)>,
}

#[cfg(any(windows, test))]
impl ErrorThrottle {
    fn allow(&mut self, message: &str, now: std::time::Instant) -> bool {
        if let Some((last, at)) = &self.last {
            if last == message && now.saturating_duration_since(*at) < ERROR_REPEAT_INTERVAL {
                return false;
            }
        }
        self.last = Some((message.to_owned(), now));
        true
    }
}

/// Live state for one native surface. Owned by the registry; dropping it
/// signals the per-window event-loop thread to exit.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct SurfaceInner {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Largest accepted side length: the device's `max_texture_dimension_2d`
    /// capped by `MAX_SURFACE_DIMENSION`.
    max_dimension: u32,
    pub(crate) backend_name: String,
    pub(crate) adapter: AdapterInfo,
    /// Channel used to send resize commands to the worker thread (which
    /// owns the EventLoop + Window). The worker picks them up on the
    /// next event-loop tick.
    resize_tx: Sender<(u32, u32)>,
    /// Shutdown flag - flipping it makes the event-loop thread exit.
    shutdown: Arc<AtomicBool>,
    /// Handle on the per-window event-loop worker. `Drop` joins it (with a
    /// deadline) so the OS window / surface / wgpu device tear down before the
    /// `SurfaceInner` returns to the registry's free list. Wrapped in `Option`
    /// so `Drop` can `take()` it.
    worker: Option<JoinHandle<()>>,
}

/// One-shot init payload the worker thread sends back to the caller of
/// `create()` after wgpu adapter selection finishes.
#[cfg(windows)]
struct InitOk {
    max_dimension: u32,
    backend_name: String,
    adapter: AdapterInfo,
}

impl SurfaceInner {
    pub(crate) fn create(surface_id: u64, opts: SurfaceOpts, emit: EmitFn) -> anyhow::Result<Self> {
        // Bail early where the threading model cannot work - see module docs. We surface the
        // diagnostic at create time so a Three.js adapter can fall back to the webview canvas
        // instead of waiting on a hung promise.
        #[cfg(not(windows))]
        {
            let _ = (surface_id, opts, emit);
            let reason = super::platform_unsupported_reason().unwrap_or("unsupported platform");
            anyhow::bail!("wgpu plugin: {reason}");
        }

        #[cfg(windows)]
        {
            let SurfaceOpts {
                title,
                width,
                height,
                backend,
                power_preference,
                present_mode,
                format,
            } = opts;
            let title = title.unwrap_or_else(|| "Tynd wgpu surface".into());

            let (init_tx, init_rx) = bounded::<anyhow::Result<InitOk>>(1);
            let (resize_tx, resize_rx) = bounded::<(u32, u32)>(8);
            let shutdown = Arc::new(AtomicBool::new(false));
            let drive_shutdown = Arc::clone(&shutdown);

            let worker = std::thread::Builder::new()
                .name("tynd-wgpu-eventloop".into())
                .spawn(move || {
                    let init_outcome = run_worker(WorkerOpts {
                        surface_id,
                        title,
                        width,
                        height,
                        backend,
                        power_preference,
                        present_mode,
                        format,
                        emit,
                        shutdown: drive_shutdown,
                        init_tx: init_tx.clone(),
                        resize_rx,
                    });
                    // If init failed *before* it could send anything,
                    // surface the error to `create()` instead of hanging.
                    if let Err(err) = init_outcome {
                        let _ = init_tx.send(Err(err));
                    }
                })?;

            // Wait for the worker to finish wgpu init. The first message
            // is either Ok(adapter info) or Err(init failure).
            let init = match init_rx.recv_timeout(Duration::from_secs(30)) {
                Ok(result) => result?,
                Err(error) => {
                    shutdown.store(true, std::sync::atomic::Ordering::Release);
                    return Err(anyhow::anyhow!(
                        "wgpu surface initialization timed out or disconnected: {error}"
                    ));
                },
            };

            Ok(Self {
                width,
                height,
                max_dimension: init.max_dimension,
                backend_name: init.backend_name,
                adapter: init.adapter,
                resize_tx,
                shutdown,
                worker: Some(worker),
            })
        }
    }

    pub(crate) fn resize(&mut self, w: u32, h: u32) -> PluginResult<()> {
        validate_surface_size("resize", w, h, self.max_dimension)?;
        // Best-effort: if the worker has already exited (window closed by the user), the send
        // fails and we surface a clear error.
        self.resize_tx
            .send_timeout((w, h), Duration::from_millis(250))
            .map_err(|error| {
                PluginError::Custom(format!("wgpu.resize: surface worker unavailable: {error}"))
            })?;
        self.width = w;
        self.height = h;
        Ok(())
    }

    /// False once the event-loop thread has ended (window closed, or the worker failed).
    pub(crate) fn is_alive(&self) -> bool {
        self.worker.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// Ask the event-loop thread to exit without waiting for it.
    pub(crate) fn request_stop(&self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for SurfaceInner {
    fn drop(&mut self) {
        // 1. Signal shutdown - the worker checks this flag on every event-loop tick and leaves
        //    `run_return` via `ControlFlow::Exit`.
        self.request_stop();
        // 2. Drop the original resize sender so the worker's `try_recv` sees `Disconnected`.
        //    `&mut self` inside `Drop` can't move the field out, so swap it for a throwaway sender.
        let (dummy_tx, _dummy_rx) = bounded::<(u32, u32)>(0);
        drop(std::mem::replace(&mut self.resize_tx, dummy_tx));
        // 3. Join the worker with a deadline so the OS window / surface / device tear down at
        //    registry eviction time. A wedged driver must not hang the caller (or host exit):
        //    past the deadline the thread is detached.
        if let Some(handle) = self.worker.take() {
            // `TimedOut` drops the returned handle, detaching the thread.
            let _ = join_with_timeout(handle, JOIN_DEADLINE);
        }
    }
}

#[cfg(windows)]
struct WorkerOpts {
    /// Surface registry id, threaded into every emitted event so the JS
    /// side can filter listeners per-surface. Required for multi-surface.
    surface_id: u64,
    title: String,
    width: u32,
    height: u32,
    backend: Backend,
    power_preference: PowerPreference,
    present_mode: PresentMode,
    format: SurfaceFormat,
    emit: EmitFn,
    shutdown: Arc<AtomicBool>,
    init_tx: Sender<anyhow::Result<InitOk>>,
    resize_rx: crossbeam_channel::Receiver<(u32, u32)>,
}

/// Worker entry point - creates the EventLoop + Window + wgpu init *on
/// this thread* (everything tao touches is `!Send`), then drives the
/// event loop until the shutdown flag flips or the window is closed.
#[cfg(windows)]
fn run_worker(opts: WorkerOpts) -> anyhow::Result<()> {
    let WorkerOpts {
        surface_id,
        title,
        width,
        height,
        backend,
        power_preference,
        present_mode,
        format,
        emit,
        shutdown,
        init_tx,
        resize_rx,
    } = opts;

    // 1. EventLoop on this dedicated thread.
    let mut event_loop = EventLoopBuilder::<()>::with_user_event()
        .with_any_thread(true)
        .build();
    let window = WindowBuilder::new()
        .with_title(&title)
        .with_inner_size(LogicalSize::new(width, height))
        .build(&event_loop)?;

    // 2. wgpu Instance + Surface from the raw window handle.
    let mut instance_desc = InstanceDescriptor::new_without_display_handle();
    instance_desc.backends = backend_to_wgpu(backend);
    let instance = Instance::new(instance_desc);
    let raw_window = window.window_handle()?.as_raw();
    let raw_display = window.display_handle()?.as_raw();
    // SAFETY: the window outlives the surface - both live on this thread, and `state` (which
    // owns the surface) is declared after `window`, so it drops first.
    let surface = unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(raw_display),
            raw_window_handle: raw_window,
        })
    }?;

    // 3. Adapter + device/queue.
    let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
        power_preference: power_preference_to_wgpu(power_preference),
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(|e| anyhow::anyhow!("no compatible wgpu adapter for the requested backend: {e}"))?;
    let info = adapter.get_info();
    let backend_name = format!("{:?}", info.backend).to_lowercase();
    let adapter_info = AdapterInfo {
        vendor: format!("0x{:04x}", info.vendor),
        device: info.name.clone(),
        driver: info.driver_info.clone(),
    };

    // Ask for the adapter's real texture-size ceiling instead of the 8192 baseline, so the
    // size validation below matches what the hardware can do.
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
        ..wgpu::DeviceDescriptor::default()
    }))?;
    install_error_handlers(&device, surface_id, &emit);
    let max_dimension = device
        .limits()
        .max_texture_dimension_2d
        .min(MAX_SURFACE_DIMENSION);
    validate_surface_size("createSurface", width, height, max_dimension)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // 4. Surface configuration (format / present mode / size).
    let config = configure_surface(
        &surface,
        &adapter,
        &device,
        (width, height),
        (format, present_mode),
    )?;

    // Tell `create()` we're up. Failure means the caller went away (its
    // recv side was dropped) - clean exit.
    if init_tx
        .send(Ok(InitOk {
            max_dimension,
            backend_name,
            adapter: adapter_info,
        }))
        .is_err()
    {
        return Ok(());
    }

    // Per-frame render state. `RefCell` (single-thread access from the
    // event-loop closure) replaces a `Mutex` to drop ~60 Hz lock overhead.
    let state = RefCell::new(SurfaceState {
        surface,
        device,
        queue,
        config,
        max_dimension,
    });

    // 5. Drive the event loop until shutdown or window close.
    drive_event_loop(
        &mut event_loop,
        &state,
        &LoopCtx {
            surface_id,
            emit: &emit,
            shutdown: &shutdown,
            resize_rx: &resize_rx,
        },
    );
    // `state` (surface, device, queue) drops here, then `window`, then `event_loop`.
    Ok(())
}

/// Pick format / present mode / alpha from the surface capabilities and apply the first
/// `configure`.
#[cfg(windows)]
fn configure_surface(
    surface: &Surface<'_>,
    adapter: &wgpu::Adapter,
    device: &Device,
    (width, height): (u32, u32),
    (format, present_mode): (SurfaceFormat, PresentMode),
) -> anyhow::Result<SurfaceConfiguration> {
    let caps = surface.get_capabilities(adapter);
    let alpha = caps
        .alpha_modes
        .iter()
        .copied()
        .find(|m| matches!(m, CompositeAlphaMode::Opaque))
        .unwrap_or(CompositeAlphaMode::Auto);
    let config = SurfaceConfiguration {
        usage: TextureUsages::RENDER_ATTACHMENT,
        format: pick_surface_format(&caps, format)?,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width,
        height,
        present_mode: present_mode_to_wgpu(present_mode, &caps),
        desired_maximum_frame_latency: 2,
        alpha_mode: alpha,
        view_formats: Vec::new(),
    };
    surface.configure(device, &config);
    Ok(config)
}

#[cfg(windows)]
struct LoopCtx<'a> {
    surface_id: u64,
    emit: &'a EmitFn,
    shutdown: &'a AtomicBool,
    resize_rx: &'a crossbeam_channel::Receiver<(u32, u32)>,
}

/// Run the tao loop until shutdown or window close. `run_return` hands control back to us on
/// `ControlFlow::Exit`; `run` would call `process::exit` and kill the host.
#[cfg(windows)]
fn drive_event_loop(
    event_loop: &mut tao::event_loop::EventLoop<()>,
    state: &RefCell<SurfaceState>,
    ctx: &LoopCtx<'_>,
) {
    let LoopCtx {
        surface_id,
        emit,
        shutdown,
        resize_rx,
    } = *ctx;
    let mut frame_count: u64 = 0;
    let mut error_throttle = ErrorThrottle::default();
    event_loop.run_return(|event, _target, control_flow| {
        if shutdown.load(std::sync::atomic::Ordering::Acquire) {
            *control_flow = ControlFlow::Exit;
            return;
        }
        *control_flow = ControlFlow::Poll;

        // Drain pending resize requests from the JS side.
        while let Ok((w, h)) = resize_rx.try_recv() {
            state.borrow_mut().reconfigure(w, h);
        }

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                emit(
                    "wgpu:close",
                    json!({ "id": surface_id, "frame": frame_count }),
                );
                *control_flow = ControlFlow::Exit;
            },
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                state.borrow_mut().reconfigure(size.width, size.height);
                emit(
                    "wgpu:resize",
                    json!({ "id": surface_id, "width": size.width, "height": size.height }),
                );
            },
            Event::MainEventsCleared => {
                // Per-frame tick: clear-only render pass. Clears keep the
                // swap chain alive + give Three.js a known-good background.
                if let Err(err) = render_one_frame(state) {
                    let message = sanitize_event_string(&err.to_string());
                    if error_throttle.allow(&message, std::time::Instant::now()) {
                        emit(
                            "wgpu:error",
                            json!({ "id": surface_id, "message": message }),
                        );
                    }
                }
                frame_count += 1;
                // Per-frame `wgpu:frame` emit gated behind `debug-frame-emit`
                // so 60 Hz JSON ser+IPC overhead stays out of release builds.
                #[cfg(feature = "debug-frame-emit")]
                emit(
                    "wgpu:frame",
                    json!({ "id": surface_id, "frame": frame_count }),
                );
                #[cfg(not(feature = "debug-frame-emit"))]
                let _ = frame_count;
            },
            _ => {},
        }
    });
}

/// Route wgpu validation / out-of-memory errors and device loss to `wgpu:error` events.
/// Without this the default handler panics, which inside the window procedure aborts the host.
#[cfg(windows)]
fn install_error_handlers(device: &Device, surface_id: u64, emit: &EmitFn) {
    let on_error = Arc::clone(emit);
    // Validation errors can repeat on every submit; throttle identical ones.
    let throttle = std::sync::Mutex::new(ErrorThrottle::default());
    device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
        let message = sanitize_event_string(&error.to_string());
        let allowed = throttle
            .lock()
            .map_or(true, |mut t| t.allow(&message, std::time::Instant::now()));
        if allowed {
            on_error(
                "wgpu:error",
                json!({ "id": surface_id, "message": message }),
            );
        }
    }));
    let on_lost = Arc::clone(emit);
    device.set_device_lost_callback(move |reason, message| {
        on_lost(
            "wgpu:error",
            json!({
                "id": surface_id,
                "message": sanitize_event_string(&format!("device lost ({reason:?}): {message}")),
            }),
        );
    });
}

#[cfg(windows)]
struct SurfaceState {
    /// `Surface<'static>` here is a half-truth - the underlying
    /// `wgpu::Surface` borrows the OS window handle that the worker
    /// thread owns. We launder the lifetime to `'static` because that
    /// thread keeps the window alive for the entire surface lifetime
    /// and **never** moves either across threads. Sharing this struct
    /// across threads (or letting the window outlive the surface) is
    /// undefined behaviour: the surface will dereference a freed
    /// `RawWindowHandle`. Keep `SurfaceState` strictly thread-local.
    surface: Surface<'static>,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    /// Device-limit-derived ceiling applied to every reconfigure.
    max_dimension: u32,
}

#[cfg(windows)]
impl SurfaceState {
    /// Reconfigure the swap chain for a new size, clamped into `[1, max_dimension]`
    /// so an OS-reported or JS-requested size can never trip wgpu validation.
    fn reconfigure(&mut self, width: u32, height: u32) {
        let (w, h) = clamp_surface_size(width, height, self.max_dimension);
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
    }
}

#[cfg(windows)]
fn render_one_frame(state: &RefCell<SurfaceState>) -> anyhow::Result<()> {
    let s = state.borrow();
    let mut needs_reconfigure = false;
    let frame = match s.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame) => frame,
        // Still presentable, but the swap chain no longer matches the window: draw this frame,
        // present it, then rebuild the swap chain (wgpu rejects `configure` while a frame from
        // the surface is still acquired).
        wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
            needs_reconfigure = true;
            frame
        },
        // Swap chain went stale (resize race, display change): rebuild it and retry next tick.
        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
            s.surface.configure(&s.device, &s.config);
            return Ok(());
        },
        // Transient (occluded / minimized window, timeout): skip the frame quietly.
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
            return Ok(());
        },
        // Validation failure inside acquisition: already reported through the
        // uncaptured-error handler; retry next tick.
        wgpu::CurrentSurfaceTexture::Validation => return Ok(()),
    };
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = s
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("tynd-wgpu-frame"),
        });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("tynd-wgpu-clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.012,
                        g: 0.012,
                        b: 0.020,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    s.queue.submit(Some(encoder.finish()));
    s.queue.present(frame);
    if needs_reconfigure {
        s.surface.configure(&s.device, &s.config);
    }
    Ok(())
}

// Enum mappers

#[cfg(windows)]
fn backend_to_wgpu(b: Backend) -> wgpu::Backends {
    match b {
        // `PRIMARY | SECONDARY` includes `BROWSER_WEBGPU`, which is
        // irrelevant on a native window surface and only confuses the
        // adapter walk. Restrict `Auto` to the four real native
        // backends + GL.
        Backend::Auto => {
            wgpu::Backends::VULKAN
                | wgpu::Backends::METAL
                | wgpu::Backends::DX12
                | wgpu::Backends::GL
        },
        Backend::Vulkan => wgpu::Backends::VULKAN,
        Backend::Metal => wgpu::Backends::METAL,
        Backend::Dx12 => wgpu::Backends::DX12,
        Backend::Gl => wgpu::Backends::GL,
    }
}

#[cfg(windows)]
fn power_preference_to_wgpu(p: PowerPreference) -> wgpu::PowerPreference {
    match p {
        PowerPreference::HighPerformance => wgpu::PowerPreference::HighPerformance,
        PowerPreference::LowPower => wgpu::PowerPreference::LowPower,
    }
}

#[cfg(windows)]
fn present_mode_to_wgpu(p: PresentMode, caps: &wgpu::SurfaceCapabilities) -> wgpu::PresentMode {
    let want = match p {
        PresentMode::Fifo => wgpu::PresentMode::Fifo,
        PresentMode::Mailbox => wgpu::PresentMode::Mailbox,
        PresentMode::Immediate => wgpu::PresentMode::Immediate,
    };
    // Fall back to FIFO when the driver doesn't expose the requested mode.
    // FIFO is the only mode the spec guarantees on every surface.
    if caps.present_modes.contains(&want) {
        want
    } else {
        wgpu::PresentMode::Fifo
    }
}

#[cfg(windows)]
fn pick_surface_format(
    caps: &wgpu::SurfaceCapabilities,
    want: SurfaceFormat,
) -> anyhow::Result<TextureFormat> {
    let preferred = match want {
        SurfaceFormat::Rgba8unorm => TextureFormat::Rgba8Unorm,
        SurfaceFormat::Bgra8unorm => TextureFormat::Bgra8Unorm,
        SurfaceFormat::Rgba16float => TextureFormat::Rgba16Float,
    };
    if caps.formats.contains(&preferred) {
        return Ok(preferred);
    }
    // Fall back to whatever sRGB-capable format the adapter offers
    // (matches the wgpu examples convention), then any format at all.
    // A truly empty `caps.formats` happens on Vulkan-stripped builds
    // and headless / broken drivers - surface a clear error instead of
    // panicking with `caps.formats[0]`.
    if let Some(srgb) = caps.formats.iter().copied().find(TextureFormat::is_srgb) {
        return Ok(srgb);
    }
    caps.formats.first().copied().ok_or_else(|| {
        anyhow::anyhow!(
            "no surface format available - adapter exposed an empty format list (Vulkan-stripped \
             build or broken driver?)"
        )
    })
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn throttle_suppresses_identical_errors_within_the_interval() {
        let mut throttle = ErrorThrottle::default();
        let t0 = Instant::now();
        assert!(throttle.allow("lost", t0));
        assert!(!throttle.allow("lost", t0 + Duration::from_millis(16)));
        assert!(!throttle.allow("lost", t0 + Duration::from_millis(900)));
        assert!(throttle.allow("lost", t0 + ERROR_REPEAT_INTERVAL));
    }

    #[test]
    fn throttle_lets_a_different_message_through_immediately() {
        let mut throttle = ErrorThrottle::default();
        let t0 = Instant::now();
        assert!(throttle.allow("a", t0));
        assert!(throttle.allow("b", t0 + Duration::from_millis(1)));
        assert!(throttle.allow("a", t0 + Duration::from_millis(2)));
    }
}

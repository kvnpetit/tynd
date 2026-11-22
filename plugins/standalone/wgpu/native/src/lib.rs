//! Native wgpu surface plugin. Spins up an OS window (via [`tao`]) carrying
//! a [`wgpu`] surface backed by the platform graphics API (Metal / D3D12 /
//! Vulkan / GLES). Bypasses the webview compositor.
//!
//! `SurfaceOpts.backend` accepts `auto | metal | vulkan | dx12 | gl`.
//! `auto` follows wgpu's primary order (DX12 → Metal → Vulkan → GL).
//!
//! [`wgpu`]: https://crates.io/crates/wgpu
//! [`tao`]: https://crates.io/crates/tao

use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tynd_plugin_sdk::{
    tynd_plugin::{plugin, plugin_impl},
    PluginCx, PluginError, PluginResult, Registry,
};

#[cfg(feature = "native")]
pub(crate) mod renderer;
#[cfg(feature = "native")]
pub(crate) mod window;

/// Hard cap on concurrent native surfaces.
const MAX_SURFACES: usize = 32;

/// Hard upper bound for any surface dimension, checked before the device is even
/// opened. The real limit is `device.limits().max_texture_dimension_2d`, which is
/// enforced again once the device exists; this cap only keeps absurd values from
/// reaching the OS window layer.
pub(crate) const MAX_SURFACE_DIMENSION: u32 = 16_384;

/// Why native surfaces cannot be created on this OS, if they cannot.
///
/// tao wants its event loop on the process main thread, which the host owns.
/// macOS (Cocoa) enforces this hard; on Linux tao is GTK, which is not thread-safe
/// and cannot run a second loop on a plugin thread next to the host's own. Only
/// Windows can build an event loop off the main thread (`with_any_thread`).
pub(crate) const fn platform_unsupported_reason() -> Option<&'static str> {
    if cfg!(windows) {
        None
    } else if cfg!(target_os = "macos") {
        Some(
            "native wgpu surfaces are not supported on macOS yet: Cocoa requires the main thread, \
             which the host owns (needs PluginCx::with_event_loop_proxy)",
        )
    } else if cfg!(target_os = "linux") {
        Some(
            "native wgpu surfaces are not supported on Linux yet: tao uses GTK, which cannot run \
             a second event loop off the host's main thread (needs PluginCx::with_event_loop_proxy)",
        )
    } else {
        Some("native wgpu surfaces are only supported on Windows")
    }
}

/// Reject zero or oversized surface dimensions with an `InvalidArg` error.
///
/// `max` is the effective per-side limit (the device's `max_texture_dimension_2d`
/// capped by [`MAX_SURFACE_DIMENSION`]).
pub(crate) fn validate_surface_size(
    op: &str,
    width: u32,
    height: u32,
    max: u32,
) -> PluginResult<()> {
    for (key, value) in [("width", width), ("height", height)] {
        if value == 0 {
            return Err(PluginError::InvalidArg {
                key: key.into(),
                detail: format!("wgpu.{op}: {key} must be greater than 0"),
            });
        }
        if value > max {
            return Err(PluginError::InvalidArg {
                key: key.into(),
                detail: format!("wgpu.{op}: {key} {value} exceeds the maximum of {max}"),
            });
        }
    }
    Ok(())
}

/// Clamp a size reported by the OS into `[1, max]` so reconfiguring the surface
/// can never trigger a wgpu validation error.
#[cfg_attr(not(feature = "native"), allow(dead_code))]
pub(crate) fn clamp_surface_size(width: u32, height: u32, max: u32) -> (u32, u32) {
    let max = max.max(1);
    (width.clamp(1, max), height.clamp(1, max))
}

const fn default_width() -> u32 {
    1280
}
const fn default_height() -> u32 {
    720
}

/// `Auto` lets wgpu pick the platform default (DX12 / Metal / Vulkan / GLES).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Backend {
    #[default]
    Auto,
    Vulkan,
    Metal,
    Dx12,
    Gl,
}

/// Maps onto `wgpu::PowerPreference` 1:1.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PowerPreference {
    #[default]
    HighPerformance,
    LowPower,
}

/// `Fifo` (vsync, default). `Mailbox` reduces latency on supporting
/// platforms; `Immediate` disables vsync (tear-prone, benchmark-only).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum PresentMode {
    #[default]
    Fifo,
    Mailbox,
    Immediate,
}

/// `Bgra8unorm` is the most universally supported swap-chain format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SurfaceFormat {
    Rgba8unorm,
    #[default]
    Bgra8unorm,
    Rgba16float,
}

/// Every field optional with sensible defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Type, bon::Builder)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SurfaceOpts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[builder(into)]
    pub title: Option<String>,
    #[serde(default = "default_width")]
    #[builder(default = default_width())]
    pub width: u32,
    #[serde(default = "default_height")]
    #[builder(default = default_height())]
    pub height: u32,
    #[serde(default)]
    #[builder(default)]
    pub backend: Backend,
    #[serde(default)]
    #[builder(default)]
    pub power_preference: PowerPreference,
    #[serde(default)]
    #[builder(default)]
    pub present_mode: PresentMode,
    #[serde(default)]
    #[builder(default)]
    pub format: SurfaceFormat,
}

impl Default for SurfaceOpts {
    fn default() -> Self {
        Self {
            title: None,
            width: default_width(),
            height: default_height(),
            backend: Backend::default(),
            power_preference: PowerPreference::default(),
            present_mode: PresentMode::default(),
            format: SurfaceFormat::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AdapterInfo {
    pub vendor: String,
    pub device: String,
    pub driver: String,
}

/// `id` rendered as JS `number` to match the registry key the JS side
/// uses for `resize` / `close` / event subscription.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceHandle {
    #[specta(type = f64)]
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub backend: String,
    pub adapter: AdapterInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AvailabilityInfo {
    pub available: bool,
    pub backends: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CloseResult {
    pub closed: bool,
}

#[plugin(name = "wgpu")]
pub struct WgpuPlugin {
    surfaces: Registry<backend::Surface>,
}

impl Default for WgpuPlugin {
    fn default() -> Self {
        Self {
            surfaces: Registry::new(MAX_SURFACES),
        }
    }
}

impl std::fmt::Debug for WgpuPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuPlugin")
            .field("surfaces", &self.surfaces)
            .finish_non_exhaustive()
    }
}

// `#[command]` requires `&self`; availability probes only shared backend state.
#[allow(clippy::unused_self)]
#[plugin_impl]
impl WgpuPlugin {
    #[command]
    pub fn available(&self) -> PluginResult<AvailabilityInfo> {
        Ok(backend::available())
    }

    #[command]
    pub fn create_surface(
        &self,
        cx: &PluginCx,
        opts: Option<SurfaceOpts>,
    ) -> PluginResult<SurfaceHandle> {
        let opts = opts.unwrap_or_default();
        validate_surface_size(
            "createSurface",
            opts.width,
            opts.height,
            MAX_SURFACE_DIMENSION,
        )?;
        if let Some(reason) = platform_unsupported_reason() {
            return Err(PluginError::Custom(format!("wgpu.createSurface: {reason}")));
        }
        // Free registry slots held by surfaces whose window the user already closed.
        self.prune_dead();
        // Native windows are privileged; absence of the security dispatcher
        // is a denial, not a compatibility escape hatch.
        cx.os_call(
            "security",
            "checkWindow",
            json!({
                "title": &opts.title,
                "width": opts.width,
                "height": opts.height,
            }),
        )?;
        let emit = cx.emitter();
        // The id is needed inside `create_surface` so the worker thread
        // can stamp it onto every emitted event (frame / resize / close)
        // — without this, multi-surface listeners can't be filtered by
        // surface id on the JS side.
        let mut info_slot: Option<backend::SurfaceInfo> = None;
        let id = self.surfaces.insert_with(|id| {
            let surface = backend::create_surface(id, opts, emit)?;
            info_slot = Some(surface.info());
            Ok::<_, PluginError>(surface)
        })?;
        let info = info_slot.expect("create_surface populates info_slot on success");
        Ok(SurfaceHandle {
            id,
            width: info.width,
            height: info.height,
            backend: info.backend,
            adapter: info.adapter,
        })
    }

    #[command]
    pub fn resize(&self, id: u64, width: u32, height: u32) -> PluginResult<()> {
        validate_surface_size("resize", width, height, MAX_SURFACE_DIMENSION)?;
        self.surfaces
            .with_arg(id, "surfaceId", "wgpu.resize", |s| s.resize(width, height))
    }

    #[command]
    pub fn close(&self, id: u64) -> PluginResult<CloseResult> {
        let removed = self.surfaces.remove(id);
        Ok(CloseResult { closed: removed })
    }

    #[command]
    pub fn list(&self) -> PluginResult<Vec<SurfaceHandle>> {
        self.prune_dead();
        Ok(self.surfaces.snapshot(|id, s| {
            let info = s.info();
            Ok(SurfaceHandle {
                id,
                width: info.width,
                height: info.height,
                backend: info.backend,
                adapter: info.adapter,
            })
        }))
    }

    /// Native host lifecycle hook. The plugin lives in a static registry that is never
    /// dropped, so `Drop` impls do not run at exit: tear every surface down explicitly
    /// (each removal signals its event-loop thread and joins it with a deadline).
    pub fn plugin_shutdown(&self, _cx: &PluginCx) -> PluginResult<()> {
        let ids = self.surfaces.ids();
        // Signal every worker first so the joins in `Drop` overlap instead of running serially.
        for id in &ids {
            let _ = self.surfaces.with(*id, |s| {
                s.request_stop();
                Ok(())
            });
        }
        for id in ids {
            self.surfaces.remove(id);
        }
        Ok(())
    }
}

impl WgpuPlugin {
    /// Remove surfaces whose event-loop thread already ended (window closed by the user,
    /// or the worker failed) so they stop counting against [`MAX_SURFACES`].
    fn prune_dead(&self) {
        for (id, alive) in self.surfaces.snapshot(|id, s| Ok((id, s.is_alive()))) {
            if !alive {
                self.surfaces.remove(id);
            }
        }
    }
}

// `--features native` pulls in wgpu + tao; the no-feature build keeps the
// dispatch surface but every GPU-touching call returns a deterministic
// "rebuild with --features native" error.

#[cfg(not(feature = "native"))]
mod backend {
    use tynd_plugin_sdk::{EmitFn, PluginError, PluginResult};

    use super::{AdapterInfo, AvailabilityInfo, SurfaceOpts};

    pub(crate) struct Surface;

    pub(crate) struct SurfaceInfo {
        pub width: u32,
        pub height: u32,
        pub backend: String,
        pub adapter: AdapterInfo,
    }

    // Stub mirrors the real `Surface` shape so dispatch is feature-agnostic;
    // `unused_self` is silenced because methods exist only for ABI parity.
    #[allow(clippy::unused_self)] // Stub methods mirror stateful native backend signatures.
    impl Surface {
        pub(crate) fn info(&self) -> SurfaceInfo {
            SurfaceInfo {
                width: 0,
                height: 0,
                backend: "stub".into(),
                adapter: AdapterInfo {
                    vendor: String::new(),
                    device: String::new(),
                    driver: String::new(),
                },
            }
        }
        pub(crate) fn resize(&mut self, _w: u32, _h: u32) -> PluginResult<()> {
            Err(PluginError::Custom(
                "wgpu.resize: rebuild with --features native".into(),
            ))
        }
        pub(crate) fn is_alive(&self) -> bool {
            false
        }
        pub(crate) fn request_stop(&self) {}
    }

    pub(crate) fn create_surface(
        _id: u64,
        _opts: SurfaceOpts,
        _emit: EmitFn,
    ) -> PluginResult<Surface> {
        Err(PluginError::Custom(
            "wgpu.createSurface: rebuild with --features native".into(),
        ))
    }

    pub(crate) fn available() -> AvailabilityInfo {
        AvailabilityInfo {
            available: false,
            backends: Vec::new(),
            reason: Some("native feature not enabled at compile time".into()),
        }
    }
}

#[cfg(feature = "native")]
mod backend {
    use tynd_plugin_sdk::{EmitFn, PluginError, PluginResult};

    use super::{renderer, window, AdapterInfo, AvailabilityInfo, SurfaceOpts};

    pub(crate) struct Surface {
        pub(crate) inner: window::SurfaceInner,
    }

    pub(crate) struct SurfaceInfo {
        pub width: u32,
        pub height: u32,
        pub backend: String,
        pub adapter: AdapterInfo,
    }

    #[allow(clippy::unused_self)] // Stub methods mirror stateful native backend signatures.
    impl Surface {
        pub(crate) fn info(&self) -> SurfaceInfo {
            SurfaceInfo {
                width: self.inner.width,
                height: self.inner.height,
                backend: self.inner.backend_name.clone(),
                adapter: self.inner.adapter.clone(),
            }
        }

        pub(crate) fn resize(&mut self, w: u32, h: u32) -> PluginResult<()> {
            self.inner.resize(w, h)
        }

        pub(crate) fn is_alive(&self) -> bool {
            self.inner.is_alive()
        }

        pub(crate) fn request_stop(&self) {
            self.inner.request_stop();
        }
    }

    pub(crate) fn create_surface(
        id: u64,
        opts: SurfaceOpts,
        emit: EmitFn,
    ) -> PluginResult<Surface> {
        let inner = window::SurfaceInner::create(id, opts, emit)
            .map_err(|e| PluginError::Custom(format!("wgpu.createSurface: {e}")))?;
        Ok(Surface { inner })
    }

    pub(crate) fn available() -> AvailabilityInfo {
        if let Some(reason) = super::platform_unsupported_reason() {
            return AvailabilityInfo {
                available: false,
                backends: Vec::new(),
                reason: Some(reason.into()),
            };
        }
        let backends = renderer::available_backends();
        AvailabilityInfo {
            available: !backends.is_empty(),
            backends,
            reason: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid_key(err: &PluginError) -> Option<&str> {
        match err {
            PluginError::InvalidArg { key, .. } => Some(key.as_str()),
            _ => None,
        }
    }

    #[test]
    fn validate_accepts_sizes_within_limit() {
        assert!(validate_surface_size("resize", 1, 1, 8192).is_ok());
        assert!(validate_surface_size("resize", 8192, 8192, 8192).is_ok());
        assert!(validate_surface_size("resize", 1280, 720, MAX_SURFACE_DIMENSION).is_ok());
    }

    #[test]
    fn validate_rejects_zero_dimensions() {
        let err = validate_surface_size("resize", 0, 720, 8192).unwrap_err();
        assert_eq!(invalid_key(&err), Some("width"));
        let err = validate_surface_size("resize", 1280, 0, 8192).unwrap_err();
        assert_eq!(invalid_key(&err), Some("height"));
    }

    #[test]
    fn validate_rejects_oversized_dimensions() {
        let err = validate_surface_size("resize", 100_000, 100_000, 8192).unwrap_err();
        assert_eq!(invalid_key(&err), Some("width"));
        let err = validate_surface_size("resize", 100, 8193, 8192).unwrap_err();
        assert_eq!(invalid_key(&err), Some("height"));
        assert!(validate_surface_size("resize", u32::MAX, 1, MAX_SURFACE_DIMENSION).is_err());
    }

    #[test]
    fn clamp_keeps_sizes_in_range() {
        assert_eq!(clamp_surface_size(0, 0, 8192), (1, 1));
        assert_eq!(clamp_surface_size(640, 480, 8192), (640, 480));
        assert_eq!(clamp_surface_size(100_000, 9000, 8192), (8192, 8192));
        assert_eq!(clamp_surface_size(5, 5, 0), (1, 1));
    }

    #[test]
    fn create_surface_rejects_invalid_size_before_touching_the_host() {
        let (cx, capture) = tynd_plugin_sdk::test_cx();
        let plugin = WgpuPlugin::default();
        for (width, height) in [(0, 720), (1280, 0), (100_000, 720), (1280, u32::MAX)] {
            let opts = SurfaceOpts {
                width,
                height,
                ..SurfaceOpts::default()
            };
            let err = plugin.create_surface(&cx, Some(opts)).unwrap_err();
            assert!(
                matches!(err, PluginError::InvalidArg { .. }),
                "{width}x{height}: {err:?}"
            );
        }
        assert!(capture.is_empty());
    }

    #[test]
    fn resize_unknown_surface_is_invalid_arg() {
        let plugin = WgpuPlugin::default();
        let err = plugin.resize(42, 800, 600).unwrap_err();
        assert_eq!(invalid_key(&err), Some("surfaceId"));
    }

    #[test]
    fn resize_rejects_oversized_before_registry_lookup() {
        // The size check runs first, so an oversized request on an unknown id
        // reports the offending dimension, not the missing surface.
        let plugin = WgpuPlugin::default();
        let err = plugin
            .resize(1, MAX_SURFACE_DIMENSION + 1, 100)
            .unwrap_err();
        assert_eq!(invalid_key(&err), Some("width"));
        let err = plugin
            .resize(1, 100, MAX_SURFACE_DIMENSION + 1)
            .unwrap_err();
        assert_eq!(invalid_key(&err), Some("height"));
    }

    // The native `Surface` needs a real window, so the existing-surface path
    // is only reachable with the stub backend.
    #[cfg(not(feature = "native"))]
    #[test]
    fn resize_rejects_oversized_on_an_existing_surface() {
        let plugin = WgpuPlugin::default();
        let id = plugin.surfaces.insert(backend::Surface).unwrap();
        for (width, height, key) in [
            (MAX_SURFACE_DIMENSION + 1, 100, "width"),
            (100, MAX_SURFACE_DIMENSION + 1, "height"),
            (0, 100, "width"),
        ] {
            let err = plugin.resize(id, width, height).unwrap_err();
            assert_eq!(invalid_key(&err), Some(key), "{width}x{height}");
        }
        // A valid size reaches the backend (the stub reports "rebuild").
        let err = plugin.resize(id, 100, 100).unwrap_err();
        assert!(invalid_key(&err).is_none(), "{err:?}");
        assert_eq!(
            plugin.list().unwrap().len(),
            0,
            "stub surfaces are not alive"
        );
    }

    #[test]
    fn list_and_shutdown_on_empty_registry() {
        let (cx, _capture) = tynd_plugin_sdk::test_cx();
        let plugin = WgpuPlugin::default();
        assert!(plugin.list().unwrap().is_empty());
        plugin.plugin_shutdown(&cx).unwrap();
        assert!(!plugin.close(7).unwrap().closed);
    }

    #[test]
    fn platform_support_matches_target() {
        assert_eq!(platform_unsupported_reason().is_none(), cfg!(windows));
    }

    #[cfg(not(feature = "native"))]
    #[test]
    fn availability_without_native_feature_reports_reason() {
        let info = WgpuPlugin::default().available().unwrap();
        assert!(!info.available);
        assert!(info.reason.is_some());
    }

    #[cfg(feature = "native")]
    #[test]
    fn availability_reports_reason_on_unsupported_platforms() {
        let info = WgpuPlugin::default().available().unwrap();
        assert_eq!(info.available, platform_unsupported_reason().is_none());
        assert_eq!(info.reason.is_some(), !info.available);
    }
}

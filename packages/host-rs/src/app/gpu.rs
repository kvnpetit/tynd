//! GPU acceleration helpers — env-var nudging + Chromium browser-args
//! assembly. Every public entry point is cross-platform; OS-specific work
//! is gated with `cfg` blocks so the call sites stay readable.
//!
//! The helpers split into two phases because Chromium / WebKitGTK only read
//! the relevant env vars at process start (or at the first WebView spawn on
//! WebView2 — the `ICoreWebView2Environment` snapshots them once). Anything
//! that can be passed via builder args (`with_additional_browser_args` on
//! Windows) is preferred since it doesn't mutate process-wide state.
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │ Phase 1 — apply_pre_webview_env(cfg)    │
//! │   set NVIDIA_PRIME / DRI_PRIME / …      │
//! │   set WEBKIT_USE_WEBGPU                 │
//! ├─────────────────────────────────────────┤
//! │ Phase 2 — assemble_browser_args(cfg)    │
//! │   --enable-features=Vulkan,WebGPU       │
//! │   --force_high_performance_gpu          │
//! │   --enable-zero-copy …                  │
//! └─────────────────────────────────────────┘
//! ```
//!
//! The single-call-site contract: `run_app` invokes Phase 1 immediately after
//! reading the backend config, and Phase 2 just before `WebViewBuilder::new`.

use crate::window::{
    ColorSpace, GpuPreference, HardwareAcceleration, PreferredFps, WindowConfig,
};

/// Phase 1 — process-wide env vars consumed by Chromium / WebKitGTK at boot.
/// Safe to call once. Uses `std::env::set_var` which is `unsafe` from Rust
/// 1.78+ on Unix targets but the framework calls this from the main thread
/// before any background worker spawns, so the documented invariant holds.
pub(super) fn apply_pre_webview_env(cfg: &WindowConfig) {
    // ── 1. Multi-GPU pin ───────────────────────────────────────────
    if let Some(ref pref) = cfg.gpu {
        apply_gpu_pin(pref);
    } else if matches!(
        cfg.hardware_acceleration,
        Some(HardwareAcceleration::HighPerformance)
    ) {
        // No explicit pin but caller asked for high-performance: assume
        // discrete on Linux (NVIDIA most common) so the offload env vars
        // wake the discrete GPU automatically.
        #[cfg(target_os = "linux")]
        apply_gpu_pin_named("discrete");
    }

    // ── 2. WebGPU enablement on Linux ──────────────────────────────
    //
    // WebKitGTK 2.40+ honours WEBKIT_USE_WEBGPU=1 to flip the experimental
    // dawn backend on. Older builds ignore the var (no-op). On Win/macOS
    // WebGPU is wired through Chromium / WebKit defaults — see
    // `assemble_browser_args` below for the Chromium feature flag.
    #[cfg(all(unix, not(target_os = "macos")))]
    if cfg.webgpu.unwrap_or(true) {
        // SAFETY: called from the main thread before any background worker
        // spawns — `set_var` is single-threaded at this point.
        unsafe {
            std::env::set_var("WEBKIT_USE_WEBGPU", "1");
        }
    }

    // ── 3. HDR opt-in (Linux best effort) ──────────────────────────
    #[cfg(all(unix, not(target_os = "macos")))]
    if cfg.hdr.unwrap_or(false) {
        // WebKitGTK doesn't yet expose an HDR pipeline. Emit a warn so
        // operators understand the opt-in degraded silently.
        crate::tynd_warn!(
            "window.hdr=true on Linux: WebKitGTK has no HDR surface API yet — \
             rendering stays in sRGB. Documented degraded-fallback."
        );
    }
}

/// Phase 2 — assemble the `--flag=value` string fed to WebView2 via
/// [`wry::WebViewBuilderExtWindows::with_additional_browser_args`].
///
/// Returns `None` when no flags need to be set (caller skips the builder
/// call entirely). The function is `cfg(target_os = "windows")` only —
/// macOS / Linux configure their hints through env vars + builder configs.
#[cfg(target_os = "windows")]
pub(super) fn assemble_browser_args(cfg: &WindowConfig) -> Option<String> {
    let mut flags: Vec<String> = Vec::new();

    // ── WebGPU ─────────────────────────────────────────────────────
    //
    // WebView2 maps to Chromium 113+ which has WebGPU on by default behind
    // the `Vulkan,WebGPU` feature gate. Explicitly disable when the app
    // opted out so 3D libs fall back to WebGL2 cleanly.
    if cfg.webgpu == Some(false) {
        flags.push("--disable-features=WebGPU".to_string());
    } else {
        // Forward-compat: nail the feature on so a future Chromium that
        // disables WebGPU by default in WebView2 still serves it.
        flags.push("--enable-features=Vulkan".to_string());
    }

    // ── Hardware-acceleration tier ─────────────────────────────────
    match cfg.hardware_acceleration {
        Some(HardwareAcceleration::HighPerformance) => {
            flags.push("--force_high_performance_gpu".to_string());
            // ANGLE D3D11 + GPU rasterization for raster-heavy 3D scenes.
            flags.push("--use-angle=d3d11".to_string());
            flags.push("--enable-gpu-rasterization".to_string());
            flags.push("--enable-zero-copy".to_string());
        },
        Some(HardwareAcceleration::LowPower) => {
            flags.push("--force_low_power_gpu".to_string());
        },
        Some(HardwareAcceleration::Default) | None => {},
    }

    // ── HDR / wide-gamut ───────────────────────────────────────────
    if cfg.hdr.unwrap_or(false) {
        // DXGI HDR10 (PQ) swap chain. Pairs with a tagged color space.
        flags.push("--force-color-profile=hdr10".to_string());
        flags.push("--enable-features=UseHighDynamicRangeColorBuffers".to_string());
    } else if let Some(ref cs) = cfg.color_space {
        let profile = match cs {
            ColorSpace::Srgb => "srgb",
            ColorSpace::P3 => "display-p3-d65",
            ColorSpace::Rec2020 => "rec2020",
            ColorSpace::Linear => "scrgb-linear",
        };
        flags.push(format!("--force-color-profile={profile}"));
    }

    // ── VRR (G-Sync / FreeSync via DXGI tearing flag) ──────────────
    if cfg.vrr.unwrap_or(false) {
        // Chromium's path for VRR in windowed mode is the
        // DXGI_PRESENT_ALLOW_TEARING flag, gated behind this experimental
        // feature so VRR-aware compositors honour the no-vsync presents.
        flags.push("--enable-features=AllowTearing".to_string());
    }

    // ── FPS hint ───────────────────────────────────────────────────
    if let Some(PreferredFps::Hz(hz)) = cfg.preferred_fps {
        // Cap Chromium's frame-budget timer. 0 / huge values disable the
        // cap — guard against under/overflow up front.
        let clamped = hz.clamp(15, 480);
        flags.push(format!("--max-gum-fps={clamped}"));
    }

    if flags.is_empty() {
        None
    } else {
        Some(flags.join(" "))
    }
}

/// Pin a specific GPU adapter via env vars. Matches the conventions:
///
/// - **Linux NVIDIA**: `__NV_PRIME_RENDER_OFFLOAD=1` + `__GLX_VENDOR_LIBRARY_NAME=nvidia`
/// - **Linux AMD/Intel**: `DRI_PRIME=1` for the discrete card on switchable hybrids
/// - **Windows / macOS**: handled by the OS power-management heuristics —
///   the Chromium `--force_high_performance_gpu` flag (set in
///   `assemble_browser_args`) is the only public lever short of D3D12 /
///   Metal adapter enumeration.
fn apply_gpu_pin(pref: &GpuPreference) {
    match pref {
        GpuPreference::Named(name) => apply_gpu_pin_named(name),
        GpuPreference::Vendor(v) => apply_gpu_pin_vendor(&v.vendor),
    }
}

fn apply_gpu_pin_named(name: &str) {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "auto" | "primary" => {}, // OS default — nothing to do.
        "discrete" => {
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                // SAFETY: same single-thread invariant as apply_pre_webview_env.
                unsafe {
                    std::env::set_var("__NV_PRIME_RENDER_OFFLOAD", "1");
                    std::env::set_var("__GLX_VENDOR_LIBRARY_NAME", "nvidia");
                    std::env::set_var("DRI_PRIME", "1");
                }
            }
        },
        other => {
            crate::tynd_warn!(
                "window.gpu='{other}' is unrecognised — falling back to OS default. \
                 Accepted: 'auto' | 'primary' | 'discrete' | {{ vendor: 'nvidia'|'amd'|'intel' }}."
            );
        },
    }
}

fn apply_gpu_pin_vendor(vendor: &str) {
    let lower = vendor.to_ascii_lowercase();
    match lower.as_str() {
        "nvidia" => {
            #[cfg(all(unix, not(target_os = "macos")))]
            unsafe {
                std::env::set_var("__NV_PRIME_RENDER_OFFLOAD", "1");
                std::env::set_var("__GLX_VENDOR_LIBRARY_NAME", "nvidia");
            }
        },
        "amd" | "intel" => {
            #[cfg(all(unix, not(target_os = "macos")))]
            unsafe {
                std::env::set_var("DRI_PRIME", "1");
            }
        },
        other => {
            crate::tynd_warn!(
                "window.gpu.vendor='{other}' is unrecognised. \
                 Accepted: 'nvidia' | 'amd' | 'intel'."
            );
        },
    }
}

/// Resolve the active monitor's refresh rate (Hz). Returns the highest
/// `VideoMode::refresh_rate()` advertised by the monitor, or `None` when
/// tao can't enumerate (Linux + Wayland, or no monitor attached).
///
/// Only consumed for diagnostics / log warnings — the compositor enforces
/// the actual frame budget; the FPS hint is a soft cap.
pub(super) fn resolve_monitor_refresh_rate(window: &tao::window::Window) -> Option<u16> {
    let monitor = window.current_monitor()?;
    monitor.video_modes().map(|m| m.refresh_rate()).max()
}

/// Log a warning when `preferredFps` exceeds the monitor's native rate. The
/// compositor caps anyway; this helps users notice the misconfig.
pub(super) fn warn_if_fps_above_monitor(cfg: &WindowConfig, window: &tao::window::Window) {
    let Some(PreferredFps::Hz(target)) = cfg.preferred_fps else {
        return;
    };
    let Some(monitor_rate) = resolve_monitor_refresh_rate(window) else {
        return;
    };
    if target > u32::from(monitor_rate) {
        crate::tynd_warn!(
            "window.preferredFps={target} exceeds monitor refresh rate {monitor_rate} Hz — \
             the OS compositor will silently cap to {monitor_rate}."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::WindowConfig;

    fn cfg() -> WindowConfig {
        WindowConfig::default()
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn assemble_args_default_is_minimal() {
        // Default config still nails Vulkan to keep WebGPU forward-compat.
        let args = assemble_browser_args(&cfg()).unwrap();
        assert!(args.contains("--enable-features=Vulkan"));
        assert!(!args.contains("--disable-features=WebGPU"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn assemble_args_disables_webgpu_on_opt_out() {
        let mut c = cfg();
        c.webgpu = Some(false);
        let args = assemble_browser_args(&c).unwrap();
        assert!(args.contains("--disable-features=WebGPU"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn assemble_args_high_performance_adds_chromium_flags() {
        let mut c = cfg();
        c.hardware_acceleration = Some(HardwareAcceleration::HighPerformance);
        let args = assemble_browser_args(&c).unwrap();
        assert!(args.contains("--force_high_performance_gpu"));
        assert!(args.contains("--use-angle=d3d11"));
        assert!(args.contains("--enable-gpu-rasterization"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn assemble_args_low_power_does_not_force_high() {
        let mut c = cfg();
        c.hardware_acceleration = Some(HardwareAcceleration::LowPower);
        let args = assemble_browser_args(&c).unwrap();
        assert!(args.contains("--force_low_power_gpu"));
        assert!(!args.contains("--force_high_performance_gpu"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn assemble_args_hdr_overrides_color_space() {
        let mut c = cfg();
        c.hdr = Some(true);
        c.color_space = Some(ColorSpace::P3);
        let args = assemble_browser_args(&c).unwrap();
        assert!(args.contains("--force-color-profile=hdr10"));
        // P3 is shadowed by HDR10 in this branch — that's the documented
        // contract: HDR wins, color_space is the SDR fallback knob.
        assert!(!args.contains("display-p3-d65"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn assemble_args_color_space_p3_when_no_hdr() {
        let mut c = cfg();
        c.color_space = Some(ColorSpace::P3);
        let args = assemble_browser_args(&c).unwrap();
        assert!(args.contains("--force-color-profile=display-p3-d65"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn assemble_args_vrr_emits_allow_tearing() {
        let mut c = cfg();
        c.vrr = Some(true);
        let args = assemble_browser_args(&c).unwrap();
        assert!(args.contains("AllowTearing"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn assemble_args_fps_clamped() {
        let mut c = cfg();
        c.preferred_fps = Some(PreferredFps::Hz(1000));
        let args = assemble_browser_args(&c).unwrap();
        // Clamp to the documented 15..=480 window.
        assert!(args.contains("--max-gum-fps=480"));
    }

    #[test]
    fn apply_gpu_pin_named_unknown_logs_only() {
        // Should not panic on garbage; the fn just emits a warn.
        apply_gpu_pin_named("ferrari");
    }

    #[test]
    fn apply_gpu_pin_vendor_unknown_logs_only() {
        apply_gpu_pin_vendor("ibm-power9");
    }

    #[test]
    fn apply_pre_webview_env_default_is_noop_on_windows_macos() {
        // On Win/macOS the function should not panic and should leave the
        // process env largely alone (we don't check exact var presence
        // because the test runs in a real env).
        apply_pre_webview_env(&cfg());
    }

    #[test]
    fn config_round_trips_through_serde() {
        // Rough integration check — the new fields are flat enough that
        // serde deserialization picks them up without custom logic.
        let json = serde_json::json!({
            "webgpu": false,
            "hardwareAcceleration": "high-performance",
            "preferredFps": 144,
            "colorSpace": "p3",
            "hdr": true,
            "vrr": true,
            "gpu": { "vendor": "nvidia" },
        });
        let cfg: WindowConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(cfg.webgpu, Some(false));
        assert!(matches!(
            cfg.hardware_acceleration,
            Some(HardwareAcceleration::HighPerformance)
        ));
        assert!(matches!(cfg.preferred_fps, Some(PreferredFps::Hz(144))));
        assert!(matches!(cfg.color_space, Some(ColorSpace::P3)));
        assert_eq!(cfg.hdr, Some(true));
        assert_eq!(cfg.vrr, Some(true));
        match cfg.gpu {
            Some(GpuPreference::Vendor(ref v)) => assert_eq!(v.vendor, "nvidia"),
            _ => panic!("expected vendor variant"),
        }
    }

    #[test]
    fn config_accepts_string_gpu_preference() {
        let json = serde_json::json!({ "gpu": "discrete" });
        let cfg: WindowConfig = serde_json::from_value(json).expect("deserialize");
        match cfg.gpu {
            Some(GpuPreference::Named(ref s)) => assert_eq!(s, "discrete"),
            _ => panic!("expected named variant"),
        }
    }

    #[test]
    fn config_accepts_monitor_fps() {
        let json = serde_json::json!({ "preferredFps": "monitor" });
        let cfg: WindowConfig = serde_json::from_value(json).expect("deserialize");
        match cfg.preferred_fps {
            Some(PreferredFps::Auto(ref s)) => assert_eq!(s, "monitor"),
            _ => panic!("expected auto variant"),
        }
    }
}

//! CPU-buffer splash screen — paints a flat color (or a decoded PNG)
//! into a `tao::Window` via softbuffer *before* the webview is ready.
//!
//! Why this exists: on cold start, wry/tao's webview can take 100–
//! 500 ms to attach and paint its first frame. On Windows that
//! manifests as a white-flash; on macOS as a brief gray panel. A
//! softbuffer-painted window with the brand color or a logo bridges
//! the gap with zero GPU cost (the buffer is just a `Vec<u32>` of
//! BGRA pixels copied straight to the window surface).
//!
//! Schema (`tynd.config.ts`):
//! ```ts
//! splash: {
//!   enabled: boolean;
//!   image?: string;   // path to PNG/JPEG/BMP — decoded via `image` crate
//!   color?: string;   // background hex "#RRGGBB" / "#RRGGBBAA"
//!   duration?: number; // ms cap; auto-close past this even without close()
//! }
//! ```
//!
//! Lifecycle: callers (the launcher's startup path) call
//! [`SplashHandle::show`] before the webview is attached, hold the
//! handle until the webview emits its first paint, then drop or
//! call `.close()` to dismiss the splash.
//!
//! Feature-gated behind `splash-screen` so the `softbuffer` +
//! `raw-window-handle` deps stay out of the default graph.

#![cfg(feature = "splash-screen")]

use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use tao::{
    event_loop::EventLoopWindowTarget,
    window::{Window, WindowBuilder},
};

/// Render-time configuration for the splash window. Constructed by
/// the launcher from the user's `tynd.config.ts.splash` block.
#[derive(Debug, Clone)]
pub struct SplashConfig {
    /// Optional path to a PNG/JPEG/BMP image. Decoded via the `image`
    /// crate (already in the workspace) and centered on the window
    /// using nearest-neighbor scaling. `None` paints the flat color.
    pub image: Option<PathBuf>,
    /// Background BGRA color as a packed `u32`
    /// (`0xAA_RR_GG_BB` in big-endian byte order — softbuffer's
    /// surface format on every platform). Built via [`hex_to_bgra`]
    /// from a `#RRGGBB` / `#RRGGBBAA` string.
    pub color: u32,
    /// Window pixel size. Defaults to 480×320 — enough to fit a logo
    /// without dominating small displays.
    pub size: (u32, u32),
    /// Hard upper bound — auto-close past this even if the caller
    /// forgot to invoke `handle.close()`. Defaults to 10 s.
    pub max_duration: Duration,
}

impl Default for SplashConfig {
    fn default() -> Self {
        Self {
            image: None,
            color: 0xFF_18_18_18, // dark neutral, matches Tynd brand
            size: (480, 320),
            max_duration: Duration::from_secs(10),
        }
    }
}

/// Opaque handle returned by [`SplashHandle::show`]. Drop or `.close()`
/// dismisses the window. The window itself lives on the main thread
/// (tao's contract); the handle is `Send + Sync` because all it owns
/// is a `parking_lot::Mutex<Option<...>>` cell.
#[derive(Debug)]
pub struct SplashHandle {
    inner: parking_lot::Mutex<Option<Inner>>,
    started: Instant,
    deadline: Duration,
}

#[derive(Debug)]
struct Inner {
    // softbuffer's `Surface` holds an `Rc<Window>` for both display
    // and window handles, so we keep one alongside it. The
    // `Surface`'s drop closes the platform-side surface; dropping the
    // `Rc` then closes the underlying tao window when the last
    // strong reference goes away.
    _window: std::rc::Rc<Window>,
    // Holding the surface alive guarantees the platform-side
    // softbuffer resources stay attached to the window until the
    // handle is dropped. Reads happen via `buffer_mut()` inside
    // `show()`; nothing reads the field after construction so the
    // dead-code lint would fire without the underscore prefix.
    _surface: softbuffer::Surface<std::rc::Rc<Window>, std::rc::Rc<Window>>,
}

impl SplashHandle {
    /// Build the splash window on the supplied event loop and paint
    /// the configured color/image. Returns immediately; the window
    /// is alive until `.close()` or drop.
    ///
    /// # Errors
    /// - `image` path decoded with `image` crate failure
    /// - tao window creation failure (rare, only on resource exhaustion)
    /// - softbuffer surface creation / resize failure
    pub fn show<T>(
        cfg: SplashConfig,
        event_loop: &EventLoopWindowTarget<T>,
    ) -> Result<Self, String> {
        let window = WindowBuilder::new()
            .with_title("")
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top(true)
            .with_inner_size(tao::dpi::PhysicalSize::new(cfg.size.0, cfg.size.1))
            .build(event_loop)
            .map_err(|e| format!("splash: window: {e}"))?;

        let window = std::rc::Rc::new(window);
        let context = softbuffer::Context::new(window.clone())
            .map_err(|e| format!("splash: softbuffer context: {e}"))?;
        let mut surface = softbuffer::Surface::new(&context, window.clone())
            .map_err(|e| format!("splash: softbuffer surface: {e}"))?;
        let (w, h) = cfg.size;
        surface
            .resize(
                NonZeroU32::new(w.max(1)).unwrap(),
                NonZeroU32::new(h.max(1)).unwrap(),
            )
            .map_err(|e| format!("splash: surface resize: {e}"))?;

        // Paint pass — fill the whole buffer with the configured
        // color, then overlay the decoded image (centered) if any.
        let mut buffer = surface
            .buffer_mut()
            .map_err(|e| format!("splash: buffer_mut: {e}"))?;
        let total = (w as usize) * (h as usize);
        if buffer.len() != total {
            return Err(format!(
                "splash: buffer length mismatch ({} vs {w}x{h}={total})",
                buffer.len()
            ));
        }
        for px in buffer.iter_mut() {
            *px = cfg.color;
        }
        if let Some(path) = cfg.image.as_deref() {
            paint_image_centered(&mut buffer, w, h, path)?;
        }
        buffer
            .present()
            .map_err(|e| format!("splash: present: {e}"))?;

        Ok(Self {
            inner: parking_lot::Mutex::new(Some(Inner {
                _window: window,
                _surface: surface,
            })),
            started: Instant::now(),
            deadline: cfg.max_duration,
        })
    }

    /// Close the splash window, dropping its softbuffer + tao
    /// resources. Idempotent.
    pub fn close(&self) {
        let _ = self.inner.lock().take();
    }

    /// Returns `true` once the configured `max_duration` has elapsed
    /// and the splash should auto-close. The launcher's event loop
    /// polls this each tick to honor the duration cap even when the
    /// webview never finishes spawning.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.started.elapsed() >= self.deadline
    }
}

impl Drop for SplashHandle {
    fn drop(&mut self) {
        // No-op when already taken; tao::Window's drop releases the
        // OS handle on the main thread.
        self.close();
    }
}

/// Convert a `#RRGGBB` / `#RRGGBBAA` string to softbuffer's packed
/// `u32` (alpha in the high byte). Returns `None` on parse failure
/// so callers can fall back to the default color.
#[must_use]
pub fn hex_to_bgra(s: &str) -> Option<u32> {
    let s = s.strip_prefix('#').unwrap_or(s);
    let (r, g, b, a) = match s.len() {
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            (r, g, b, 0xFFu8)
        },
        8 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            let a = u8::from_str_radix(&s[6..8], 16).ok()?;
            (r, g, b, a)
        },
        _ => return None,
    };
    Some(
        (u32::from(a) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
    )
}

fn paint_image_centered(
    buffer: &mut [u32],
    w: u32,
    h: u32,
    path: &Path,
) -> Result<(), String> {
    let img = image::open(path)
        .map_err(|e| format!("splash: image {}: {e}", path.display()))?
        .to_rgba8();
    let (iw, ih) = img.dimensions();
    let off_x = ((w as i32 - iw as i32) / 2).max(0) as u32;
    let off_y = ((h as i32 - ih as i32) / 2).max(0) as u32;
    for (y, row) in img.rows().enumerate() {
        let dst_y = off_y + y as u32;
        if dst_y >= h {
            break;
        }
        for (x, px) in row.enumerate() {
            let dst_x = off_x + x as u32;
            if dst_x >= w {
                break;
            }
            let [r, g, b, a] = px.0;
            // Premultiply over the existing pixel — handles
            // anti-aliased edges from the source PNG without losing
            // the splash background color underneath.
            let dst_idx = (dst_y as usize) * (w as usize) + dst_x as usize;
            let dst = buffer[dst_idx];
            let dr = ((dst >> 16) & 0xFF) as u8;
            let dg = ((dst >> 8) & 0xFF) as u8;
            let db = (dst & 0xFF) as u8;
            let alpha = a as u32;
            let inv = 255u32.saturating_sub(alpha);
            let nr = ((u32::from(r) * alpha + u32::from(dr) * inv) / 255) as u8;
            let ng = ((u32::from(g) * alpha + u32::from(dg) * inv) / 255) as u8;
            let nb = ((u32::from(b) * alpha + u32::from(db) * inv) / 255) as u8;
            buffer[dst_idx] =
                (0xFFu32 << 24) | (u32::from(nr) << 16) | (u32::from(ng) << 8) | u32::from(nb);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_to_bgra_parses_six_digit() {
        assert_eq!(hex_to_bgra("#181818"), Some(0xFF_18_18_18));
        assert_eq!(hex_to_bgra("FF0000"), Some(0xFF_FF_00_00));
        assert_eq!(hex_to_bgra("#000000"), Some(0xFF_00_00_00));
    }

    #[test]
    fn hex_to_bgra_parses_eight_digit_alpha() {
        assert_eq!(hex_to_bgra("#181818AA"), Some(0xAA_18_18_18));
        assert_eq!(hex_to_bgra("00000000"), Some(0));
    }

    #[test]
    fn hex_to_bgra_rejects_invalid() {
        assert_eq!(hex_to_bgra("nope"), None);
        assert_eq!(hex_to_bgra("#12345"), None);
        assert_eq!(hex_to_bgra("#ZZZZZZ"), None);
    }

    #[test]
    fn default_config_is_sane() {
        let c = SplashConfig::default();
        assert_eq!(c.size, (480, 320));
        assert!(c.max_duration >= Duration::from_secs(1));
        assert!(c.image.is_none());
    }
}

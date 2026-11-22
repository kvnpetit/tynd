//! Windows taskbar badge: renders a count or a short label into a small
//! icon and applies it with `ITaskbarList3::SetOverlayIcon` (the badge
//! Windows draws over the bottom-right corner of the taskbar button).

use std::{ffi::c_void, ptr};

use windows_sys::{
    core::{GUID, HRESULT},
    Win32::{
        Foundation::{HWND, RECT, RPC_E_CHANGED_MODE},
        Graphics::Gdi::{
            CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC,
            DeleteObject, DrawTextW, SelectObject, SetBkMode, SetTextColor, ANTIALIASED_QUALITY,
            BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, DT_CENTER, DT_NOPREFIX,
            DT_SINGLELINE, DT_VCENTER, FW_BOLD, HBITMAP, HGDIOBJ, TRANSPARENT,
        },
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        },
        UI::{
            Shell::TaskbarList,
            WindowsAndMessaging::{
                CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON,
            },
        },
    },
};

/// Longest label that stays legible in a 16 px overlay.
const MAX_LABEL_CHARS: usize = 3;

/// Badge background, `0xRRGGBB` (Fluent "danger" red).
const BADGE_RGB: u32 = 0x00c4_2b1c;

/// Text to draw for `setBadge`, or `None` to clear the overlay. A label wins
/// over a count, matching macOS.
pub(crate) fn badge_text(
    label: Option<&str>,
    count: Option<i64>,
) -> Result<Option<String>, String> {
    if let Some(label) = label {
        let label = label.trim();
        if label.is_empty() {
            return Ok(None);
        }
        if label.chars().count() > MAX_LABEL_CHARS {
            return Err(format!(
                "setBadge: Windows taskbar badges fit at most {MAX_LABEL_CHARS} characters; \
                 pass a shorter `label` or a `count`"
            ));
        }
        return Ok(Some(label.to_string()));
    }
    Ok(match count {
        Some(n) if n > 99 => Some("99+".to_string()),
        Some(n) if n > 0 => Some(n.to_string()),
        _ => None,
    })
}

/// Set (or clear with `None`) the overlay badge of `hwnd`'s taskbar button.
/// Must run on the thread that owns `hwnd`.
pub(crate) fn set_overlay(hwnd: HWND, text: Option<&str>) -> Result<(), String> {
    let _apartment = Apartment::enter();
    let taskbar = TaskbarList3::create()?;
    let icon = match text {
        Some(text) => Some(render_icon(text)?),
        None => None,
    };
    let description: Vec<u16> = text.unwrap_or("").encode_utf16().chain([0]).collect();
    let hr = taskbar.set_overlay_icon(
        hwnd,
        icon.as_ref().map_or(ptr::null_mut(), |i| i.0),
        &description,
    );
    if hr < 0 {
        return Err(format!(
            "setBadge: ITaskbarList3::SetOverlayIcon failed (hr=0x{hr:08x}); the window needs a \
             taskbar button (not hidden, not skipTaskbar)"
        ));
    }
    Ok(())
}

/// IID_ITaskbarList3 (ShObjIdl_core.h).
const IID_TASKBAR_LIST3: GUID = GUID::from_u128(0xea1a_fb91_9e28_4b86_90e9_9e9f_8a5e_efaf);

/// Vtable prefix of `ITaskbarList3` up to `SetOverlayIcon`.
#[repr(C)]
struct TaskbarList3Vtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(this: *mut c_void) -> u32,
    hr_init: unsafe extern "system" fn(this: *mut c_void) -> HRESULT,
    // AddTab, DeleteTab, ActivateTab, SetActiveAlt, MarkFullscreenWindow,
    // SetProgressValue, SetProgressState, RegisterTab, UnregisterTab,
    // SetTabOrder, SetTabActive, ThumbBarAddButtons, ThumbBarUpdateButtons,
    // ThumbBarSetImageList.
    _skipped: [usize; 14],
    set_overlay_icon: unsafe extern "system" fn(
        this: *mut c_void,
        hwnd: HWND,
        icon: HICON,
        description: *const u16,
    ) -> HRESULT,
}

/// Owned `ITaskbarList3` pointer, released on drop.
struct TaskbarList3(*mut c_void);

impl TaskbarList3 {
    fn vtbl(&self) -> &TaskbarList3Vtbl {
        // SAFETY: a COM object's first field is its vtable pointer.
        unsafe { &**self.0.cast::<*const TaskbarList3Vtbl>() }
    }

    fn create() -> Result<Self, String> {
        let mut object: *mut c_void = ptr::null_mut();
        let clsid = TaskbarList;
        let iid = IID_TASKBAR_LIST3;
        // SAFETY: valid CLSID / IID pointers and out pointer.
        let hr = unsafe {
            CoCreateInstance(
                &raw const clsid,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &raw const iid,
                &raw mut object,
            )
        };
        if hr < 0 || object.is_null() {
            return Err(format!("setBadge: TaskbarList unavailable (hr=0x{hr:08x})"));
        }
        let list = Self(object);
        // SAFETY: `HrInit` must be called once before any other method.
        let hr = unsafe { (list.vtbl().hr_init)(list.0) };
        if hr < 0 {
            return Err(format!(
                "setBadge: ITaskbarList3::HrInit failed (hr=0x{hr:08x})"
            ));
        }
        Ok(list)
    }

    fn set_overlay_icon(&self, hwnd: HWND, icon: HICON, description: &[u16]) -> HRESULT {
        // SAFETY: live interface; `icon` is null or a valid HICON;
        // `description` is NUL-terminated and outlives the call.
        unsafe { (self.vtbl().set_overlay_icon)(self.0, hwnd, icon, description.as_ptr()) }
    }
}

impl Drop for TaskbarList3 {
    fn drop(&mut self) {
        // SAFETY: balances the reference returned by CoCreateInstance.
        unsafe { (self.vtbl().release)(self.0) };
    }
}

/// COM apartment for this call; uninitialized on drop when this call
/// initialized it.
struct Apartment(bool);

impl Apartment {
    fn enter() -> Self {
        // SAFETY: reserved pointer must be null.
        let hr = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        Self(hr >= 0 && hr != RPC_E_CHANGED_MODE)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        }
    }
}

/// Owned HICON, destroyed on drop (the taskbar keeps its own copy).
struct Icon(HICON);

impl Drop for Icon {
    fn drop(&mut self) {
        // SAFETY: icon created by CreateIconIndirect, destroyed once.
        unsafe { DestroyIcon(self.0) };
    }
}

/// Owned GDI object, deleted on drop.
struct GdiObject(HGDIOBJ);

impl Drop for GdiObject {
    fn drop(&mut self) {
        // SAFETY: object created by this module and no longer selected.
        unsafe { DeleteObject(self.0) };
    }
}

/// A 32 bpp top-down DIB section and its pixel buffer.
fn dib(size: i32) -> Result<(GdiObject, *mut u32), String> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };
    let mut bits: *mut c_void = ptr::null_mut();
    // SAFETY: `info` describes a valid 32 bpp DIB; `bits` receives its buffer.
    let bitmap = unsafe {
        CreateDIBSection(
            ptr::null_mut(),
            &raw const info,
            DIB_RGB_COLORS,
            &raw mut bits,
            ptr::null_mut(),
            0,
        )
    };
    if bitmap.is_null() || bits.is_null() {
        return Err("setBadge: CreateDIBSection failed".into());
    }
    Ok((GdiObject(bitmap), bits.cast::<u32>()))
}

/// Draw `text` white-on-black into a `size`² buffer; returns per-pixel
/// coverage (0-255).
fn text_coverage(text: &str, size: i32) -> Result<Vec<u8>, String> {
    let (bitmap, bits) = dib(size)?;
    let pixels = (size * size) as usize;
    // SAFETY: CreateDIBSection allocated `size * size` u32 pixels.
    unsafe { ptr::write_bytes(bits, 0, pixels) };

    // SAFETY: memory DC compatible with the screen; released below.
    let dc = unsafe { CreateCompatibleDC(ptr::null_mut()) };
    if dc.is_null() {
        return Err("setBadge: CreateCompatibleDC failed".into());
    }
    let face: Vec<u16> = "Segoe UI".encode_utf16().chain([0]).collect();
    let font_height = font_height(size, text.chars().count());
    // SAFETY: plain logical-font creation; the face name is NUL-terminated.
    let font = GdiObject(unsafe {
        CreateFontW(
            -font_height,
            0,
            0,
            0,
            FW_BOLD.cast_signed(),
            0,
            0,
            0,
            0,
            0,
            0,
            u32::from(ANTIALIASED_QUALITY),
            0,
            face.as_ptr(),
        )
    });
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: size,
        bottom: size,
    };
    // SAFETY: `dc` is a live memory DC; the previous objects are restored
    // before the DC is deleted, so `bitmap` / `font` are unselected on drop.
    unsafe {
        let old_bitmap = SelectObject(dc, bitmap.0);
        let old_font = SelectObject(dc, font.0);
        SetBkMode(dc, TRANSPARENT.cast_signed());
        SetTextColor(dc, 0x00ff_ffff);
        DrawTextW(
            dc,
            wide.as_ptr(),
            -1,
            &raw mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(dc, old_font);
        SelectObject(dc, old_bitmap);
        DeleteDC(dc);
    }
    // SAFETY: the buffer holds `pixels` u32 values and GDI is done with it.
    let drawn = unsafe { std::slice::from_raw_parts(bits, pixels) };
    Ok(drawn.iter().map(|&px| ((px >> 8) & 0xff) as u8).collect())
}

/// Pixel font height for `chars` characters in a `size` px badge.
fn font_height(size: i32, chars: usize) -> i32 {
    let ratio = match chars {
        0 | 1 => 0.8,
        2 => 0.66,
        _ => 0.5,
    };
    ((f64::from(size) * ratio).round() as i32).max(6)
}

/// Straight-alpha BGRA badge: an anti-aliased disc in [`BADGE_RGB`] with
/// the text coverage blended in white.
fn compose(size: i32, text: &[u8]) -> Vec<u32> {
    let side = size as usize;
    let center = f64::from(size) / 2.0;
    let radius = center - 0.5;
    let (r, g, b) = (
        f64::from((BADGE_RGB >> 16) & 0xff),
        f64::from((BADGE_RGB >> 8) & 0xff),
        f64::from(BADGE_RGB & 0xff),
    );
    let mut out = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            let dx = x as f64 + 0.5 - center;
            let dy = y as f64 + 0.5 - center;
            let disc = (radius + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0);
            let ink = f64::from(text[y * side + x]) / 255.0;
            let mix = |base: f64| (base + (255.0 - base) * ink).round() as u32;
            let alpha = (disc * 255.0).round() as u32;
            out.push((alpha << 24) | (mix(r) << 16) | (mix(g) << 8) | mix(b));
        }
    }
    out
}

fn render_icon(text: &str) -> Result<Icon, String> {
    // SAFETY: plain metric query.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64);
    let coverage = text_coverage(text, size)?;
    let pixels = compose(size, &coverage);
    let (color, bits) = dib(size)?;
    // SAFETY: both buffers hold `size * size` pixels.
    unsafe { ptr::copy_nonoverlapping(pixels.as_ptr(), bits, pixels.len()) };
    // All-zero AND mask: the alpha channel of the color bitmap decides.
    let mask_bytes = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
    // SAFETY: a 1 bpp bitmap whose rows are WORD-aligned, matching `mask_bytes`.
    let mask: HBITMAP = unsafe { CreateBitmap(size, size, 1, 1, mask_bytes.as_ptr().cast()) };
    if mask.is_null() {
        return Err("setBadge: CreateBitmap failed".into());
    }
    let mask = GdiObject(mask);
    let info = ICONINFO {
        fIcon: 1,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask.0,
        hbmColor: color.0,
    };
    // SAFETY: both bitmaps are valid; CreateIconIndirect copies them.
    let icon = unsafe { CreateIconIndirect(&raw const info) };
    if icon.is_null() {
        return Err("setBadge: CreateIconIndirect failed".into());
    }
    Ok(Icon(icon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_text_rules() {
        assert_eq!(badge_text(None, Some(3)).unwrap().as_deref(), Some("3"));
        assert_eq!(badge_text(None, Some(150)).unwrap().as_deref(), Some("99+"));
        assert_eq!(badge_text(None, Some(0)).unwrap(), None);
        assert_eq!(badge_text(None, None).unwrap(), None);
        assert_eq!(
            badge_text(Some(" ! "), Some(4)).unwrap().as_deref(),
            Some("!")
        );
        assert_eq!(badge_text(Some(""), Some(4)).unwrap(), None);
        assert!(badge_text(Some("new!"), None)
            .unwrap_err()
            .contains("at most 3"));
    }

    #[test]
    fn compose_draws_an_opaque_disc_with_transparent_corners() {
        let size = 16;
        let pixels = compose(size, &vec![0; 256]);
        assert_eq!(pixels[0] >> 24, 0, "corner is transparent");
        let middle = pixels[8 * 16 + 8];
        assert_eq!(middle >> 24, 255, "center is opaque");
        assert_eq!(middle & 0x00ff_ffff, BADGE_RGB);
        let inked = compose(size, &vec![255; 256]);
        assert_eq!(
            inked[8 * 16 + 8] & 0x00ff_ffff,
            0x00ff_ffff,
            "text is white"
        );
    }

    #[test]
    fn renders_an_icon() {
        assert!(render_icon("7").is_ok());
        assert!(render_icon("99+").is_ok());
    }
}

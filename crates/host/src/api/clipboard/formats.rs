//! Raw clipboard formats: RTF, custom formats by MIME type / Windows format
//! name / macOS UTI, and the list of formats on the clipboard.
//!
//! `arboard` only covers text, HTML, images and file lists, so these go to the
//! native clipboard: Win32 `OpenClipboard` / `GetClipboardData` on Windows,
//! `NSPasteboard` on macOS. Linux (X11 / Wayland selections) is not
//! supported: every call rejects.

/// MIME type of rich text.
pub(super) const RTF: &str = "text/rtf";
/// Largest payload read or written through these calls.
pub(super) const MAX_FORMAT_BYTES: usize = 64 * 1024 * 1024;

/// Validate a caller-supplied format name.
pub(super) fn validate_format(format: &str, op: &str) -> Result<(), String> {
    if format.is_empty() || format.len() > 256 || format.chars().any(char::is_control) {
        return Err(format!(
            "clipboard.{op}: invalid format '{format}' (1-256 characters, no control characters)"
        ));
    }
    if format.eq_ignore_ascii_case("text/html") {
        return Err(format!(
            "clipboard.{op}: use readHtml / writeHtml for text/html"
        ));
    }
    Ok(())
}

/// Native formats that hand files to the app the clipboard is pasted into:
/// Windows shell formats, macOS file URLs / filename lists, Linux file-manager
/// formats and the `text/uri-list` MIME type. Compared case-insensitively.
///
/// URL formats are refused too (`UniformResourceLocator(W)`, `public.url`,
/// `Apple URL pasteboard type`, `text/x-moz-url`): they carry `file:` URLs as
/// readily as web ones and Explorer / Finder turn a pasted one into a file
/// shortcut. A plain URL goes through `writeText`.
const FILE_REFERENCE_FORMATS: &[&str] = &[
    // Windows (`CF_HDROP` is format 15; the rest are registered shell names).
    "CF_HDROP",
    "FileName",
    "FileNameW",
    "FileNameMap",
    "FileNameMapW",
    "Shell IDList Array",
    "FileGroupDescriptor",
    "FileGroupDescriptorW",
    "FileContents",
    "Shell Object Offsets",
    "UniformResourceLocator",
    "UniformResourceLocatorW",
    // macOS.
    "public.file-url",
    "public.url",
    "NSFilenamesPboardType",
    "NSFilesPromisePboardType",
    "Apple files promise pasteboard type",
    "Apple URL pasteboard type",
    "com.apple.NSFilePromiseItemMetaData",
    "CorePasteboardFlavorType 0x6675726C",
    "CorePasteboardFlavorType 0x75726C20",
    // MIME (Linux file managers, and the macOS / Windows mapping).
    "text/uri-list",
    "text/x-moz-url",
    "x-special/gnome-copied-files",
    "x-special/nautilus-clipboard",
    "application/x-kde-cutselection",
    "application/x-kde4-urilist",
];

/// Prefixes of file-reference format families, compared case-insensitively:
/// - `dyn.`: macOS dynamic UTIs, which alias any legacy pasteboard type
///   (`NSFilenamesPboardType` has one) and so can name every entry above;
/// - `com.apple.pasteboard.promised-file-`: Finder file promises.
const FILE_REFERENCE_PREFIXES: &[&str] = &["dyn.", "com.apple.pasteboard.promised-file-"];

fn is_file_reference(name: &str) -> bool {
    FILE_REFERENCE_FORMATS
        .iter()
        .any(|denied| denied.eq_ignore_ascii_case(name))
        || FILE_REFERENCE_PREFIXES.iter().any(|prefix| {
            name.get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        })
}

/// [`validate_format`] for a format about to be written: also refuses the
/// file-reference formats, which would hand files to other apps without the
/// filesystem policy check `writeFiles` applies.
pub(super) fn validate_writable_format(format: &str, op: &str) -> Result<(), String> {
    validate_format(format, op)?;
    if is_file_reference(format)
        || is_file_reference(uti_for_mime(format))
        || is_file_reference(windows_name_for_mime(format))
    {
        return Err(format!(
            "clipboard.{op}: '{format}' is a file-reference format; use writeFiles, which checks the filesystem policy"
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) use windows::{available_formats, read_format, write_formats};

#[cfg(target_os = "macos")]
pub(super) use macos::{available_formats, read_format, write_formats};

#[cfg(all(unix, not(target_os = "macos")))]
fn unsupported(op: &str) -> String {
    format!("clipboard.{op}: custom clipboard formats are not supported on linux")
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn available_formats() -> Result<Vec<String>, String> {
    Err(unsupported("availableFormats"))
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn read_format(_format: &str, op: &str) -> Result<Option<Vec<u8>>, String> {
    Err(unsupported(op))
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn write_formats(_items: &[(String, Vec<u8>)], op: &str) -> Result<(), String> {
    Err(unsupported(op))
}

/// Map a native Windows format name back to the MIME type callers use.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(super) fn mime_for_windows_name(name: &str) -> &str {
    match name {
        "Rich Text Format" => RTF,
        "HTML Format" => "text/html",
        "PNG" => "image/png",
        other => other,
    }
}

/// Map a MIME type to the Windows registered format name.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(super) fn windows_name_for_mime(format: &str) -> &str {
    match format.to_ascii_lowercase().as_str() {
        RTF | "application/rtf" => "Rich Text Format",
        "image/png" => "PNG",
        _ => format,
    }
}

/// Map a MIME type to the macOS pasteboard type (UTI).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn uti_for_mime(format: &str) -> &str {
    match format.to_ascii_lowercase().as_str() {
        "text/plain" => "public.utf8-plain-text",
        RTF | "application/rtf" => "public.rtf",
        "image/png" => "public.png",
        "image/tiff" => "public.tiff",
        "text/uri-list" => "public.file-url",
        _ => format,
    }
}

/// Map a macOS pasteboard type back to a MIME type.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn mime_for_uti(uti: &str) -> &str {
    match uti {
        "public.utf8-plain-text" => "text/plain",
        "public.rtf" => RTF,
        "public.html" => "text/html",
        "public.png" => "image/png",
        "public.tiff" => "image/tiff",
        "public.file-url" => "text/uri-list",
        other => other,
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::time::Duration;

    use windows_sys::Win32::{
        Foundation::GlobalFree,
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
                GetClipboardFormatNameW, IsClipboardFormatAvailable, OpenClipboard,
                RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
        },
    };

    use super::{mime_for_windows_name, windows_name_for_mime, MAX_FORMAT_BYTES};

    const CF_BITMAP: u32 = 2;
    const CF_DIB: u32 = 8;
    const CF_UNICODETEXT: u32 = 13;
    const CF_HDROP: u32 = 15;
    const CF_DIBV5: u32 = 17;
    /// First id of a registered (named) format.
    const FIRST_REGISTERED: u32 = 0xC000;

    /// Open clipboard session, closed on drop. Retries while another process
    /// holds the clipboard.
    struct Session;

    impl Session {
        fn open(op: &str) -> Result<Self, String> {
            for _ in 0..20 {
                // SAFETY: plain Win32 call; a null owner is allowed.
                if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                    return Ok(Self);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(format!(
                "clipboard.{op}: the clipboard is in use by another application"
            ))
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            // SAFETY: the clipboard was opened by `Session::open`.
            unsafe {
                CloseClipboard();
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Clipboard format id for a MIME type or format name.
    fn format_id(format: &str, op: &str) -> Result<u32, String> {
        if format.eq_ignore_ascii_case("text/plain") {
            return Ok(CF_UNICODETEXT);
        }
        let name = wide(windows_name_for_mime(format));
        // SAFETY: `name` is a NUL-terminated UTF-16 string.
        let id = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
        if id == 0 {
            return Err(format!(
                "clipboard.{op}: cannot register clipboard format '{format}'"
            ));
        }
        Ok(id)
    }

    fn registered_name(id: u32) -> Option<String> {
        let mut buf = [0_u16; 256];
        // SAFETY: `buf` holds 256 UTF-16 units.
        let len = unsafe { GetClipboardFormatNameW(id, buf.as_mut_ptr(), 256) };
        let len = usize::try_from(len).ok().filter(|&len| len > 0)?;
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    pub(in crate::api::clipboard) fn available_formats() -> Result<Vec<String>, String> {
        let _session = Session::open("availableFormats")?;
        let mut formats: Vec<String> = Vec::new();
        let mut push = |format: String| {
            if !formats.contains(&format) {
                formats.push(format);
            }
        };
        let mut id = 0;
        loop {
            // SAFETY: the clipboard is open.
            id = unsafe { EnumClipboardFormats(id) };
            if id == 0 {
                break;
            }
            match id {
                CF_UNICODETEXT => push("text/plain".to_owned()),
                CF_BITMAP | CF_DIB | CF_DIBV5 => push("image/bmp".to_owned()),
                CF_HDROP => push("text/uri-list".to_owned()),
                id if id >= FIRST_REGISTERED => {
                    if let Some(name) = registered_name(id) {
                        push(mime_for_windows_name(&name).to_owned());
                    }
                },
                // Synthesized aliases (CF_TEXT, CF_LOCALE...) add nothing.
                _ => {},
            }
        }
        Ok(formats)
    }

    pub(in crate::api::clipboard) fn read_format(
        format: &str,
        op: &str,
    ) -> Result<Option<Vec<u8>>, String> {
        let id = format_id(format, op)?;
        let _session = Session::open(op)?;
        // SAFETY: the clipboard is open.
        if unsafe { IsClipboardFormatAvailable(id) } == 0 {
            return Ok(None);
        }
        // SAFETY: the clipboard is open; the handle stays owned by it.
        let handle = unsafe { GetClipboardData(id) };
        if handle.is_null() {
            return Ok(None);
        }
        // SAFETY: clipboard data handles of these formats are HGLOBALs.
        let size = unsafe { GlobalSize(handle) };
        if size > MAX_FORMAT_BYTES {
            return Err(format!(
                "clipboard.{op}: '{format}' payload exceeds the {MAX_FORMAT_BYTES}-byte cap"
            ));
        }
        // SAFETY: as above; unlocked right after the copy.
        let ptr = unsafe { GlobalLock(handle) };
        if ptr.is_null() {
            return Err(format!("clipboard.{op}: cannot lock '{format}' data"));
        }
        // SAFETY: `ptr` points at `size` readable bytes while locked.
        let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), size) }.to_vec();
        // SAFETY: matches the `GlobalLock` above.
        unsafe {
            GlobalUnlock(handle);
        }
        Ok(Some(if id == CF_UNICODETEXT {
            utf16_bytes_to_utf8(&bytes)
        } else {
            bytes
        }))
    }

    /// UTF-16LE clipboard text (NUL-terminated) as UTF-8 bytes.
    fn utf16_bytes_to_utf8(bytes: &[u8]) -> Vec<u8> {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .take_while(|&unit| unit != 0)
            .collect();
        String::from_utf16_lossy(&units).into_bytes()
    }

    pub(in crate::api::clipboard) fn write_formats(
        items: &[(String, Vec<u8>)],
        op: &str,
    ) -> Result<(), String> {
        let ids = items
            .iter()
            .map(|(format, _)| format_id(format, op))
            .collect::<Result<Vec<_>, _>>()?;
        let _session = Session::open(op)?;
        // SAFETY: the clipboard is open.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(format!("clipboard.{op}: cannot empty the clipboard"));
        }
        for ((format, data), id) in items.iter().zip(ids) {
            let payload = if id == CF_UNICODETEXT {
                String::from_utf8_lossy(data)
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .flat_map(u16::to_le_bytes)
                    .collect()
            } else {
                data.clone()
            };
            set_data(id, &payload).map_err(|e| format!("clipboard.{op}: '{format}': {e}"))?;
        }
        Ok(())
    }

    fn set_data(id: u32, payload: &[u8]) -> Result<(), &'static str> {
        // SAFETY: plain allocation; freed below unless the clipboard takes it.
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, payload.len().max(1)) };
        if handle.is_null() {
            return Err("out of memory");
        }
        // SAFETY: `handle` is a fresh movable block of at least `payload.len()` bytes.
        let ptr = unsafe { GlobalLock(handle) };
        if ptr.is_null() {
            // SAFETY: `handle` is still owned by us.
            unsafe { GlobalFree(handle) };
            return Err("cannot lock memory");
        }
        // SAFETY: `ptr` is writable for `payload.len()` bytes while locked.
        unsafe {
            std::ptr::copy_nonoverlapping(payload.as_ptr(), ptr.cast::<u8>(), payload.len());
            GlobalUnlock(handle);
        }
        // SAFETY: the clipboard is open (caller); on success it owns `handle`.
        if unsafe { SetClipboardData(id, handle) }.is_null() {
            // SAFETY: ownership did not transfer.
            unsafe { GlobalFree(handle) };
            return Err("SetClipboardData failed");
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSData, NSString};

    use super::{mime_for_uti, uti_for_mime, MAX_FORMAT_BYTES};

    pub(in crate::api::clipboard) fn available_formats() -> Result<Vec<String>, String> {
        let pasteboard = NSPasteboard::generalPasteboard();
        let Some(types) = pasteboard.types() else {
            return Ok(Vec::new());
        };
        let mut formats: Vec<String> = Vec::new();
        for uti in types.iter() {
            let format = mime_for_uti(&uti.to_string()).to_owned();
            if !formats.contains(&format) {
                formats.push(format);
            }
        }
        Ok(formats)
    }

    pub(in crate::api::clipboard) fn read_format(
        format: &str,
        op: &str,
    ) -> Result<Option<Vec<u8>>, String> {
        let pasteboard = NSPasteboard::generalPasteboard();
        let uti = NSString::from_str(uti_for_mime(format));
        let Some(data) = pasteboard.dataForType(&uti) else {
            return Ok(None);
        };
        if data.len() > MAX_FORMAT_BYTES {
            return Err(format!(
                "clipboard.{op}: '{format}' payload exceeds the {MAX_FORMAT_BYTES}-byte cap"
            ));
        }
        Ok(Some(data.to_vec()))
    }

    pub(in crate::api::clipboard) fn write_formats(
        items: &[(String, Vec<u8>)],
        op: &str,
    ) -> Result<(), String> {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        for (format, bytes) in items {
            let uti = NSString::from_str(uti_for_mime(format));
            let data = NSData::with_bytes(bytes);
            if !pasteboard.setData_forType(Some(&data), &uti) {
                return Err(format!("clipboard.{op}: cannot write '{format}'"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_names_are_validated() {
        assert!(validate_format("text/rtf", "readFormat").is_ok());
        assert!(validate_format("com.example.custom", "readFormat").is_ok());
        for bad in ["", "a\nb", &"x".repeat(257)] {
            let e = validate_format(bad, "readFormat").unwrap_err();
            assert!(e.contains("invalid format"), "{e}");
        }
        let e = validate_format("TEXT/HTML", "writeFormat").unwrap_err();
        assert!(e.contains("readHtml / writeHtml"), "{e}");
    }

    #[test]
    fn file_reference_formats_cannot_be_written() {
        for format in [
            "CF_HDROP",
            "filenamew",
            "Shell IDList Array",
            "public.file-url",
            "NSFilenamesPboardType",
            "TEXT/URI-LIST",
            // Dynamic UTIs alias legacy types (this one is NSFilenamesPboardType).
            "dyn.ah62d4rv4gu8yc6durvwwaznwmuuha2pxsvw0e55bsmwca7d3sbwu",
            "DYN.anything",
            "com.apple.pasteboard.promised-file-url",
            "COM.APPLE.PASTEBOARD.PROMISED-FILE-NAME",
            "UniformResourceLocatorW",
            "x-special/nautilus-clipboard",
            "application/x-kde-cutselection",
        ] {
            let e = validate_writable_format(format, "writeFormats").unwrap_err();
            assert!(e.contains("use writeFiles"), "{format}: {e}");
            // Reading them stays allowed.
            assert!(validate_format(format, "readFormat").is_ok(), "{format}");
        }
        assert!(validate_writable_format("text/rtf", "writeFormats").is_ok());
        assert!(validate_writable_format("application/x-custom", "writeFormats").is_ok());
        // Prefixes only match at the start, and a short name is not a panic.
        for format in [
            "com.example.dyn.custom",
            "dyn",
            "d",
            "public.utf8-plain-text",
        ] {
            assert!(
                validate_writable_format(format, "writeFormats").is_ok(),
                "{format}"
            );
        }
    }

    #[test]
    fn mime_types_map_to_native_names_and_back() {
        assert_eq!(windows_name_for_mime("text/rtf"), "Rich Text Format");
        assert_eq!(windows_name_for_mime("image/png"), "PNG");
        assert_eq!(windows_name_for_mime("x-custom"), "x-custom");
        assert_eq!(mime_for_windows_name("Rich Text Format"), "text/rtf");
        assert_eq!(uti_for_mime("text/rtf"), "public.rtf");
        assert_eq!(mime_for_uti("public.rtf"), "text/rtf");
        assert_eq!(mime_for_uti("com.example.x"), "com.example.x");
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "touches the real system clipboard"]
    fn custom_format_round_trip() {
        write_formats(
            &[
                ("application/x-tynd-test".to_owned(), vec![1, 2, 3]),
                ("text/plain".to_owned(), b"plain".to_vec()),
            ],
            "writeFormat",
        )
        .unwrap();
        assert_eq!(
            read_format("application/x-tynd-test", "readFormat").unwrap(),
            Some(vec![1, 2, 3])
        );
        assert_eq!(
            read_format("text/plain", "readFormat").unwrap(),
            Some(b"plain".to_vec())
        );
        let formats = available_formats().unwrap();
        assert!(
            formats.iter().any(|f| f == "application/x-tynd-test"),
            "{formats:?}"
        );
        assert!(formats.iter().any(|f| f == "text/plain"), "{formats:?}");
    }
}

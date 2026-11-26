use std::path::{Path, PathBuf};

pub(crate) fn slug_ascii(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pending_dash = false;

    for byte in input.bytes() {
        let byte = byte.to_ascii_lowercase();
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            out.push(char::from(byte));
            pending_dash = false;
        } else if !out.is_empty() {
            pending_dash = true;
        }
    }

    out
}

#[cfg(any(feature = "api-compute", feature = "api-http"))]
pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(char::from(HEX[(byte >> 4) as usize]));
        out.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    out
}

pub(crate) fn collect_files_recursive(base: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect_files_recursive_into(base, &mut out)?;
    Ok(out)
}

fn collect_files_recursive_into(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_file() {
            out.push(path);
        } else if file_type.is_dir() {
            collect_files_recursive_into(&path, out)?;
        }
    }
    Ok(())
}

pub(crate) fn mime_for_path(path: &Path) -> &'static str {
    let lookup = strip_encoded_suffix(path);
    match lookup
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html",
        Some("css") => "text/css",
        Some("js" | "mjs" | "cjs" | "ts" | "tsx") => "application/javascript",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("txt" | "text" | "md") => "text/plain",
        Some("xml") => "application/xml",
        Some("wasm") => "application/wasm",
        Some("png") => "image/png",
        Some("jpg" | "jpeg" | "jfif" | "pjpeg" | "pjp") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico" | "cur") => "image/x-icon",
        Some("bmp") => "image/bmp",
        Some("pdf") => "application/pdf",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        _ => "application/octet-stream",
    }
}

fn strip_encoded_suffix(path: &Path) -> PathBuf {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("gz" | "zst" | "zstd") => path
            .file_stem()
            .map_or_else(|| path.to_path_buf(), PathBuf::from),
        _ => path.to_path_buf(),
    }
}

pub(crate) fn mime_sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else if bytes.starts_with(b"\0asm") {
        Some("application/wasm")
    } else if bytes.starts_with(b"\0\0\x01\0") {
        Some("image/x-icon")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

#[must_use]
pub(crate) struct Defer<F: FnOnce()> {
    callback: Option<F>,
}

impl<F: FnOnce()> std::fmt::Debug for Defer<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Defer")
            .field("active", &self.callback.is_some())
            .finish()
    }
}

impl<F: FnOnce()> Drop for Defer<F> {
    fn drop(&mut self) {
        if let Some(callback) = self.callback.take() {
            callback();
        }
    }
}

pub(crate) fn defer<F: FnOnce()>(callback: F) -> Defer<F> {
    Defer {
        callback: Some(callback),
    }
}

#[cfg(feature = "api-dialog")]
pub(crate) use blocking::block_on;

/// Minimal park/unpark executor for the one-shot futures native dialogs return.
#[cfg(feature = "api-dialog")]
mod blocking {
    use std::{
        future::Future,
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
        thread,
    };

    struct ThreadWaker(thread::Thread);

    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut future = Box::pin(future);

        loop {
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(output) => return output,
                Poll::Pending => thread::park(),
            }
        }
    }
}

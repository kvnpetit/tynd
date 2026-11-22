//! "Open with" / file-association delivery with a cold-start buffer.
//!
//! Files reach the host three ways: macOS `application:openFiles:` (tao
//! `Event::Opened`), argv at launch (Windows / Linux, and a macOS terminal
//! launch), and argv forwarded by a second instance. They can arrive before
//! the frontend has subscribed, so they are held in a process-wide buffer
//! until the first `app.onOpenFile` subscription drains it. From then on
//! every file is emitted live as `app:open-file { path }`.

use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

use parking_lot::Mutex;
use serde_json::{json, Value};
use url::Url;

/// Upper bound on files held before the frontend subscribes. A launcher that
/// opens thousands of files must not grow host memory without limit.
const MAX_PENDING: usize = 256;

#[derive(Debug, Default)]
pub(super) struct OpenFileBuffer {
    pending: Vec<String>,
    subscribed: bool,
}

impl OpenFileBuffer {
    pub(super) fn with_pending(pending: Vec<String>) -> Self {
        let mut buffer = Self::default();
        buffer.buffer(pending);
        buffer
    }

    fn buffer(&mut self, paths: Vec<String>) {
        for path in paths {
            if self.pending.len() >= MAX_PENDING {
                self.pending.remove(0);
            }
            self.pending.push(path);
        }
    }

    /// Route freshly opened files. Returns the files to emit now: everything
    /// once a subscriber exists, nothing while they are being buffered.
    pub(super) fn deliver(&mut self, paths: Vec<String>) -> Vec<String> {
        if self.subscribed {
            paths
        } else {
            self.buffer(paths);
            Vec::new()
        }
    }

    /// Mark the frontend as subscribed and hand back everything buffered so
    /// far, oldest first. Later calls return an empty list.
    pub(super) fn subscribe(&mut self) -> Vec<String> {
        self.subscribed = true;
        std::mem::take(&mut self.pending)
    }
}

static BUFFER: LazyLock<Mutex<OpenFileBuffer>> =
    LazyLock::new(|| Mutex::new(OpenFileBuffer::with_pending(startup_files())));

/// Files named on this process's own command line. macOS hands documents over
/// through `Event::Opened` instead, so argv is only scanned elsewhere.
fn startup_files() -> Vec<String> {
    if cfg!(target_os = "macos") {
        return Vec::new();
    }
    let cwd = std::env::current_dir().ok();
    argv_file_paths(std::env::args().skip(1), cwd.as_deref(), Path::is_file)
}

/// Host flags whose next argv entry is their value, not a document.
const VALUE_FLAGS: &[&str] = &["-e", "--backend-entry"];

/// Keep the argv entries that name an existing file. Flags (and the value of a
/// value-taking host flag such as `--backend-entry <path>`) and URLs are
/// skipped, relative paths resolve against `cwd` (the launching directory).
pub(super) fn argv_file_paths<I, F>(args: I, cwd: Option<&Path>, is_file: F) -> Vec<String>
where
    I: IntoIterator<Item = String>,
    F: Fn(&Path) -> bool,
{
    let mut skip_value = false;
    args.into_iter()
        .filter(|arg| {
            if std::mem::take(&mut skip_value) {
                return false;
            }
            if VALUE_FLAGS.contains(&arg.as_str()) {
                skip_value = true;
                return false;
            }
            !arg.is_empty() && !arg.starts_with('-') && !arg.contains("://")
        })
        .filter_map(|arg| {
            let path = PathBuf::from(arg);
            let resolved = match cwd {
                Some(cwd) if path.is_relative() => cwd.join(path),
                _ => path,
            };
            is_file(&resolved).then(|| resolved.to_string_lossy().into_owned())
        })
        .collect()
}

/// Split the URLs of a tao `Event::Opened` into local file paths and other
/// URLs (custom-scheme deep links also arrive through this event on macOS).
pub(super) fn split_opened(urls: &[Url]) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut others = Vec::new();
    for url in urls {
        if url.scheme() == "file" {
            match url.to_file_path() {
                Ok(path) => files.push(path.to_string_lossy().into_owned()),
                Err(()) => others.push(url.as_str().to_owned()),
            }
        } else {
            others.push(url.as_str().to_owned());
        }
    }
    (files, others)
}

/// Deliver files to the frontend, or buffer them until it subscribes.
pub(super) fn deliver_files(paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    let ready = BUFFER.lock().deliver(paths);
    for path in ready {
        // Local file paths must never reach remote-content child webviews.
        crate::ipc::events::emit_scoped(
            "app:open-file",
            &json!({ "path": path }),
            crate::ipc::events::EventScope::Local,
        );
    }
}

/// Deliver non-file URLs through the existing `app:open-url` event. Only URLs
/// passing the same `looks_like_url` filter as the argv path are forwarded.
pub(super) fn deliver_urls(urls: &[String]) {
    for url in urls
        .iter()
        .filter(|url| crate::webview::navigation::looks_like_url(url))
    {
        crate::ipc::events::emit_scoped(
            "app:open-url",
            &json!({ "url": url }),
            crate::ipc::events::EventScope::Local,
        );
    }
}

/// Backs `app.subscribeOpenFiles`: flips the buffer to live delivery and
/// returns the files opened before the subscription, oldest first.
pub(super) fn subscribe() -> Value {
    json!({ "paths": BUFFER.lock().subscribe() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn buffers_until_first_subscription_then_goes_live() {
        let mut buffer = OpenFileBuffer::default();
        assert_eq!(buffer.deliver(strings(&["a.txt"])), strings(&[]));
        assert_eq!(buffer.deliver(strings(&["b.txt", "c.txt"])), strings(&[]));

        assert_eq!(buffer.subscribe(), strings(&["a.txt", "b.txt", "c.txt"]));
        assert_eq!(buffer.deliver(strings(&["d.txt"])), strings(&["d.txt"]));
        assert!(buffer.subscribe().is_empty(), "drained exactly once");
    }

    #[test]
    fn seeded_files_are_delivered_on_first_subscription() {
        let mut buffer = OpenFileBuffer::with_pending(strings(&["startup.txt"]));
        assert_eq!(buffer.subscribe(), strings(&["startup.txt"]));
    }

    #[test]
    fn pending_is_bounded_and_keeps_the_newest() {
        let mut buffer = OpenFileBuffer::default();
        for index in 0..(MAX_PENDING + 10) {
            buffer.deliver(vec![format!("f{index}")]);
        }
        let drained = buffer.subscribe();
        assert_eq!(drained.len(), MAX_PENDING);
        assert_eq!(drained.first().map(String::as_str), Some("f10"));
        assert_eq!(
            drained.last().map(String::as_str),
            Some(format!("f{}", MAX_PENDING + 9).as_str())
        );
    }

    #[test]
    fn argv_keeps_existing_files_only() {
        let args = strings(&["--flag", "myapp://x", "", "doc.txt", "missing.txt", "-v"]);
        let cwd = PathBuf::from("base");
        let found = argv_file_paths(args, Some(&cwd), |path| path.ends_with("doc.txt"));
        assert_eq!(
            found,
            vec![cwd.join("doc.txt").to_string_lossy().into_owned()]
        );
    }

    #[test]
    fn argv_skips_values_of_host_flags() {
        let args = strings(&[
            "--backend-entry",
            "/abs/backend/main.ts",
            "-e",
            "other.ts",
            "--backend-entry=eq.ts",
            "doc.txt",
        ]);
        let found = argv_file_paths(args, None, |_| true);
        assert_eq!(found, strings(&["doc.txt"]));
    }

    #[test]
    fn open_urls_are_filtered_like_the_argv_path() {
        assert!(crate::webview::navigation::looks_like_url(
            "https://example.com/a"
        ));
        assert!(!crate::webview::navigation::looks_like_url(
            "javascript:alert(1)"
        ));
        assert!(!crate::webview::navigation::looks_like_url(
            "data:text/html,x"
        ));
    }

    #[test]
    fn custom_scheme_deep_links_are_accepted_but_paths_are_not() {
        let looks = crate::webview::navigation::looks_like_url;
        assert!(looks("myapp://x"));
        assert!(looks("myapp://open?id=1"));
        assert!(looks("https://x"));
        assert!(!looks("C:\\a"));
        assert!(!looks("C://a"));
        assert!(!looks("javascript://%0Aalert(1)"));
        assert!(!looks("doc.txt"));
        assert!(!looks("1app://x"));
        assert!(!looks("my app://x"));
        assert!(!looks("://x"));
    }

    #[test]
    fn argv_does_not_rebase_absolute_paths() {
        let absolute = std::env::temp_dir().join("doc.txt");
        let args = vec![absolute.to_string_lossy().into_owned()];
        let found = argv_file_paths(args, Some(Path::new("elsewhere")), |_| true);
        assert_eq!(found, vec![absolute.to_string_lossy().into_owned()]);
    }

    #[test]
    fn opened_urls_split_into_files_and_others() {
        let file_url = Url::from_file_path(std::env::temp_dir().join("a b.txt")).unwrap();
        let deep_link = Url::parse("myapp://open?id=1").unwrap();
        let (files, others) = split_opened(&[file_url, deep_link]);
        assert_eq!(
            files,
            vec![std::env::temp_dir()
                .join("a b.txt")
                .to_string_lossy()
                .into_owned()]
        );
        assert_eq!(others, strings(&["myapp://open?id=1"]));
    }
}

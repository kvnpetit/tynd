use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        LazyLock, Mutex, PoisonError,
    },
    time::Instant,
};

use serde_json::{json, Value};
use wry::{
    cookie::{Cookie, SameSite},
    DragDropEvent, NewWindowFeatures, NewWindowResponse, PageLoadEvent, ProxyConfig, ProxyEndpoint,
    WebViewBuilder,
};

use crate::ipc::events::EventScope;
use crate::webview::navigation::{check_navigation_url, remote_origin};

// ---------------------------------------------------------------------------
// App-local content: what plugin hooks, events and downloads key off.
// ---------------------------------------------------------------------------

/// `true` when `url` is app content: the bundled `tynd://` scheme, its
/// WebView2 `http://tynd.localhost` rewrite, the configured dev server
/// origin (`tynd dev` only), or the transient `about:blank`. Everything else,
/// any other loopback origin included, is remote content.
pub(crate) fn is_app_local_url(url: &str) -> bool {
    is_app_local_url_with(url, crate::webview::navigation::dev_origin().as_deref())
}

fn is_app_local_url_with(url: &str, dev_origin: Option<&str>) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    match parsed.scheme() {
        "tynd" => true,
        "about" => url == "about:blank",
        "http" | "https" => {
            // Exact WebView2 rewrite origin only (Windows): a different port
            // or `https` is not intercepted and reaches loopback.
            (cfg!(target_os = "windows")
                && crate::webview::navigation::is_webview2_app_origin(&parsed))
                || dev_origin == Some(parsed.origin().ascii_serialization().as_str())
        },
        _ => false,
    }
}

/// Whether the webview labelled `label` currently shows app-local content
/// (it has not navigated to a remote origin).
pub(crate) fn is_app_local_webview(label: &str) -> bool {
    remote_origin(label).is_none()
}

/// Build a DragDrop handler bound to a window label. Emits `window:drag-enter`,
/// `window:drag-over`, `window:drag-leave`, `window:drop` with paths + cursor
/// position. Returns `false` so the WebView still runs default HTML5 DnD.
pub(crate) fn drag_drop_handler(label: &str) -> impl Fn(DragDropEvent) -> bool + 'static {
    let label = label.to_string();
    move |evt| {
        match evt {
            DragDropEvent::Enter { paths, position } => {
                let paths: Vec<String> = paths
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                // Same paths as `window:drop`: local webviews only.
                crate::ipc::events::emit_scoped(
                    "window:drag-enter",
                    &json!({ "label": label, "paths": paths, "x": position.0, "y": position.1 }),
                    EventScope::Origin(label.clone()),
                );
            },
            DragDropEvent::Over { position } => {
                crate::ipc::events::emit(
                    "window:drag-over",
                    &json!({ "label": label, "x": position.0, "y": position.1 }),
                );
            },
            DragDropEvent::Drop { paths, position } => {
                let paths: Vec<String> = paths
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                // Local file paths: never delivered to remote-content webviews.
                crate::ipc::events::emit_scoped(
                    "window:drop",
                    &json!({ "label": label, "paths": paths, "x": position.0, "y": position.1 }),
                    EventScope::Origin(label.clone()),
                );
            },
            DragDropEvent::Leave => {
                crate::ipc::events::emit("window:drag-leave", &json!({ "label": label }));
            },
            _ => {},
        }
        false
    }
}

/// Page-load observer: fires once when the document starts loading and once
/// when it finishes. Frontend listens via `tyndWindow.onPageLoad`.
pub(crate) fn page_load_handler(label: &str) -> impl Fn(PageLoadEvent, String) + 'static {
    let label = label.to_string();
    move |event, url| {
        let phase = match event {
            PageLoadEvent::Started => {
                // Started is the commit of a new main-frame document on every
                // platform: the only point where app-local trust is granted.
                crate::webview::navigation::document_committed(&label, &url);
                "started"
            },
            PageLoadEvent::Finished => "finished",
        };
        crate::ipc::events::emit_scoped(
            "webview:page-load",
            &json!({ "label": label, "phase": phase, "url": url }),
            EventScope::Origin(label.clone()),
        );
    }
}

// ---------------------------------------------------------------------------
// Webview policy: new-window requests, downloads, proxy.
// ---------------------------------------------------------------------------

/// What to do with `window.open()` / `target="_blank"` requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum NewWindowPolicy {
    /// Block every request.
    Deny,
    /// Open `http` / `https` / `mailto` URLs in the system browser.
    External,
    /// Let the platform open its default popup, but only for URLs that pass
    /// the navigation policy (default: the behaviour before policies existed).
    #[default]
    Allow,
}

impl NewWindowPolicy {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "deny" => Some(Self::Deny),
            "external" => Some(Self::External),
            "allow" => Some(Self::Allow),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NewWindowDecision {
    Deny,
    OpenExternal(String),
    AllowDefault,
}

impl NewWindowDecision {
    fn action(&self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::OpenExternal(_) => "external",
            Self::AllowDefault => "allow",
        }
    }
}

/// Schemes the `External` policy hands to the OS. Kept identical to
/// `shell.openExternal` so a page cannot reach protocol handlers through
/// `window.open`.
fn is_external_openable(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    match parsed.scheme() {
        "http" | "https" => parsed.host_str().is_some_and(|host| !host.is_empty()),
        "mailto" => true,
        _ => false,
    }
}

/// Pure policy decision for a new-window request.
pub(crate) fn decide_new_window(policy: NewWindowPolicy, url: &str) -> NewWindowDecision {
    match policy {
        NewWindowPolicy::External if is_external_openable(url) => {
            NewWindowDecision::OpenExternal(url.to_owned())
        },
        NewWindowPolicy::Allow if check_navigation_url(url).is_ok() => {
            NewWindowDecision::AllowDefault
        },
        NewWindowPolicy::Deny | NewWindowPolicy::External | NewWindowPolicy::Allow => {
            NewWindowDecision::Deny
        },
    }
}

/// Minimum gap between two page-initiated external opens. A hostile page can
/// call `window.open` in a loop; this keeps it from flooding the desktop with
/// browser tabs.
const EXTERNAL_OPEN_MIN_GAP_MS: u64 = 300;

static PROCESS_START: LazyLock<Instant> = LazyLock::new(Instant::now);
static LAST_EXTERNAL_OPEN_MS: AtomicU64 = AtomicU64::new(0);

/// `true` when an external open at `now_ms` is outside the throttle window.
/// Records the attempt on success. `now_ms` must be non-zero.
fn external_open_permitted(now_ms: u64, last: &AtomicU64) -> bool {
    let previous = last.load(Ordering::Relaxed);
    if previous != 0 && now_ms.saturating_sub(previous) < EXTERNAL_OPEN_MIN_GAP_MS {
        return false;
    }
    last.store(now_ms, Ordering::Relaxed);
    true
}

fn open_in_system_browser(url: String) {
    let now_ms = u64::try_from(PROCESS_START.elapsed().as_millis())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    if !external_open_permitted(now_ms, &LAST_EXTERNAL_OPEN_MS) {
        crate::tynd_warn!("new-window: external open throttled");
        return;
    }
    #[cfg(feature = "api-shell")]
    {
        let submitted = crate::ipc::call_pool::submit(move || {
            if let Err(error) = crate::api::shell::ShellApi::open_external(url) {
                crate::tynd_warn!("new-window: open external failed: {error}");
            }
        });
        if submitted.is_err() {
            crate::tynd_warn!("new-window: call pool saturated, external open dropped");
        }
    }
    #[cfg(not(feature = "api-shell"))]
    {
        let _ = url;
        crate::tynd_warn!("new-window: `api-shell` is disabled, external open dropped");
    }
}

/// Handler for wry's new-window request hook. Emits `webview:new-window`
/// with the decision so the app can observe blocked or redirected popups.
pub(crate) fn new_window_handler(
    label: &str,
    policy: NewWindowPolicy,
) -> impl Fn(String, NewWindowFeatures) -> NewWindowResponse + 'static {
    let label = label.to_string();
    move |url, _features| {
        let decision = decide_new_window(policy, &url);
        // Popup URLs can carry OAuth codes: never broadcast them to remote pages.
        crate::ipc::events::emit_scoped(
            "webview:new-window",
            &json!({ "label": label, "url": url, "action": decision.action() }),
            EventScope::Origin(label.clone()),
        );
        match decision {
            NewWindowDecision::Deny => NewWindowResponse::Deny,
            NewWindowDecision::AllowDefault => NewWindowResponse::Allow,
            NewWindowDecision::OpenExternal(target) => {
                open_in_system_browser(target);
                NewWindowResponse::Deny
            },
        }
    }
}

const MAX_DOWNLOAD_NAME_CHARS: usize = 200;
const MAX_UNIQUE_SUFFIX: u32 = 999;
const MAX_TRACKED_DOWNLOADS: usize = 256;

/// Unicode format (Cf), line/paragraph separator and other invisible code
/// points that can reorder or hide part of a file name (a right-to-left
/// override turns `invoice<U+202E>gpj.exe` into `invoiceexe.jpg` on screen).
fn is_invisible_or_bidi(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{08E2}'
            | '\u{115F}'..='\u{1160}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// Reduce a suggested download name to a single safe path component.
pub(crate) fn sanitize_download_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| {
            !c.is_control()
                && !is_invisible_or_bidi(*c)
                && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == ' ' || c == '.');
    let mut name: String = trimmed.chars().take(MAX_DOWNLOAD_NAME_CHARS).collect();
    if name.is_empty() {
        return "download".to_owned();
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix)
                .is_some_and(|digit| matches!(digit.as_bytes(), [b'1'..=b'9']))
        });
    if reserved {
        name.insert(0, '_');
    }
    name
}

/// First free `name`, `name (1)`, `name (2)`, ... inside `dir`.
pub(crate) fn unique_destination(
    dir: &Path,
    name: &str,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let first = dir.join(name);
    if !exists(&first) {
        return Some(first);
    }
    let as_path = Path::new(name);
    let stem = as_path.file_stem()?.to_string_lossy().into_owned();
    let extension = as_path
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    (1..=MAX_UNIQUE_SUFFIX)
        .map(|n| dir.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| !exists(candidate))
}

/// Final on-disk target for a download: always a fresh file directly inside
/// the OS downloads directory. `suggested` only contributes its file name.
pub(crate) fn download_destination(
    suggested: &Path,
    downloads_dir: Option<PathBuf>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let dir = downloads_dir.or_else(|| {
        suggested
            .parent()
            .filter(|parent| parent.is_absolute())
            .map(Path::to_path_buf)
    })?;
    let name = sanitize_download_name(&suggested.file_name()?.to_string_lossy());
    unique_destination(&dir, &name, exists)
}

/// Whether a webview may download files at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DownloadPolicy {
    Allow,
    Deny,
}

impl DownloadPolicy {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// Whether a download URL is app content (`blob:` URLs are judged by the
/// origin they embed). The engine does not tell us which frame started the
/// download and main-frame navigation tracking cannot see a remote iframe, so
/// under the default policy the URL is the only provenance signal: a download
/// whose initiator cannot be shown to be app-local is treated as remote.
/// `data:` URLs carry no provenance at all (a remote iframe inside local
/// content can start one), so they are refused unless the policy is an
/// explicit `allow`.
fn download_url_is_app_local(url: &str) -> bool {
    is_app_local_url(url.strip_prefix("blob:").unwrap_or(url))
}

/// Effective download decision. No explicit policy: app-local content may
/// download app-local resources, remote or unattributable ones may not.
fn downloads_permitted(
    policy: Option<DownloadPolicy>,
    content_local: bool,
    download_url: &str,
) -> bool {
    match policy {
        Some(DownloadPolicy::Allow) => true,
        Some(DownloadPolicy::Deny) => false,
        None => content_local && download_url_is_app_local(download_url),
    }
}

const MAX_RESERVED_DESTINATIONS: usize = 1_024;

/// Destinations handed to in-flight downloads. The engine writes the file
/// itself (and some engines refuse to overwrite a pre-created one), so the
/// uniqueness check and the claim happen together under one lock instead:
/// two concurrent downloads of the same name can never get the same path.
#[derive(Default)]
struct DownloadReservations {
    taken: Mutex<HashSet<PathBuf>>,
}

impl DownloadReservations {
    fn reserve(
        &self,
        suggested: &Path,
        downloads_dir: Option<PathBuf>,
        exists: impl Fn(&Path) -> bool,
    ) -> Option<PathBuf> {
        let mut taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
        if taken.len() >= MAX_RESERVED_DESTINATIONS {
            taken.clear();
        }
        let destination =
            download_destination(suggested, downloads_dir, |p| taken.contains(p) || exists(p))?;
        taken.insert(destination.clone());
        Some(destination)
    }

    fn release(&self, path: &Path) {
        self.taken
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(path);
    }
}

static RESERVATIONS: LazyLock<DownloadReservations> = LazyLock::new(DownloadReservations::default);

static NEXT_DOWNLOAD_ID: AtomicU64 = AtomicU64::new(1);

type PendingDownloads = Rc<RefCell<HashMap<String, VecDeque<(u64, PathBuf)>>>>;
type DownloadStartedFn = Box<dyn FnMut(String, &mut PathBuf) -> bool + 'static>;
type DownloadCompletedFn = Box<dyn Fn(String, Option<PathBuf>, bool) + 'static>;

/// Download handlers for one webview. The started handler refuses the download
/// when the policy denies it, otherwise redirects it into the OS downloads
/// directory and emits `webview:download-started`; the completed handler emits
/// `webview:download-completed`. Both carry the same `id`, and both events go
/// only to this webview (when app-local) or to app-local webviews.
pub(crate) fn download_handlers(
    label: &str,
    policy: Option<DownloadPolicy>,
) -> (DownloadStartedFn, DownloadCompletedFn) {
    let pending: PendingDownloads = Rc::new(RefCell::new(HashMap::new()));
    let started_label = label.to_string();
    let completed_label = label.to_string();
    let started_pending = Rc::clone(&pending);

    let started = move |url: String, path: &mut PathBuf| {
        start_download(&started_label, policy, &started_pending, url, path)
    };

    let completed = move |url: String, path: Option<PathBuf>, success: bool| {
        let entry = pending
            .borrow_mut()
            .get_mut(&url)
            .and_then(VecDeque::pop_front);
        let id = entry.as_ref().map_or_else(
            || NEXT_DOWNLOAD_ID.fetch_add(1, Ordering::Relaxed),
            |(id, _)| *id,
        );
        if let Some((_, reserved)) = &entry {
            RESERVATIONS.release(reserved);
        }
        pending.borrow_mut().retain(|_, ids| !ids.is_empty());
        crate::ipc::events::emit_scoped(
            "webview:download-completed",
            &json!({
                "label": completed_label,
                "id": id,
                "url": url,
                "path": path.as_deref().map(|p| p.to_string_lossy().into_owned()),
                "success": success,
            }),
            EventScope::Origin(completed_label.clone()),
        );
    };

    (Box::new(started), Box::new(completed))
}

/// Started-handler body of [`download_handlers`]: `false` refuses the
/// download, `true` accepts it with `path` redirected to its reserved
/// destination.
fn start_download(
    label: &str,
    policy: Option<DownloadPolicy>,
    pending: &PendingDownloads,
    url: String,
    path: &mut PathBuf,
) -> bool {
    if !downloads_permitted(policy, is_app_local_webview(label), &url) {
        crate::tynd_warn!("download from {url:?} denied by the webview download policy");
        return false;
    }
    let Some(destination) = RESERVATIONS.reserve(path, dirs::download_dir(), Path::exists) else {
        crate::tynd_warn!("download from {url:?} denied: no usable downloads directory");
        return false;
    };
    if let Some(parent) = destination.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            RESERVATIONS.release(&destination);
            crate::tynd_warn!("download from {url:?} denied: create dir failed: {error}");
            return false;
        }
    }
    let id = NEXT_DOWNLOAD_ID.fetch_add(1, Ordering::Relaxed);
    {
        let mut map = pending.borrow_mut();
        if map.len() >= MAX_TRACKED_DOWNLOADS {
            for (_, entries) in map.drain() {
                for (_, reserved) in entries {
                    RESERVATIONS.release(&reserved);
                }
            }
        }
        map.entry(url.clone())
            .or_default()
            .push_back((id, destination.clone()));
    }
    crate::ipc::events::emit_scoped(
        "webview:download-started",
        &json!({
            "label": label,
            "id": id,
            "url": url,
            "path": destination.to_string_lossy(),
        }),
        EventScope::Origin(label.to_owned()),
    );
    *path = destination;
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxyKind {
    Http,
    Socks5,
}

/// Validated proxy endpoint. wry's `ProxyConfig` is not comparable, so the
/// request structs hold this and convert at build time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProxySpec {
    pub(crate) kind: ProxyKind,
    pub(crate) host: String,
    pub(crate) port: String,
}

impl ProxySpec {
    /// Parse `http://host:port` or `socks5://host:port`. Credentials are not
    /// supported by the underlying webview APIs and are rejected.
    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        let parsed =
            url::Url::parse(raw).map_err(|error| format!("invalid proxy URL '{raw}': {error}"))?;
        let kind = match parsed.scheme() {
            "http" => ProxyKind::Http,
            "socks5" => ProxyKind::Socks5,
            other => {
                return Err(format!(
                    "unsupported proxy scheme '{other}': use http:// or socks5://"
                ))
            },
        };
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err("proxy credentials are not supported".into());
        }
        if !matches!(parsed.path(), "" | "/") || parsed.query().is_some() {
            return Err("proxy URL must be scheme://host:port with no path".into());
        }
        let host = parsed
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or_else(|| "proxy URL has no host".to_string())?;
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| "proxy URL needs an explicit port".to_string())?;
        Ok(Self {
            kind,
            host: host.to_owned(),
            port: port.to_string(),
        })
    }

    fn to_wry(&self) -> ProxyConfig {
        let endpoint = ProxyEndpoint {
            host: self.host.clone(),
            port: self.port.clone(),
        };
        match self.kind {
            ProxyKind::Http => ProxyConfig::Http(endpoint),
            ProxyKind::Socks5 => ProxyConfig::Socks5(endpoint),
        }
    }
}

/// Per-webview policy knobs resolved before the builder runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WebviewPolicy {
    pub(crate) new_window: NewWindowPolicy,
    /// `None`: allow for app-local content, deny for remote content.
    pub(crate) downloads: Option<DownloadPolicy>,
    pub(crate) proxy: Option<ProxySpec>,
}

impl WebviewPolicy {
    /// Build from raw config strings. `None` and empty strings select the
    /// defaults (`allow` popups, content-dependent downloads, no proxy).
    pub(crate) fn from_raw(
        new_window: Option<&str>,
        downloads: Option<&str>,
        proxy: Option<&str>,
        operation: &str,
    ) -> Result<Self, String> {
        let new_window = match new_window.filter(|value| !value.is_empty()) {
            None => NewWindowPolicy::default(),
            Some(raw) => NewWindowPolicy::parse(raw).ok_or_else(|| {
                format!("{operation}: newWindow must be \"deny\", \"external\" or \"allow\"")
            })?,
        };
        let downloads = downloads
            .filter(|value| !value.is_empty())
            .map(|raw| {
                DownloadPolicy::parse(raw)
                    .ok_or_else(|| format!("{operation}: downloads must be \"allow\" or \"deny\""))
            })
            .transpose()?;
        let proxy = proxy
            .filter(|value| !value.is_empty())
            .map(|raw| ProxySpec::parse(raw).map_err(|error| format!("{operation}: {error}")))
            .transpose()?;
        Ok(Self {
            new_window,
            downloads,
            proxy,
        })
    }

    /// Read `newWindow` / `downloads` / `proxy` from the arguments of
    /// `window.create` / `webview.create`.
    pub(crate) fn from_args(args: &Value, operation: &str) -> Result<Self, String> {
        Self::from_raw(
            args.get("newWindow").and_then(Value::as_str),
            args.get("downloads").and_then(Value::as_str),
            args.get("proxy").and_then(Value::as_str),
            operation,
        )
    }

    /// Policy of the primary window, from its `newWindow` / `downloads` /
    /// `proxy` config.
    pub(crate) fn for_primary(config: &crate::window::WindowConfig) -> Result<Self, String> {
        Self::from_raw(
            config.new_window.as_deref(),
            config.downloads.as_deref(),
            config.proxy.as_deref(),
            "window",
        )
    }
}

/// Install the new-window, download, proxy and plugin-registry hooks on a
/// builder. `initial_url` is the page the webview opens on; `None` means
/// bundled or dev-server content.
pub(crate) fn apply_webview_policy<'a>(
    builder: WebViewBuilder<'a>,
    label: &str,
    policy: &WebviewPolicy,
    initial_url: Option<&str>,
) -> WebViewBuilder<'a> {
    let (started, completed) = download_handlers(label, policy.downloads);
    let mut builder = builder
        .with_new_window_req_handler(new_window_handler(label, policy.new_window))
        .with_download_started_handler(started)
        .with_download_completed_handler(completed);
    if let Some(proxy) = &policy.proxy {
        builder = builder.with_proxy_config(proxy.to_wry());
    }
    #[cfg(feature = "plugins")]
    let builder = crate::webview::plugin_handlers::apply_plugin_hooks(builder, label, initial_url);
    #[cfg(not(feature = "plugins"))]
    let _ = initial_url;
    builder
}

// ---------------------------------------------------------------------------
// Cookies: JSON <-> wry cookie conversion.
// ---------------------------------------------------------------------------

const MAX_COOKIE_FIELD_BYTES: usize = 4096;

fn valid_cookie_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_COOKIE_FIELD_BYTES
        && !name.chars().any(|c| {
            c.is_control() || c.is_whitespace() || matches!(c, ';' | ',' | '=' | '"' | '\\')
        })
}

fn valid_cookie_value(value: &str) -> bool {
    value.len() <= MAX_COOKIE_FIELD_BYTES && !value.chars().any(|c| c.is_control() || c == ';')
}

fn valid_cookie_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'))
}

fn valid_cookie_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= MAX_COOKIE_FIELD_BYTES
        && !path.chars().any(|c| c.is_control() || c == ';')
}

/// Parse an `http` / `https` URL argument.
pub(crate) fn parse_http_url(raw: &str, operation: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(raw)
        .map_err(|error| format!("{operation}: invalid url '{raw}': {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(format!("{operation}: url must be an http(s) URL"));
    }
    Ok(parsed)
}

pub(crate) fn cookie_to_json(cookie: &Cookie<'_>) -> Value {
    json!({
        "name": cookie.name(),
        "value": cookie.value(),
        "domain": cookie.domain(),
        "path": cookie.path(),
        "secure": cookie.secure().unwrap_or(false),
        "httpOnly": cookie.http_only().unwrap_or(false),
        "sameSite": cookie.same_site().map(|same_site| match same_site {
            SameSite::Strict => "strict",
            SameSite::Lax => "lax",
            SameSite::None => "none",
        }),
        "expires": cookie.expires_datetime().map(wry::cookie::time::OffsetDateTime::unix_timestamp),
    })
}

fn cookie_domain_and_path(
    object: &Value,
    base_url: Option<&url::Url>,
    operation: &str,
) -> Result<(String, String), String> {
    let domain = match object.get("domain").and_then(Value::as_str) {
        Some(domain) if valid_cookie_domain(domain) => domain.to_owned(),
        Some(_) => return Err(format!("{operation}: invalid cookie domain")),
        None => base_url
            .and_then(url::Url::host_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{operation}: cookie needs a 'domain' or a 'url'"))?,
    };
    let path = match object.get("path").and_then(Value::as_str) {
        Some(path) if valid_cookie_path(path) => path.to_owned(),
        Some(_) => return Err(format!("{operation}: invalid cookie path")),
        None => "/".to_owned(),
    };
    Ok((domain, path))
}

/// Build a cookie from the `set` payload. `base_url` supplies the domain
/// when the cookie object omits it.
pub(crate) fn cookie_from_json(
    object: &Value,
    base_url: Option<&url::Url>,
    operation: &str,
) -> Result<Cookie<'static>, String> {
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| valid_cookie_name(name))
        .ok_or_else(|| format!("{operation}: invalid or missing cookie 'name'"))?;
    let value = object
        .get("value")
        .and_then(Value::as_str)
        .filter(|value| valid_cookie_value(value))
        .ok_or_else(|| format!("{operation}: invalid or missing cookie 'value'"))?;
    let (domain, path) = cookie_domain_and_path(object, base_url, operation)?;
    let mut builder = Cookie::build((name.to_owned(), value.to_owned()))
        .domain(domain)
        .path(path);
    if let Some(secure) = object.get("secure").and_then(Value::as_bool) {
        builder = builder.secure(secure);
    }
    if let Some(http_only) = object.get("httpOnly").and_then(Value::as_bool) {
        builder = builder.http_only(http_only);
    }
    match object.get("sameSite").and_then(Value::as_str) {
        None => {},
        Some("strict") => builder = builder.same_site(SameSite::Strict),
        Some("lax") => builder = builder.same_site(SameSite::Lax),
        Some("none") => builder = builder.same_site(SameSite::None),
        Some(_) => {
            return Err(format!(
                "{operation}: sameSite must be \"strict\", \"lax\" or \"none\""
            ))
        },
    }
    if let Some(expires) = object.get("expires").filter(|value| !value.is_null()) {
        let seconds = expires
            .as_i64()
            .ok_or_else(|| format!("{operation}: expires must be integer unix seconds"))?;
        let date = wry::cookie::time::OffsetDateTime::from_unix_timestamp(seconds)
            .map_err(|_| format!("{operation}: expires out of range"))?;
        builder = builder.expires(date);
    }
    Ok(builder.build())
}

/// Identify one cookie for deletion: `name` plus `domain` (or `url`) and
/// `path`.
pub(crate) fn cookie_key_from_json(
    object: &Value,
    base_url: Option<&url::Url>,
    operation: &str,
) -> Result<Cookie<'static>, String> {
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| valid_cookie_name(name))
        .ok_or_else(|| format!("{operation}: invalid or missing cookie 'name'"))?;
    let (domain, path) = cookie_domain_and_path(object, base_url, operation)?;
    Ok(Cookie::build((name.to_owned(), String::new()))
        .domain(domain)
        .path(path)
        .build())
}

/// Origin the `tynd-bin` custom protocol is reachable on. WebView2 cannot
/// navigate to custom schemes, so wry serves it under `http://<scheme>.localhost`
/// there; the client must not guess this from the (app-settable) user agent.
const fn tynd_bin_origin() -> &'static str {
    if cfg!(target_os = "windows") {
        "http://tynd-bin.localhost"
    } else {
        "tynd-bin://localhost"
    }
}

/// Runtime globals shared by production assets and external development servers.
/// The CLI supplies only the resolved public updater configuration.
pub(crate) fn inject_runtime_config() -> String {
    use serialize_to_javascript::{Options, Template};

    const MAX_RUNTIME_CONFIG_BYTES: usize = 64 * 1024;
    let updater = std::env::var("TYND_FRONTEND_RUNTIME_CONFIG")
        .ok()
        .filter(|value| value.len() <= MAX_RUNTIME_CONFIG_BYTES)
        .and_then(|value| match serde_json::from_str::<Value>(&value) {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::warn!(
                    target: "tynd",
                    "ignoring invalid frontend runtime config: {error}"
                );
                None
            },
        })
        .unwrap_or(Value::Null);

    #[derive(Template)]
    struct RuntimeConfigScript<'a> {
        updater: &'a Value,
        bin_origin: &'a str,
    }

    RuntimeConfigScript {
        updater: &updater,
        bin_origin: tynd_bin_origin(),
    }
    .render(
        "globalThis.__TYND_RUNTIME__ = \"bun\";\
         globalThis.__TYND_BIN_ORIGIN__ = __TEMPLATE_bin_origin__;\
         globalThis.__TYND_UPDATER_CONFIG__ = __TEMPLATE_updater__;",
        &Options::default(),
    )
    .map_or_else(
        |_| {
            format!(
                "globalThis.__TYND_RUNTIME__=\"bun\";globalThis.__TYND_BIN_ORIGIN__=\"{}\";\
                 globalThis.__TYND_UPDATER_CONFIG__=null;",
                tynd_bin_origin()
            )
        },
        serialize_to_javascript::Serialized::into_string,
    )
}

/// Init script injected into every webview so frontend code knows both its
/// native parent window and webview identity.
///
/// Uses `serialize_to_javascript` so embedded U+2028 / U+2029, `</script>`,
/// quotes, and other JS-string-terminating characters are escaped at the
/// serialization layer.
pub(crate) fn inject_context_labels(window_label: &str, webview_label: &str) -> String {
    use serialize_to_javascript::{Options, Template};

    #[derive(Template)]
    struct ContextLabelScript<'a> {
        window_label: &'a str,
        webview_label: &'a str,
    }

    let script = ContextLabelScript {
        window_label,
        webview_label,
    };
    match script.render(
        "window.__TYND_WINDOW_LABEL__ = __TEMPLATE_window_label__;\
         window.__TYND_WEBVIEW_LABEL__ = __TEMPLATE_webview_label__;",
        &Options::default(),
    ) {
        Ok(s) => s.into_string(),
        Err(_) => {
            r#"window.__TYND_WINDOW_LABEL__ = "";window.__TYND_WEBVIEW_LABEL__ = "";"#.to_owned()
        },
    }
}

#[cfg(test)]
mod tests;

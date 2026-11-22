//! Connection-accepting + request-routing core of the debug protocol.
//!
//! Single-client by contract. The accept loop runs on a dedicated
//! thread; the second connection that arrives while a primary is
//! attached gets `{ error: { code: -32099, message: "already_attached" } }`
//! framed onto its socket and is then closed.

use std::{
    io::{BufReader, BufWriter, Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::Value;
use subtle::ConstantTimeEq;

use crate::protocol::{encode, read_frame, write_frame, Notification, Request, Response, RpcError};

/// A peer that has not completed the `hello` handshake within this window is
/// dropped, so a stray local client cannot hold the single attach slot forever.
const PRE_AUTH_TIMEOUT: Duration = Duration::from_secs(5);

/// Reader whose read timeout can be changed on the owning handle. The timeout
/// is set on the stream the reader actually reads from: options set on a
/// `try_clone` are not reliably shared across clones on every platform.
pub(crate) trait TimedRead: Read + Send {
    /// Returns whether the transport enforces the timeout (Windows named pipes
    /// do not).
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> bool;

    /// Number of bytes a read would return without blocking, without consuming
    /// them. `None` when the transport cannot tell. `Ok(0)` means "empty, the
    /// peer is still connected"; a closed peer is an error (see
    /// [`is_peer_closed`]). Used to poll against a deadline where read
    /// timeouts are unavailable, so the pipe itself stays in blocking mode.
    fn peek_available(&mut self) -> Option<std::io::Result<usize>> {
        None
    }
}

impl TimedRead for std::net::TcpStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> bool {
        std::net::TcpStream::set_read_timeout(self, timeout).is_ok()
    }
}

#[cfg(feature = "local-socket")]
impl TimedRead for interprocess::local_socket::Stream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> bool {
        use interprocess::local_socket::traits::Stream as _;
        self.set_recv_timeout(timeout).is_ok()
    }

    #[cfg(windows)]
    fn peek_available(&mut self) -> Option<std::io::Result<usize>> {
        use std::os::windows::io::{AsHandle, AsRawHandle};

        use windows_sys::Win32::System::Pipes::PeekNamedPipe;

        let interprocess::local_socket::Stream::NamedPipe(pipe) = self;
        let handle = pipe.as_handle().as_raw_handle();
        let mut available: u32 = 0;
        // SAFETY: `handle` is a live named-pipe handle borrowed from `self`; a
        // null buffer with size 0 only queries the byte count, and `available`
        // outlives the call.
        let ok = unsafe {
            PeekNamedPipe(
                handle.cast(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &raw mut available,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Some(Err(std::io::Error::last_os_error()));
        }
        Some(Ok(available as usize))
    }
}

/// Poll interval while enforcing the deadline on a transport without read
/// timeouts.
const DEADLINE_POLL: Duration = Duration::from_millis(10);

/// Whether a peek/read error means the peer closed the connection (a real EOF),
/// as opposed to a transient or unexpected failure. Windows named pipes report
/// `ERROR_BROKEN_PIPE` (109) or `ERROR_PIPE_NOT_CONNECTED` (233).
fn is_peer_closed(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::BrokenPipe
        || matches!(error.raw_os_error(), Some(109 | 233))
}

/// Wraps the stream with an absolute pre-auth deadline. A per-read socket
/// timeout alone is reset by every byte, so a peer trickling one byte at a time
/// would never expire; this re-arms the timeout with the time *remaining* before
/// each read and fails once the deadline has passed. Transports without read
/// timeouts (Windows named pipes: a blocking read never returns for an idle
/// peer) are polled with a non-consuming peek until the deadline. The pipe is
/// never switched to `PIPE_NOWAIT`: there an empty pipe reads as
/// `ERROR_NO_DATA`, indistinguishable from a closed peer, which dropped slow
/// handshakes as EOF.
pub(crate) struct DeadlineReader {
    inner: Box<dyn TimedRead>,
    deadline: Option<Instant>,
    polling: bool,
}

impl DeadlineReader {
    pub(crate) fn new(mut inner: Box<dyn TimedRead>, deadline: Option<Instant>) -> Self {
        let mut polling = false;
        if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if !inner.set_read_timeout(Some(remaining)) {
                polling = inner.peek_available().is_some();
                if !polling {
                    tracing::warn!(
                        target: "tynd::debug",
                        "debug transport has neither read timeouts nor peek support; the pre-auth deadline cannot be enforced"
                    );
                }
            }
        }
        Self {
            inner,
            deadline,
            polling,
        }
    }

    /// The peer authenticated: lift the deadline and the socket timeout.
    pub(crate) fn clear_deadline(&mut self) {
        self.deadline = None;
        if self.polling {
            self.polling = false;
        } else {
            self.inner.set_read_timeout(None);
        }
    }
}

impl Read for DeadlineReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(deadline) = self.deadline else {
            return self.inner.read(buf);
        };
        if self.polling {
            loop {
                match self.inner.peek_available() {
                    // Data (or a pending EOF marker) is ready: this read cannot block.
                    Some(Ok(available)) if available > 0 => return self.inner.read(buf),
                    // Empty pipe, peer still connected: keep waiting.
                    Some(Ok(_)) => {},
                    Some(Err(error)) if is_peer_closed(&error) => return Ok(0),
                    Some(Err(error)) => return Err(error),
                    None => return self.inner.read(buf),
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(deadline_exceeded());
                }
                thread::sleep(remaining.min(DEADLINE_POLL));
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(deadline_exceeded());
        }
        self.inner.set_read_timeout(Some(remaining));
        self.inner.read(buf)
    }
}

fn deadline_exceeded() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "authentication deadline exceeded",
    )
}

/// Where the debug server should listen.
#[derive(Debug, Clone)]
pub enum Endpoint {
    /// Path-based UDS (Unix) or named pipe (Windows).
    Unix(PathBuf),
    /// TCP loopback (127.0.0.1).
    Tcp(u16),
}

/// Outcome of a synchronous method call.
#[derive(Debug)]
pub enum CallOutcome {
    /// Plain result - wrap in a JSON-RPC response and ship.
    Result(Value),
    /// Error - wrap as JSON-RPC error.
    Error(RpcError),
    /// Subscription started - caller's job to track the id and stream
    /// notifications until `unsubscribe` arrives.
    Subscription(SubscriptionHandle),
}

/// Host-provided RPC method dispatcher.
pub type MethodRouter = Arc<dyn Fn(&str, Value) -> CallOutcome + Send + Sync + 'static>;

/// Emit callback handed to subscription pumps. Returns `true` while the
/// peer is still attached and the writer is healthy; `false` once the
/// pump should exit (peer disconnected or `unsubscribe` arrived).
pub type SubscriptionEmit = Box<dyn Fn(Value) -> bool + Send>;

/// Closure body of a subscription pump. Receives the emit callback and
/// runs until cancellation.
pub type SubscriptionPump = Box<dyn FnOnce(SubscriptionEmit, Arc<AtomicBool>) + Send + 'static>;

/// Per-subscription pump. The closure runs on a dedicated thread and
/// repeatedly emits notification payloads until the subscription is
/// cancelled.
pub struct SubscriptionHandle {
    pub method: &'static str,
    pub run: SubscriptionPump,
}

impl std::fmt::Debug for SubscriptionHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SubscriptionHandle")
            .field("method", &self.method)
            .finish_non_exhaustive()
    }
}

impl SubscriptionHandle {
    pub fn new(method: &'static str, run: SubscriptionPump) -> Self {
        Self { method, run }
    }
}

/// Shared writer half of the connection. Wrapped in a `Mutex` so
/// subscription threads can interleave `notify` frames with response
/// frames from the main read loop.
type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;
type Subscriptions = Arc<Mutex<std::collections::HashMap<u64, Subscription>>>;

struct Subscription {
    cancel: Arc<AtomicBool>,
}

struct AttachmentGuard(Arc<AtomicBool>);

impl Drop for AttachmentGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Spawn the accept loop on a dedicated thread.
pub fn spawn_accept_loop(
    endpoint: Endpoint,
    auth_token: Option<String>,
    router: MethodRouter,
) -> std::io::Result<()> {
    // Never listen without a secret: an unauthenticated local endpoint would
    // let any process on the machine drive the debugger.
    if auth_token.as_deref().is_none_or(str::is_empty) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the debug server requires a non-empty auth token",
        ));
    }
    let attached = Arc::new(AtomicBool::new(false));
    match endpoint {
        Endpoint::Tcp(port) => {
            let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
            thread::Builder::new()
                .name("tynd-debug-accept".into())
                .spawn(move || tcp_accept(listener, attached, auth_token, router))?;
        },
        Endpoint::Unix(path) => {
            spawn_local_socket(path, attached, auth_token, router)?;
        },
    }
    Ok(())
}

fn tcp_accept(
    listener: std::net::TcpListener,
    attached: Arc<AtomicBool>,
    auth_token: Option<String>,
    router: MethodRouter,
) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        if attached
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            let _ = reject_busy(&mut stream);
            continue;
        }
        let attached2 = attached.clone();
        let auth_token = auth_token.clone();
        let router = router.clone();
        let Ok(writer) = stream.try_clone() else {
            attached.store(false, Ordering::Release);
            continue;
        };
        if let Err(e) = thread::Builder::new()
            .name("tynd-debug-conn".into())
            .spawn(move || {
                let _attachment = AttachmentGuard(attached2);
                let r: Box<dyn TimedRead> = Box::new(stream);
                let w: Box<dyn Write + Send> = Box::new(writer);
                handle_connection(r, w, auth_token, router);
            })
        {
            tracing::warn!(target: "tynd::debug", "debug server thread spawn failed: {e}");
            attached.store(false, Ordering::Release);
        }
    }
}

fn reject_busy<W: Write + ?Sized>(w: &mut W) -> std::io::Result<()> {
    let resp = Response::err(None, RpcError::already_attached());
    let body = encode(&resp).unwrap_or_else(|_| b"{}".to_vec());
    write_frame(w, &body)
}

#[cfg(feature = "local-socket")]
fn spawn_local_socket(
    path: PathBuf,
    attached: Arc<AtomicBool>,
    auth_token: Option<String>,
    router: MethodRouter,
) -> std::io::Result<()> {
    spawn_local_socket_impl(path, attached, auth_token, router)
}

#[cfg(not(feature = "local-socket"))]
fn spawn_local_socket(
    _path: PathBuf,
    _attached: Arc<AtomicBool>,
    _auth_token: Option<String>,
    _router: MethodRouter,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "local debug sockets require the local-socket feature",
    ))
}

#[cfg(all(unix, feature = "local-socket"))]
fn spawn_local_socket_impl(
    path: PathBuf,
    attached: Arc<AtomicBool>,
    auth_token: Option<String>,
    router: MethodRouter,
) -> std::io::Result<()> {
    use interprocess::local_socket::{prelude::*, GenericFilePath, ListenerOptions};
    use std::os::unix::fs::FileTypeExt;

    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_socket() => std::fs::remove_file(&path)?,
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "refusing to replace non-socket debug endpoint '{}'",
                    path.display()
                ),
            ));
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error),
    }
    let name = path
        .as_os_str()
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{e}")))?;
    let listener = ListenerOptions::new().name(name).create_sync()?;
    thread::Builder::new()
        .name("tynd-debug-accept".into())
        .spawn(move || local_accept(listener, attached, auth_token, router))?;
    Ok(())
}

#[cfg(all(windows, feature = "local-socket"))]
fn spawn_local_socket_impl(
    path: PathBuf,
    attached: Arc<AtomicBool>,
    auth_token: Option<String>,
    router: MethodRouter,
) -> std::io::Result<()> {
    use interprocess::{
        local_socket::{prelude::*, GenericNamespaced, ListenerOptions},
        os::windows::local_socket::ListenerOptionsExt,
    };
    // On Windows the `path` argument is interpreted as a pipe name
    // (`\\.\pipe\<name>`). Strip the leading backslashes if present.
    let raw = path.to_string_lossy().to_string();
    let pipe_name = raw
        .trim_start_matches(r"\\.\pipe\")
        .trim_start_matches(r"\\?\pipe\")
        .to_string();
    let leaked: &'static str = Box::leak(pipe_name.into_boxed_str());
    let name = leaked
        .to_ns_name::<GenericNamespaced>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{e}")))?;
    // Owner-only DACL: the default one lets any other local account connect
    // (and occupy the single attach slot) while the app runs.
    let descriptor = owner_only_security_descriptor()?;
    let listener = ListenerOptions::new()
        .name(name)
        .security_descriptor(descriptor)
        .create_sync()?;
    thread::Builder::new()
        .name("tynd-debug-accept".into())
        .spawn(move || local_accept(listener, attached, auth_token, router))?;
    Ok(())
}

/// Security descriptor granting Generic-All to the current user's SID only
/// (protected DACL, so nothing is inherited).
#[cfg(all(windows, feature = "local-socket"))]
fn owner_only_security_descriptor(
) -> std::io::Result<interprocess::os::windows::security_descriptor::SecurityDescriptor> {
    let sid = win_sid::current_user_sid_string()?;
    let sddl = format!("O:{sid}G:{sid}D:P(A;;GA;;;{sid})");
    let sddl = widestring::U16CString::from_str(&sddl)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    interprocess::os::windows::security_descriptor::SecurityDescriptor::deserialize(&sddl)
}

#[cfg(all(windows, feature = "local-socket"))]
mod win_sid {
    use std::{ffi::c_void, io, ptr};

    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree, HANDLE},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TokenUser, TOKEN_QUERY,
            TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    struct Token(HANDLE);

    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful `OpenProcessToken`.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// SID of the running user in string form (`S-1-5-21-...`).
    pub(super) fn current_user_sid_string() -> io::Result<String> {
        let mut raw: HANDLE = ptr::null_mut();
        // SAFETY: the pseudo-handle is always valid and `raw` outlives the call.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut raw) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Token(raw);

        let mut size: u32 = 0;
        // SAFETY: a null buffer with size 0 only queries the required size.
        unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &raw mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        // `u64` backing keeps the `TOKEN_USER` cast aligned.
        let mut buffer = vec![0u64; (size as usize).div_ceil(size_of::<u64>())];
        // SAFETY: `buffer` holds at least `size` bytes.
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast::<c_void>(),
                size,
                &raw mut size,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the buffer was filled with a `TOKEN_USER` by the call above.
        let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        if sid.is_null() {
            return Err(io::Error::other("null user SID"));
        }
        let mut wide: *mut u16 = ptr::null_mut();
        // SAFETY: `sid` points into `buffer`, which is alive; `wide` is freed below.
        if unsafe { ConvertSidToStringSidW(sid, &raw mut wide) } == 0 || wide.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `wide` is a NUL-terminated UTF-16 string allocated by the call above.
        let text = unsafe {
            let mut len = 0;
            while *wide.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(wide, len))
        };
        // SAFETY: allocated with `LocalAlloc` by `ConvertSidToStringSidW`.
        unsafe { LocalFree(wide.cast::<c_void>()) };
        Ok(text)
    }
}

#[cfg(feature = "local-socket")]
fn local_accept(
    listener: interprocess::local_socket::Listener,
    attached: Arc<AtomicBool>,
    auth_token: Option<String>,
    router: MethodRouter,
) {
    use interprocess::{local_socket::traits::ListenerExt, TryClone};
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if attached
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            // Stream is bidirectional — wrap into a writer to emit the
            // already_attached error frame. interprocess gives us
            // Read+Write on the same handle.
            let mut w: Box<dyn Write + Send> = Box::new(stream);
            let _ = reject_busy(&mut *w);
            continue;
        }
        let attached2 = attached.clone();
        let auth_token = auth_token.clone();
        let router = router.clone();
        if let Err(error) = thread::Builder::new()
            .name("tynd-debug-conn".into())
            .spawn(move || {
                let _attachment = AttachmentGuard(attached2);
                let writer_clone = match stream.try_clone() {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(
                            target: "tynd::debug",
                            "stream try_clone: {e}"
                        );
                        return;
                    },
                };
                let writer_half: Box<dyn Write + Send> = Box::new(writer_clone);
                let reader_half: Box<dyn TimedRead> = Box::new(stream);
                handle_connection(reader_half, writer_half, auth_token, router);
            })
        {
            tracing::warn!(target: "tynd::debug", "debug server thread spawn failed: {error}");
            attached.store(false, Ordering::Release);
        }
    }
}

fn handle_connection(
    reader: Box<dyn TimedRead>,
    writer: Box<dyn Write + Send>,
    auth_token: Option<String>,
    router: MethodRouter,
) {
    let mut authed = auth_token.is_none();
    let deadline = (!authed).then(|| Instant::now() + PRE_AUTH_TIMEOUT);
    let mut reader = BufReader::new(DeadlineReader::new(reader, deadline));
    let writer: SharedWriter = Arc::new(Mutex::new(Box::new(BufWriter::new(writer))));
    let next_sub_id = Arc::new(AtomicU64::new(1));
    let subs: Subscriptions = Arc::new(Mutex::new(std::collections::HashMap::new()));

    loop {
        let body = match read_frame(&mut reader) {
            Ok(Some(b)) => b,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!(target: "tynd::debug", "frame read error: {e}");
                break;
            },
        };
        let Ok(req) = serde_json::from_slice::<Request>(&body) else {
            let _ = send(&writer, &Response::err(None, RpcError::parse_error()));
            if authed {
                continue;
            }
            // Pre-auth garbage is never retried: drop the peer and free the slot.
            break;
        };
        if req.jsonrpc != "2.0" || req.method.is_empty() {
            let _ = send(&writer, &Response::err(req.id, RpcError::invalid_request()));
            if authed {
                continue;
            }
            break;
        }
        if !authed {
            if !authenticate(req, &writer, auth_token.as_deref().unwrap_or("")) {
                break;
            }
            authed = true;
            reader.get_mut().clear_deadline();
            continue;
        }
        process_request(req, &writer, &next_sub_id, &subs, &router);
    }

    // Cancel any outstanding subscriptions on disconnect.
    let mut subs = subs.lock();
    for (_, s) in subs.drain() {
        s.cancel.store(true, Ordering::Release);
    }
}

/// Answer the pre-auth `hello` / `bridge.hello` handshake. Returns `true` and
/// acknowledges the peer when its cookie matches `expected`; otherwise sends
/// the error response and returns `false` so the caller drops the peer.
fn authenticate(req: Request, writer: &SharedWriter, expected: &str) -> bool {
    if req.method != "hello" && req.method != "bridge.hello" {
        let _ = send(
            writer,
            &Response::err(req.id, RpcError::invalid_params("hello required")),
        );
        return false;
    }
    let cookie = req
        .params
        .get("cookie")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !bool::from(cookie.as_bytes().ct_eq(expected.as_bytes())) {
        let _ = send(
            writer,
            &Response::err(req.id, RpcError::invalid_params("bad token")),
        );
        return false;
    }
    let _ = send(
        writer,
        // Echo the authenticated cookie so the client can prove it
        // reached the process that wrote the discovery file rather
        // than a different listener racing on the same loopback
        // endpoint.
        &Response::ok(req.id, serde_json::json!({ "cookie": cookie })),
    );
    true
}

fn process_request(
    req: Request,
    writer: &SharedWriter,
    next_sub_id: &Arc<AtomicU64>,
    subs: &Subscriptions,
    router: &MethodRouter,
) {
    if handle_unsubscribe(&req, writer, subs) {
        return;
    }

    let response_id = req.id.clone();
    let notification = req.id.is_none();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        router(&req.method, req.params)
    }))
    .unwrap_or_else(|_| CallOutcome::Error(RpcError::internal_error("method router panicked")));
    respond_to_outcome(
        outcome,
        response_id,
        notification,
        writer,
        next_sub_id,
        subs,
    );
}

fn handle_unsubscribe(req: &Request, writer: &SharedWriter, subs: &Subscriptions) -> bool {
    if req.method != "unsubscribe" && req.method != "ipc.tap_stop" {
        return false;
    }
    let mut subscriptions = subs.lock();
    if req.method == "ipc.tap_stop" {
        for (_, subscription) in subscriptions.drain() {
            subscription.cancel.store(true, Ordering::Release);
        }
    } else {
        let id = req.params.get("id").and_then(Value::as_u64).unwrap_or(0);
        if let Some(subscription) = subscriptions.remove(&id) {
            subscription.cancel.store(true, Ordering::Release);
        }
    }
    if req.id.is_some() {
        let _ = send(
            writer,
            &Response::ok(req.id.clone(), serde_json::json!({ "ok": true })),
        );
    }
    true
}

fn respond_to_outcome(
    outcome: CallOutcome,
    response_id: Option<Value>,
    notification: bool,
    writer: &SharedWriter,
    next_sub_id: &Arc<AtomicU64>,
    subs: &Subscriptions,
) {
    match outcome {
        CallOutcome::Result(value) => {
            if !notification {
                let _ = send(writer, &Response::ok(response_id, value));
            }
        },
        CallOutcome::Error(error) => {
            if !notification {
                let _ = send(writer, &Response::err(response_id, error));
            }
        },
        CallOutcome::Subscription(handle) => {
            start_subscription(handle, response_id, notification, writer, next_sub_id, subs);
        },
    }
}

fn start_subscription(
    handle: SubscriptionHandle,
    response_id: Option<Value>,
    notification: bool,
    writer: &SharedWriter,
    next_sub_id: &Arc<AtomicU64>,
    subs: &Subscriptions,
) {
    if notification {
        return;
    }
    let sub_id = next_sub_id.fetch_add(1, Ordering::AcqRel);
    let cancel = Arc::new(AtomicBool::new(false));
    subs.lock().insert(
        sub_id,
        Subscription {
            cancel: cancel.clone(),
        },
    );
    let subscription_writer = writer.clone();
    let method = handle.method;
    let cancel_for_pump = cancel.clone();
    let subs_for_pump = subs.clone();
    let (start_tx, start_rx) = std::sync::mpsc::sync_channel(0);
    let spawn_result = thread::Builder::new()
        .name(format!("tynd-debug-sub-{sub_id}"))
        .spawn(move || {
            run_subscription_pump(
                handle,
                sub_id,
                subscription_writer,
                cancel_for_pump,
                subs_for_pump,
                start_rx,
            );
        });
    match spawn_result {
        Ok(_) => {
            acknowledge_subscription(writer, response_id, method, sub_id, &cancel, subs, start_tx);
        },
        Err(error) => {
            report_subscription_spawn_error(writer, response_id, sub_id, &cancel, subs, error);
        },
    }
}

fn run_subscription_pump(
    handle: SubscriptionHandle,
    sub_id: u64,
    writer: SharedWriter,
    cancel: Arc<AtomicBool>,
    subs: Subscriptions,
    start_rx: std::sync::mpsc::Receiver<()>,
) {
    if start_rx.recv().is_err() {
        return;
    }
    let writer_for_emit = writer.clone();
    let cancel_for_emit = cancel.clone();
    let method = handle.method;
    let emit: SubscriptionEmit = Box::new(move |payload| {
        if cancel_for_emit.load(Ordering::Acquire) {
            return false;
        }
        let note = Notification {
            jsonrpc: "2.0",
            method,
            params: serde_json::json!({
                "subscriptionId": sub_id,
                "payload": payload,
            }),
        };
        let Ok(body) = encode(&note) else {
            return false;
        };
        let mut writer = writer_for_emit.lock();
        write_frame(&mut **writer, &body).is_ok()
    });
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (handle.run)(emit, cancel.clone());
    }));
    cancel.store(true, Ordering::Release);
    subs.lock().remove(&sub_id);
}

fn acknowledge_subscription(
    writer: &SharedWriter,
    response_id: Option<Value>,
    method: &'static str,
    sub_id: u64,
    cancel: &Arc<AtomicBool>,
    subs: &Subscriptions,
    start_tx: std::sync::mpsc::SyncSender<()>,
) {
    if send(
        writer,
        &Response::ok(
            response_id,
            serde_json::json!({
                "subscriptionId": sub_id,
                "method": method
            }),
        ),
    )
    .is_ok()
    {
        let _ = start_tx.send(());
    } else {
        cancel.store(true, Ordering::Release);
        subs.lock().remove(&sub_id);
    }
}

fn report_subscription_spawn_error(
    writer: &SharedWriter,
    response_id: Option<Value>,
    sub_id: u64,
    cancel: &Arc<AtomicBool>,
    subs: &Subscriptions,
    error: std::io::Error,
) {
    cancel.store(true, Ordering::Release);
    subs.lock().remove(&sub_id);
    let _ = send(
        writer,
        &Response::err(
            response_id,
            RpcError::internal_error(format!("subscription thread spawn failed: {error}")),
        ),
    );
}
fn send<T: serde::Serialize>(writer: &SharedWriter, value: &T) -> std::io::Result<()> {
    let body =
        encode(value).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut w = writer.lock();
    write_frame(&mut **w, &body)
}

#[cfg(test)]
#[cfg(all(windows, feature = "local-socket"))]
mod windows_tests {
    #[test]
    fn owner_only_descriptor_builds_for_the_current_user() {
        super::owner_only_security_descriptor().expect("owner-only descriptor");
    }
}

#[cfg(test)]
mod auth_tests {
    use std::{
        io::{self, Write},
        sync::Arc,
    };

    use parking_lot::Mutex;
    use serde_json::{json, Value};

    use super::{authenticate, SharedWriter};
    use crate::protocol::Request;

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn hello(params: Value) -> bool {
        let writer: SharedWriter = Arc::new(Mutex::new(Box::new(Sink::default())));
        let req: Request = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "hello",
            "params": params,
        }))
        .unwrap();
        authenticate(req, &writer, "secret")
    }

    #[test]
    fn hello_accepts_the_cookie_param() {
        assert!(hello(json!({ "cookie": "secret" })));
    }

    #[test]
    fn hello_rejects_the_token_param() {
        assert!(!hello(json!({ "token": "secret" })));
    }
}

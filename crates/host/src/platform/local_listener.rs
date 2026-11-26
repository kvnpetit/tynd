//! Shared lifecycle for bounded local-socket forwarding listeners.
//!
//! Single-instance and deep-link forwarding use the same transport. Keeping
//! accept polling, slow-client limits, handler ownership, and shutdown joins in
//! one place prevents either API from regressing to detached threads.

use std::{
    io::{ErrorKind, Read as _},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use interprocess::local_socket::{prelude::*, Listener, ListenerNonblockingMode, Name, Stream};

const ACCEPT_POLL: Duration = Duration::from_millis(25);
const READ_POLL: Duration = Duration::from_millis(10);
const CLIENT_DEADLINE: Duration = Duration::from_secs(2);
const MAX_ACTIVE_HANDLERS: usize = 32;
const MAX_FORWARD_BYTES: usize = 64 * 1024;
/// Longest a forwarding client waits on a busy endpoint.
const CONNECT_WAIT: Duration = Duration::from_secs(2);
/// Longest the endpoint probe waits on a busy pipe. A live instance of ours
/// always has a listening instance within this window.
#[cfg(not(unix))]
const PROBE_WAIT: Duration = Duration::from_millis(200);

/// Endpoint name for a forwarding socket, scoped to the current user.
///
/// - **Windows**: namespaced named pipe `tynd-<kind>-<hash(user SID, key)>`; the owner-only DACL
///   applied at bind time keeps other accounts out and `FILE_FLAG_FIRST_PIPE_INSTANCE` (set by
///   `interprocess` on the first instance) makes bind fail when the name is already taken.
/// - **Unix**: a filesystem socket `<dir>/<kind>-<hash>.sock` inside a per-user `0700` directory
///   (`$XDG_RUNTIME_DIR/tynd`, else a `tynd-run` dir under the per-user runtime/cache dir, else
///   `<tmp>/tynd-<uid>`). A per-machine name would let another local user bind it first or inject
///   argv. The directory is verified (owner = us, no group/other access) on every call.
///
/// This is the **bind (server) path**: on Windows it probes the name and may
/// rotate a squatted nonce. Clients must use [`connect_endpoint`], which never
/// rotates (a client rotating would strand the running primary).
pub(crate) fn endpoint_name(kind: &str, key: &str) -> Result<Name<'static>, String> {
    endpoint(kind, key, Role::Bind).map(|endpoint| endpoint.name)
}

/// Who is resolving the endpoint: only the server about to bind may rotate.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Bind,
    Connect,
}

/// A resolved forwarding endpoint: the listener/connect name plus, on Windows,
/// the pipe name needed for a bounded connect (`Name` exposes no path).
struct Endpoint {
    name: Name<'static>,
    #[cfg(not(unix))]
    pipe_base: String,
}

fn endpoint(kind: &str, key: &str, role: Role) -> Result<Endpoint, String> {
    #[cfg(unix)]
    {
        let _ = role;
        let path = unix_endpoint::socket_path(kind, key)?;
        path.to_fs_name::<interprocess::local_socket::GenericFilePath>()
            .map(|name| Endpoint { name })
            .map_err(|error| format!("socket name: {error}"))
    }
    #[cfg(not(unix))]
    {
        // User SIDs are not secret, so the name also carries a random nonce
        // kept in a per-user, per-endpoint file: another account cannot
        // compute the name and pre-create (squat) the pipe a victim will use.
        let sid = crate::platform::win_security::current_user_sid_string()
            .map_err(|error| format!("current user SID: {error}"))?;
        let dir = nonce::nonce_dir()?;
        let file = nonce::file_name(kind, key);
        let base = resolve_pipe_base(&dir, &file, &sid, kind, key, role, probe_endpoint)?;
        windows_endpoint(base)
    }
}

/// Pick the pipe base for an endpoint from its nonce file.
///
/// - `Connect`: read the current nonce and return it; never probe, never rotate.
/// - `Bind`: probe first; only a server confirmed foreign (another account, or
///   our DACL-refused access) triggers a rotation. A busy pipe is *not* proof of
///   a squatter (a slow primary of ours looks the same), so it keeps the name:
///   the bind then fails and the caller degrades.
#[cfg(not(unix))]
fn resolve_pipe_base(
    dir: &std::path::Path,
    file: &str,
    sid: &str,
    kind: &str,
    key: &str,
    role: Role,
    probe: impl Fn(&str) -> EndpointOwner,
) -> Result<String, String> {
    let nonce = nonce::load_or_create(dir, file)?;
    let base = windows_pipe_base(sid, kind, key, &nonce);
    if role == Role::Connect || !matches!(probe(&base), EndpointOwner::Foreign) {
        return Ok(base);
    }
    // Pipe names can be listed by every local account, so a name can leak
    // and be pre-created by another user. Our own server would have passed
    // the probe, hence a foreign one is a squatter: move this endpoint to a
    // fresh nonce instead of running without forwarding for ever. Only this
    // endpoint rotates, so other running apps keep their pipes.
    crate::tynd_warn!("{kind} endpoint is held by another account; using a fresh pipe name");
    let nonce = nonce::rotate(dir, file)?;
    Ok(windows_pipe_base(sid, kind, key, &nonce))
}

#[cfg(not(unix))]
fn windows_endpoint(pipe_base: String) -> Result<Endpoint, String> {
    Ok(Endpoint {
        name: windows_pipe_name(&pipe_base)?,
        pipe_base,
    })
}

/// Connect to a forwarding endpoint as a client, waiting at most
/// [`CONNECT_WAIT`] when every pipe instance is busy. The plain
/// `Stream::connect` waits without limit on Windows, so a squatter holding the
/// only instance would hang the launch; on timeout this returns an error and
/// the caller degrades like a bind failure.
pub(crate) fn connect_endpoint(kind: &str, key: &str) -> Result<Stream, String> {
    // Client path: resolve without probing or rotating.
    let endpoint = endpoint(kind, key, Role::Connect)?;
    #[cfg(unix)]
    {
        interprocess::local_socket::ConnectOptions::new()
            .name(endpoint.name)
            .wait_mode(interprocess::ConnectWaitMode::Timeout(CONNECT_WAIT))
            .connect_sync()
            .map_err(|error| format!("connect: {error}"))
    }
    #[cfg(not(unix))]
    {
        connect_pipe(&endpoint.pipe_base, CONNECT_WAIT)
    }
}

/// Bounded connect to `\\.\pipe\<base>` returned as a `local_socket` stream.
#[cfg(not(unix))]
fn connect_pipe(base: &str, wait: Duration) -> Result<Stream, String> {
    use std::os::windows::io::OwnedHandle;

    use interprocess::{
        os::windows::named_pipe::{
            local_socket::Stream as PipeLocalStream, pipe_mode::Bytes, DuplexPipeStream,
        },
        ConnectWaitMode,
    };

    let path = format!(r"\\.\pipe\{base}");
    let pipe = DuplexPipeStream::<Bytes>::connect_by_path_with_wait_mode(
        path.as_str(),
        ConnectWaitMode::Timeout(wait),
    )
    .map_err(|error| format!("connect: {error}"))?;
    let handle = OwnedHandle::try_from(pipe)
        .map_err(|_| "connect: unexpected split pipe stream".to_string())?;
    PipeLocalStream::try_from(handle)
        .map(Stream::from)
        .map_err(|error| format!("connect: {error}"))
}

/// Pipe name without the `\\.\pipe\` prefix.
#[cfg(not(unix))]
fn windows_pipe_base(sid: &str, kind: &str, key: &str, nonce: &str) -> String {
    let hash = blake3::hash(format!("{sid}:{key}:{nonce}").as_bytes());
    format!("tynd-{kind}-{}", &hash.to_hex().as_str()[..24])
}

#[cfg(not(unix))]
fn windows_pipe_name(base: &str) -> Result<Name<'static>, String> {
    let leaked: &'static str = Box::leak(base.to_owned().into_boxed_str());
    leaked
        .to_ns_name::<interprocess::local_socket::GenericNamespaced>()
        .map_err(|error| format!("socket name: {error}"))
}

/// Who answers on an endpoint name right now.
#[cfg(not(unix))]
enum EndpointOwner {
    /// Nobody, or we could not tell (transient error): keep the name.
    Free,
    /// Every instance stayed busy past [`PROBE_WAIT`]: possibly a squatter, but
    /// also a slow primary of ours, so the name is kept.
    Busy,
    /// A server of the current user: a legitimate running instance.
    Ours,
    /// A server of another account, or one whose DACL refuses us: a squatter.
    Foreign,
}

/// Connect (writing nothing) to find out whether the name is taken by a server
/// that is not ours. A legitimate running instance answers as the current user
/// and is never reported as foreign.
///
/// The wait on a busy pipe is bounded by [`PROBE_WAIT`]: a squatter holding the
/// only instance connected would otherwise block the caller for ever (the
/// `local_socket` connect waits without limit on `ERROR_PIPE_BUSY`), and this
/// runs under the single-instance state lock.
#[cfg(not(unix))]
fn probe_endpoint(base: &str) -> EndpointOwner {
    use interprocess::{
        os::windows::named_pipe::{pipe_mode::Bytes, DuplexPipeStream},
        ConnectWaitMode,
    };

    let path = format!(r"\\.\pipe\{base}");
    let stream = match DuplexPipeStream::<Bytes>::connect_by_path_with_wait_mode(
        path.as_str(),
        ConnectWaitMode::Timeout(PROBE_WAIT),
    ) {
        Ok(stream) => stream,
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            return EndpointOwner::Foreign;
        },
        Err(error) if error.kind() == ErrorKind::TimedOut => return EndpointOwner::Busy,
        Err(_) => return EndpointOwner::Free,
    };
    let Ok(pid) = stream.peer_process_id() else {
        return EndpointOwner::Free;
    };
    match crate::platform::win_security::process_is_current_user(pid) {
        Ok(true) => EndpointOwner::Ours,
        Ok(false) => EndpointOwner::Foreign,
        Err(_) => EndpointOwner::Free,
    }
}

/// Per-user random nonce that keeps Windows pipe names unguessable by other
/// accounts. The file lives under the user's local app-data directory, which
/// the default ACL restricts to that user (and administrators).
#[cfg(any(windows, test))]
mod nonce {
    #[cfg(windows)]
    use std::path::PathBuf;
    use std::{fs, io::Write as _, path::Path, time::Duration};

    const NONCE_BYTES: usize = 32;

    /// One nonce file per endpoint (`kind` + `key`), so rotating a squatted
    /// endpoint never changes the pipe name of another running app.
    pub(super) fn file_name(kind: &str, key: &str) -> String {
        let hash = blake3::hash(format!("{kind}:{key}").as_bytes());
        format!("pipe-nonce-{}", &hash.to_hex().as_str()[..16])
    }

    #[cfg(windows)]
    pub(super) fn nonce_dir() -> Result<PathBuf, String> {
        dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .map(|dir| dir.join("tynd"))
            .ok_or_else(|| "no per-user app-data directory for the pipe nonce".to_string())
    }

    fn is_valid(text: &str) -> bool {
        text.len() == NONCE_BYTES * 2 && text.bytes().all(|b| b.is_ascii_hexdigit())
    }

    /// Read the nonce in `dir`, creating it atomically when missing: the value
    /// is written to a private temp file and hard-linked into place, so a
    /// concurrent reader never sees partial content and two racing creators
    /// converge on one value.
    pub(super) fn load_or_create(dir: &Path, file_name: &str) -> Result<String, String> {
        fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let path = dir.join(file_name);
        for _ in 0..5 {
            if let Ok(text) = fs::read_to_string(&path) {
                let text = text.trim();
                if is_valid(text) {
                    return Ok(text.to_owned());
                }
                // Truncated or tampered: replace it (we are the file's owner).
                let _ = fs::remove_file(&path);
            }
            let mut bytes = [0u8; NONCE_BYTES];
            getrandom::fill(&mut bytes).map_err(|e| format!("random nonce: {e}"))?;
            let value = blake3::Hash::from(bytes).to_hex().to_string();
            let temp = dir.join(format!("{file_name}.{}.tmp", &value[..16]));
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|e| format!("create {}: {e}", temp.display()))?;
            file.write_all(value.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|e| format!("write {}: {e}", temp.display()))?;
            drop(file);
            let linked = fs::hard_link(&temp, &path);
            let _ = fs::remove_file(&temp);
            match linked {
                Ok(()) => return Ok(value),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::thread::sleep(Duration::from_millis(10));
                },
                Err(e) => return Err(format!("publish {}: {e}", path.display())),
            }
        }
        Err(format!("cannot establish {}", path.display()))
    }

    /// Replace the nonce in `dir` with a fresh one and return it.
    pub(super) fn rotate(dir: &Path, file_name: &str) -> Result<String, String> {
        let _ = fs::remove_file(dir.join(file_name));
        load_or_create(dir, file_name)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const FILE: &str = "pipe-nonce-test";

        #[test]
        fn nonce_is_random_persisted_and_repaired() {
            let tmp = tempfile::tempdir().unwrap();
            let a = load_or_create(&tmp.path().join("a"), FILE).unwrap();
            assert!(is_valid(&a));
            assert_eq!(a, load_or_create(&tmp.path().join("a"), FILE).unwrap());
            let b = load_or_create(&tmp.path().join("b"), FILE).unwrap();
            assert_ne!(a, b);
            fs::write(tmp.path().join("a").join(FILE), "short").unwrap();
            let repaired = load_or_create(&tmp.path().join("a"), FILE).unwrap();
            assert!(is_valid(&repaired));
            assert!(fs::read_dir(tmp.path().join("a")).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")));
        }

        #[test]
        fn rotation_replaces_only_the_named_endpoint() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path();
            let first = load_or_create(dir, FILE).unwrap();
            let other = load_or_create(dir, "pipe-nonce-other").unwrap();
            let rotated = rotate(dir, FILE).unwrap();
            assert!(is_valid(&rotated));
            assert_ne!(first, rotated);
            assert_eq!(rotated, load_or_create(dir, FILE).unwrap());
            assert_eq!(other, load_or_create(dir, "pipe-nonce-other").unwrap());
        }

        #[test]
        fn file_names_are_per_endpoint() {
            assert_ne!(
                file_name("single-instance", "app"),
                file_name("deeplink", "app")
            );
            assert_ne!(file_name("deeplink", "a"), file_name("deeplink", "b"));
            assert_eq!(file_name("deeplink", "a"), file_name("deeplink", "a"));
        }
    }
}

#[cfg(all(test, windows))]
// clippy only treats a bare `#[cfg(test)]` module as test code; this one sits
// mid-file (items_after_test_module), so allow unwrap in its helpers here.
#[allow(clippy::unwrap_used, reason = "test-only helpers")]
mod probe_tests {
    use super::*;

    fn unique_base(tag: &str) -> String {
        format!("tynd-probe-test-{tag}-{}", std::process::id())
    }

    fn listen(base: &str) -> Listener {
        let name = windows_pipe_name(base).unwrap();
        interprocess::local_socket::ListenerOptions::new()
            .name(name)
            .create_sync()
            .unwrap()
    }

    #[test]
    fn unused_name_is_free() {
        assert!(matches!(
            probe_endpoint(&unique_base("free")),
            EndpointOwner::Free
        ));
    }

    #[test]
    fn our_own_server_is_never_foreign() {
        let base = unique_base("ours");
        let _listener = listen(&base);
        assert!(matches!(probe_endpoint(&base), EndpointOwner::Ours));
    }

    #[test]
    fn busy_squatter_times_out_instead_of_hanging() {
        let base = unique_base("busy");
        let listener = listen(&base);
        // Occupy the only listening instance and never accept: every further
        // connect sees ERROR_PIPE_BUSY.
        let _squatted = Stream::connect(windows_pipe_name(&base).unwrap()).unwrap();
        let started = Instant::now();
        let owner = probe_endpoint(&base);
        assert!(matches!(owner, EndpointOwner::Busy));
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(listener);
    }

    fn resolve(dir: &std::path::Path, role: Role, owner: fn() -> EndpointOwner) -> (String, usize) {
        let calls = std::cell::Cell::new(0usize);
        let base = resolve_pipe_base(dir, "pipe-nonce-test", "sid", "kind", "key", role, |_| {
            calls.set(calls.get() + 1);
            owner()
        })
        .unwrap();
        (base, calls.get())
    }

    #[test]
    fn client_never_probes_or_rotates() {
        let tmp = tempfile::tempdir().unwrap();
        let (first, _) = resolve(tmp.path(), Role::Bind, || EndpointOwner::Ours);
        let (client, calls) = resolve(tmp.path(), Role::Connect, || EndpointOwner::Foreign);
        assert_eq!(client, first);
        assert_eq!(calls, 0);
        let (again, _) = resolve(tmp.path(), Role::Bind, || EndpointOwner::Ours);
        assert_eq!(again, first);
    }

    #[test]
    fn only_a_confirmed_foreign_owner_rotates_on_bind() {
        let tmp = tempfile::tempdir().unwrap();
        let (first, _) = resolve(tmp.path(), Role::Bind, || EndpointOwner::Ours);
        for owner in [
            (|| EndpointOwner::Busy) as fn() -> EndpointOwner,
            || EndpointOwner::Free,
            || EndpointOwner::Ours,
        ] {
            let (base, calls) = resolve(tmp.path(), Role::Bind, owner);
            assert_eq!(base, first);
            assert_eq!(calls, 1);
        }
        let (rotated, _) = resolve(tmp.path(), Role::Bind, || EndpointOwner::Foreign);
        assert_ne!(rotated, first);
        let (client, _) = resolve(tmp.path(), Role::Connect, || EndpointOwner::Foreign);
        assert_eq!(client, rotated);
    }

    #[test]
    fn bounded_connect_reaches_a_live_server() {
        let base = unique_base("connect-ok");
        let _listener = listen(&base);
        assert!(connect_pipe(&base, Duration::from_millis(500)).is_ok());
    }

    #[test]
    fn bounded_connect_gives_up_on_a_busy_squatter() {
        let base = unique_base("connect-busy");
        let _listener = listen(&base);
        let _squatted = Stream::connect(windows_pipe_name(&base).unwrap()).unwrap();
        let started = Instant::now();
        assert!(connect_pipe(&base, Duration::from_millis(300)).is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}

/// Client-side check run after connecting and before writing anything:
/// the server end must belong to the current user. Refuses a squatter that
/// pre-created the endpoint under another account.
pub(crate) fn verify_server_is_current_user(stream: &Stream) -> Result<(), String> {
    use interprocess::local_socket::traits::StreamCommon as _;

    let creds = stream
        .peer_creds()
        .map_err(|error| format!("cannot identify the endpoint owner: {error}"))?;
    #[cfg(windows)]
    {
        let pid = creds
            .pid()
            .ok_or_else(|| "cannot identify the endpoint owner process".to_string())?;
        match crate::platform::win_security::process_is_current_user(pid) {
            Ok(true) => Ok(()),
            Ok(false) => Err("endpoint is owned by another user; refusing to connect".into()),
            Err(error) => Err(format!("cannot verify the endpoint owner: {error}")),
        }
    }
    #[cfg(unix)]
    {
        match creds.euid() {
            Some(peer) if peer != unix_endpoint::current_uid() => {
                Err("endpoint is owned by another user; refusing to connect".into())
            },
            _ => Ok(()),
        }
    }
}

/// Remove a socket file left by a crashed previous owner. Call only while
/// holding the instance lock (so nobody live owns the file) and before bind;
/// without it a crash leaves the path behind and the next bind fails with
/// `EADDRINUSE`. No-op on Windows.
pub(crate) fn remove_stale_endpoint(kind: &str, key: &str) {
    #[cfg(unix)]
    if let Ok(path) = unix_endpoint::socket_path(kind, key) {
        let _ = std::fs::remove_file(path);
    }
    #[cfg(not(unix))]
    let _ = (kind, key);
}

#[cfg(unix)]
mod unix_endpoint {
    use std::{
        fs,
        os::unix::fs::{DirBuilderExt as _, MetadataExt as _},
        path::{Path, PathBuf},
    };

    /// Longest `sun_path` we allow (macOS: 104 incl. NUL).
    const MAX_SOCKET_PATH_BYTES: usize = 100;

    pub(super) fn current_uid() -> u32 {
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        unsafe { libc::geteuid() }
    }

    fn base_dir(uid: u32) -> PathBuf {
        match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(dir) if Path::new(&dir).is_absolute() => PathBuf::from(dir).join("tynd"),
            _ => dirs::runtime_dir().or_else(dirs::cache_dir).map_or_else(
                || std::env::temp_dir().join(format!("tynd-{uid}")),
                |dir| dir.join("tynd-run"),
            ),
        }
    }

    pub(super) fn socket_path(kind: &str, key: &str) -> Result<PathBuf, String> {
        let uid = current_uid();
        let dir = base_dir(uid);
        ensure_private_dir(&dir, uid)?;
        let path = dir.join(file_name(kind, key, uid));
        if path.as_os_str().len() > MAX_SOCKET_PATH_BYTES {
            return Err(format!(
                "socket path {} exceeds the {MAX_SOCKET_PATH_BYTES}-byte sun_path limit",
                path.display()
            ));
        }
        Ok(path)
    }

    /// `<kind>-<16 hex of blake3(uid:key)>.sock`: bounded length whatever the
    /// app id is, and distinct per user.
    pub(super) fn file_name(kind: &str, key: &str, uid: u32) -> String {
        let hash = blake3::hash(format!("{uid}:{key}").as_bytes());
        format!("{kind}-{}.sock", &hash.to_hex().as_str()[..16])
    }

    /// Create `dir` (mode 0700) when missing and refuse it unless it is a real
    /// directory owned by `uid` with no group/other permissions.
    pub(super) fn ensure_private_dir(dir: &Path, uid: u32) -> Result<(), String> {
        match fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(error) => return Err(format!("create {}: {error}", dir.display())),
        }
        let meta = fs::symlink_metadata(dir)
            .map_err(|error| format!("stat {}: {error}", dir.display()))?;
        if !meta.is_dir() {
            return Err(format!("{} is not a directory", dir.display()));
        }
        if meta.uid() != uid {
            return Err(format!(
                "{} is owned by uid {}, not the current user",
                dir.display(),
                meta.uid()
            ));
        }
        if meta.mode() & 0o077 != 0 {
            return Err(format!(
                "{} must not be accessible by group/other (mode {:o})",
                dir.display(),
                meta.mode() & 0o777
            ));
        }
        Ok(())
    }

    /// True when the peer of `stream` runs as the current user. Platforms
    /// that expose no peer uid are accepted: the socket lives in a private
    /// directory only this user can enter.
    pub(super) fn peer_is_current_user(stream: &super::Stream) -> bool {
        use interprocess::local_socket::traits::StreamCommon as _;
        match stream.peer_creds() {
            Ok(creds) => creds.euid().is_none_or(|peer| peer == current_uid()),
            // Doc contract: platforms that expose no peer uid are accepted.
            Err(error) => error.kind() == std::io::ErrorKind::Unsupported,
        }
    }

    #[cfg(test)]
    mod tests {
        use std::os::unix::fs::PermissionsExt as _;

        use super::*;

        #[test]
        fn file_name_is_bounded_and_per_user() {
            let long = "x".repeat(500);
            let a = file_name("si", &long, 1000);
            assert!(a.len() < 40, "{a}");
            assert_ne!(a, file_name("si", &long, 1001));
            assert_ne!(file_name("si", "app", 1000), file_name("dl", "app", 1000));
        }

        #[test]
        fn private_dir_is_created_0700_and_accepted() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("run");
            let uid = current_uid();
            ensure_private_dir(&dir, uid).unwrap();
            assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
            ensure_private_dir(&dir, uid).unwrap();
        }

        #[test]
        fn loose_or_foreign_dir_is_refused() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("run");
            let uid = current_uid();
            ensure_private_dir(&dir, uid).unwrap();
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(ensure_private_dir(&dir, uid).is_err());
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(ensure_private_dir(&dir, uid.wrapping_add(1)).is_err());
        }

        #[test]
        fn symlinked_dir_is_refused() {
            let tmp = tempfile::tempdir().unwrap();
            let real = tmp.path().join("real");
            ensure_private_dir(&real, current_uid()).unwrap();
            let link = tmp.path().join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert!(ensure_private_dir(&link, current_uid()).is_err());
        }
    }
}

/// Running forwarding listener. Dropping or [`shutdown`](Self::shutdown)
/// stops accepting and joins the accept thread (which joins its handlers).
pub(crate) struct LocalListener {
    stopping: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl LocalListener {
    fn stop_and_join(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }

    pub(crate) fn shutdown(mut self) {
        self.stop_and_join();
    }
}

impl Drop for LocalListener {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

/// Accept connections on `listener` from a `thread_name` thread and pass
/// each client's first line (at most 64 KiB, within a 2 s deadline) to
/// `on_line`.
pub(crate) fn spawn<F>(
    thread_name: &'static str,
    listener: Listener,
    on_line: F,
) -> Result<LocalListener, String>
where
    F: Fn(String) + Send + Sync + 'static,
{
    listener
        .set_nonblocking(ListenerNonblockingMode::Accept)
        .map_err(|error| format!("{thread_name}: set listener nonblocking: {error}"))?;
    let stopping = Arc::new(AtomicBool::new(false));
    let worker_stopping = Arc::clone(&stopping);
    let on_line = Arc::new(on_line);
    let join = std::thread::Builder::new()
        .name(thread_name.into())
        .spawn(move || run(thread_name, listener, &worker_stopping, &on_line))
        .map_err(|error| format!("{thread_name}: listener spawn failed: {error}"))?;
    Ok(LocalListener {
        stopping,
        join: Some(join),
    })
}

fn run<F>(
    thread_name: &'static str,
    listener: Listener,
    stopping: &Arc<AtomicBool>,
    on_line: &Arc<F>,
) where
    F: Fn(String) + Send + Sync + 'static,
{
    let mut handlers = Vec::<JoinHandle<()>>::new();
    while !stopping.load(Ordering::Acquire) {
        reap_finished(&mut handlers);
        match listener.accept() {
            Ok(stream) => {
                #[cfg(unix)]
                if !unix_endpoint::peer_is_current_user(&stream) {
                    crate::tynd_warn!("{thread_name}: rejected a connection from another user");
                    continue;
                }
                if handlers.len() >= MAX_ACTIVE_HANDLERS {
                    continue;
                }
                match spawn_handler(thread_name, stream, stopping, on_line) {
                    Ok(join) => handlers.push(join),
                    Err(error) => {
                        crate::tynd_warn!("local forwarding handler spawn failed: {error}");
                    },
                }
            },
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL);
            },
            Err(error) if error.kind() == ErrorKind::Interrupted => {},
            Err(error) => {
                crate::tynd_warn!("local forwarding accept failed: {error}");
                std::thread::sleep(ACCEPT_POLL);
            },
        }
    }

    for handler in handlers {
        let _ = handler.join();
    }
}

/// Read one bounded line from `stream` on its own thread and hand it to
/// `on_line`, unless the listener started stopping meanwhile.
fn spawn_handler<F>(
    thread_name: &'static str,
    stream: Stream,
    stopping: &Arc<AtomicBool>,
    on_line: &Arc<F>,
) -> std::io::Result<JoinHandle<()>>
where
    F: Fn(String) + Send + Sync + 'static,
{
    let stopping = Arc::clone(stopping);
    let on_line = Arc::clone(on_line);
    std::thread::Builder::new()
        .name(format!("{thread_name}-handler"))
        .spawn(move || {
            if let Some(line) = read_bounded_line(stream, &stopping) {
                if !stopping.load(Ordering::Acquire) {
                    on_line(line);
                }
            }
        })
}

fn reap_finished(handlers: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < handlers.len() {
        if handlers[index].is_finished() {
            let handler = handlers.swap_remove(index);
            let _ = handler.join();
        } else {
            index += 1;
        }
    }
}

fn read_bounded_line(mut stream: Stream, stopping: &AtomicBool) -> Option<String> {
    stream.set_nonblocking(true).ok()?;
    let deadline = Instant::now() + CLIENT_DEADLINE;
    let mut line = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];

    loop {
        if stopping.load(Ordering::Acquire) || Instant::now() >= deadline {
            return None;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(read) => {
                if let Some(newline) = chunk[..read].iter().position(|byte| *byte == b'\n') {
                    if line.len() + newline > MAX_FORWARD_BYTES {
                        return None;
                    }
                    line.extend_from_slice(&chunk[..newline]);
                    return String::from_utf8(line).ok();
                }
                if line.len() + read > MAX_FORWARD_BYTES {
                    return None;
                }
                line.extend_from_slice(&chunk[..read]);
            },
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(READ_POLL);
            },
            Err(error) if error.kind() == ErrorKind::Interrupted => {},
            Err(_) => return None,
        }
    }
}

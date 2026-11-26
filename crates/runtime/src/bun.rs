#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    ffi::OsString,
    io::BufReader,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use crossbeam_channel as mpsc;
use tynd_host::tynd_error;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

use tynd_protocol::runtime::{BackendBridge, BackendCall, BackendConfig, BackendEvent};

mod inbound;
mod outbound;
mod wire;

use inbound::{spawn_stderr_reader, spawn_stdout_reader, StdoutReaderContext};
use outbound::spawn_stdin_forwarder;

/// Guarantees the Bun child is killed when the last `ReloadHandle` clone
/// drops. `std::process::Child::drop` leaks the process on Unix, so we
/// explicitly kill + wait here to avoid orphans on host panic / abnormal exit.
struct ChildSlot(Mutex<Option<Child>>);

impl ChildSlot {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Child>> {
        // Recover from poison rather than panicking the caller — the
        // protected state is just `Option<Child>` and a forwarder-thread
        // panic shouldn't take the host with it. Matches the recovery
        // pattern used across `os/` (process, log, websocket, …).
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Exit code of a child that is exiting on its own, waiting up to
    /// `timeout`. `None` when it was killed by a signal or is still running
    /// (it is left in the slot for `terminate`).
    fn wait_exit_code(&self, timeout: std::time::Duration) -> Option<i32> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.lock().as_mut()?.try_wait() {
                Ok(Some(status)) => return status.code(),
                Ok(None) => {},
                Err(_) => return None,
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Kill and reap the current child, if any.
    fn terminate(&self) {
        if let Some(mut child) = self.lock().take() {
            terminate_child(&mut child);
        }
    }
}

impl Drop for ChildSlot {
    fn drop(&mut self) {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(mut child) = guard.take() {
            terminate_child(&mut child);
        }
    }
}

/// Dev-mode handle: swap the Bun subprocess in place while the host and WebView stay alive.
#[derive(Clone)]
pub(crate) struct ReloadHandle {
    entry_path: String,
    /// Forwarded to every spawned child after the entry path.
    app_args: Vec<OsString>,
    stdin_slot: Arc<Mutex<Option<ChildStdin>>>,
    child_slot: Arc<ChildSlot>,
    event_tx: mpsc::Sender<BackendEvent>,
    call_tx: mpsc::Sender<BackendCall>,
    /// Serializes cloned reload handles. Without this, concurrent file-watch
    /// notifications could spawn two children and overwrite one live handle.
    reload_lock: Arc<Mutex<()>>,
    /// `true` during a controlled reload so the stdin forwarder can suppress
    /// the expected pipe close.
    reload_in_progress: Arc<AtomicBool>,
    /// Shutdown marker belonging to the current Bun generation. Each stdout
    /// reader owns a distinct marker, preventing a delayed EOF from an old
    /// child being classified using the new child's state.
    reader_shutdown_expected: Arc<Mutex<Arc<AtomicBool>>>,
}

impl ReloadHandle {
    /// Webview-only reload (no Bun respawn) — used when only static frontend
    /// files changed on disk.
    pub(crate) fn reload_frontend_only(&self) {
        let _ = self.event_tx.send(BackendEvent::Reload);
    }

    pub(crate) fn reload(&self) {
        let _reload_guard = self
            .reload_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Mark only the current generation's stdout close as expected.
        self.reload_in_progress.store(true, Ordering::SeqCst);
        self.reader_shutdown_expected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .store(true, Ordering::SeqCst);

        // Killing the child closes stdout, unblocking the reader thread.
        self.child_slot.terminate();
        *self
            .stdin_slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;

        let new_reader_shutdown_expected = Arc::new(AtomicBool::new(false));

        match spawn_bun(
            &self.entry_path,
            &self.app_args,
            &self.event_tx,
            &self.call_tx,
            Arc::clone(&new_reader_shutdown_expected),
            &BunSlots {
                child: &self.child_slot,
                stdin: &self.stdin_slot,
            },
        ) {
            Ok(new_config) => {
                *self
                    .reader_shutdown_expected
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    new_reader_shutdown_expected;
                self.reload_in_progress.store(false, Ordering::SeqCst);
                // BackendConfig from the freshly-spawned child is intentionally
                // discarded here: the host's window / menu / tray were created
                // once at startup and the main event loop has no
                // `BackendEvent::ConfigUpdate` handler that would re-apply the
                // new shape. Surface a hint so HMR users who change
                // `window.title`, `menu`, or `tray` know to restart instead of
                // wondering why their tweak didn't land. Frontend changes
                // (HTML/JS/CSS) reload normally via the `Reload` event below.
                let drop_hint = !new_config.menu.is_empty() || new_config.tray.is_some();
                let _ = (new_config, drop_hint);
                if drop_hint {
                    tynd_host::tynd_warn!(
                        "reload: window/menu/tray config changes are not applied — restart the \
                         app to pick up the new shape"
                    );
                }
                let _ = self.event_tx.send(BackendEvent::Reload);
            },
            Err(e) => {
                self.reload_in_progress.store(false, Ordering::SeqCst);
                let _ = self.event_tx.send(BackendEvent::Error {
                    message: format!("Failed to reload backend: {e}"),
                });
            },
        }
    }
}

/// Spawn a Bun subprocess, wait for the initial config, and return the bridge
/// plus a reload handle. When launched by a packed binary, TYND_BUN_PATH points
/// to the packaged Bun binary instead of the system Bun.
pub(crate) fn start(entry_path: &str, app_args: Vec<OsString>) -> (BackendBridge, ReloadHandle) {
    let (call_tx, call_rx) = mpsc::unbounded::<BackendCall>();
    let (event_tx, event_rx) = mpsc::unbounded::<BackendEvent>();
    let reload_in_progress = Arc::new(AtomicBool::new(false));
    let initial_reader_shutdown_expected = Arc::new(AtomicBool::new(false));

    let stdin_slot = Arc::new(Mutex::new(None));
    let child_slot = Arc::new(ChildSlot(Mutex::new(None)));
    let config = spawn_bun(
        entry_path,
        &app_args,
        &event_tx,
        &call_tx,
        Arc::clone(&initial_reader_shutdown_expected),
        &BunSlots {
            child: &child_slot,
            stdin: &stdin_slot,
        },
    )
    .unwrap_or_else(|error| {
        tynd_error!("{error}");
        std::process::exit(1);
    });

    if let Err(error) = spawn_stdin_forwarder(
        call_rx,
        Arc::clone(&stdin_slot),
        event_tx.clone(),
        Arc::clone(&reload_in_progress),
    ) {
        tynd_host::tynd_error!("Failed to start Bun stdin forwarder: {error}");
        std::process::exit(1);
    }

    let bridge = BackendBridge {
        config,
        call_tx: call_tx.clone(),
        event_rx,
    };
    let reload = ReloadHandle {
        entry_path: entry_path.to_string(),
        app_args,
        stdin_slot,
        child_slot,
        event_tx,
        call_tx,
        reload_lock: Arc::new(Mutex::new(())),
        reload_in_progress,
        reader_shutdown_expected: Arc::new(Mutex::new(initial_reader_shutdown_expected)),
    };
    (bridge, reload)
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

struct BunPipes {
    stdin: ChildStdin,
    stdout: ChildStdout,
    stderr: ChildStderr,
}

type StdinSlot = Arc<Mutex<Option<ChildStdin>>>;

/// The shared slots a Bun generation is published into. Readers get the child
/// slot weakly (see `StdoutReaderContext`).
struct BunSlots<'a> {
    child: &'a Arc<ChildSlot>,
    stdin: &'a StdinSlot,
}

impl BunSlots<'_> {
    /// Kill the child and make outbound calls fail fast.
    fn discard(&self) {
        self.child.terminate();
        *self
            .stdin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

/// Spawn a Bun subprocess, wait for `tynd:config`, spawn the stdout reader thread,
/// and publish the child and its stdin into `slots`. The child is registered
/// before the reader starts so the reader can kill it on a framing error; stdin
/// is only published once the config handshake succeeded.
fn spawn_bun(
    entry_path: &str,
    app_args: &[OsString],
    event_tx: &mpsc::Sender<BackendEvent>,
    call_tx: &mpsc::Sender<BackendCall>,
    shutdown_expected: Arc<AtomicBool>,
    slots: &BunSlots<'_>,
) -> Result<BackendConfig, String> {
    let mut child = spawn_bun_process(entry_path, app_args)?;
    let BunPipes {
        stdin,
        stdout,
        stderr,
    } = take_bun_pipes(&mut child)?;

    if let Err(error) = spawn_stderr_reader(stderr) {
        terminate_child(&mut child);
        return Err(format!("Failed to start Bun stderr reader: {error}"));
    }
    *slots.child.lock() = Some(child);

    let reader = BufReader::with_capacity(64 * 1024, stdout);
    let reader_event_tx = event_tx.clone();
    let reader_call_tx = call_tx.clone();
    let packaged_smoke = std::env::var_os("TYND_PACKAGED_SMOKE").is_some();
    let (config_tx, config_rx) = mpsc::bounded(1);
    let reader_context = StdoutReaderContext {
        event_tx: reader_event_tx,
        call_tx: reader_call_tx,
        config_tx,
        shutdown_expected,
        packaged_smoke,
        child_slot: Arc::downgrade(slots.child),
        stdin_slot: Arc::clone(slots.stdin),
    };
    if let Err(error) = spawn_stdout_reader(reader, reader_context) {
        slots.discard();
        return Err(format!("Failed to start Bun stdout reader: {error}"));
    }

    match wait_for_startup_config(&config_rx) {
        Ok(config) => {
            *slots
                .stdin
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(stdin);
            Ok(config)
        },
        Err(error) => {
            slots.discard();
            Err(error)
        },
    }
}

fn spawn_bun_process(entry_path: &str, app_args: &[OsString]) -> Result<Child, String> {
    let bun_bin = std::env::var("TYND_BUN_PATH").unwrap_or_else(|_| "bun".into());
    let mut command = Command::new(&bun_bin);
    let entry_for_child = absolute_entry_path(entry_path);
    command
        .arg("run")
        .arg(entry_path)
        // Everything after the entry belongs to the app (`process.argv`).
        .args(app_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // The runtime re-imports the user's module when dispatching backend functions.
    command.env("TYND_ENTRY", &entry_for_child);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    command.spawn().map_err(|error| {
        let mut message = format!("Failed to start runtime: {error}");
        if std::env::var("TYND_BUN_PATH").is_err() {
            message.push_str("\nEnsure runtime is installed and in PATH");
        }
        message
    })
}

fn take_bun_pipes(child: &mut Child) -> Result<BunPipes, String> {
    let Some(stdin) = child.stdin.take() else {
        terminate_child(child);
        return Err("Runtime started without a piped stdin handle".into());
    };
    let Some(stdout) = child.stdout.take() else {
        terminate_child(child);
        return Err("Runtime started without a piped stdout handle".into());
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_child(child);
        return Err("Runtime started without a piped stderr handle".into());
    };
    Ok(BunPipes {
        stdin,
        stdout,
        stderr,
    })
}

fn wait_for_startup_config(
    config_rx: &mpsc::Receiver<Result<BackendConfig, String>>,
) -> Result<BackendConfig, String> {
    match config_rx.recv_timeout(STARTUP_TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "Runtime did not send tynd:config within {} seconds",
            STARTUP_TIMEOUT.as_secs()
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("Runtime stdout reader stopped before sending tynd:config".into())
        },
    }
}

fn absolute_entry_path(entry_path: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(entry_path);
    if path.is_absolute() {
        return path;
    }
    std::env::current_dir().map_or(path.clone(), |cwd| cwd.join(path))
}

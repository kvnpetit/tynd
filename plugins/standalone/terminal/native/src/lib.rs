//! Embedded PTY plugin. One reader thread per session streams base64
//! `terminal:data`; a waiter emits `terminal:exit`.

#![forbid(unsafe_code)]

use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use crossbeam_channel::{bounded, Sender};
use parking_lot::{Condvar, Mutex};
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tynd_plugin_sdk::{
    join_with_timeout,
    tynd_plugin::{plugin, plugin_impl},
    JoinOutcome, PluginCx, PluginError, PluginResult, Registry,
};

const MAX_SESSIONS: usize = 64;

/// Input is cut into chunks of this size and queued for the per-session
/// writer thread, so a child that never reads stdin can only stall that
/// thread, never a shared call-pool worker or the session lock.
const INPUT_CHUNK_BYTES: usize = 8 * 1024;
/// Bounded queue depth (in chunks) between `write` and the writer thread.
const INPUT_QUEUE_CHUNKS: usize = 64;
/// How long one `write` call waits for queue space before failing.
const INPUT_ENQUEUE_TIMEOUT: Duration = Duration::from_secs(2);
/// How long the waiter lets the reader drain final output after the child
/// exits before it emits `terminal:exit` anyway.
const READER_DRAIN_TIMEOUT: Duration = Duration::from_secs(3);
/// Drain budget when the session was killed (exit event is suppressed).
const KILLED_DRAIN_TIMEOUT: Duration = Duration::from_millis(200);

/// Name prefix of the per-session reader thread.
const READER_THREAD_PREFIX: &str = "tynd-terminal-read-";

/// Cloned `ChildKiller` handle so `kill()` from the dispatch thread can
/// signal the child without locking the waiter thread's `Child`.
type SharedKiller = Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>;

#[derive(Default)]
struct ActivationGate {
    active: Mutex<bool>,
    changed: Condvar,
}

impl ActivationGate {
    fn wait(&self) {
        let mut active = self.active.lock();
        while !*active {
            self.changed.wait(&mut active);
        }
    }

    fn activate(&self) {
        let mut active = self.active.lock();
        if *active {
            return;
        }
        *active = true;
        self.changed.notify_all();
    }
}

struct Session {
    master: Box<dyn MasterPty + Send>,
    /// Feeds the session's writer thread; the thread exits when this is
    /// dropped (session removed) or a write fails.
    input: Sender<Vec<u8>>,
    /// Set by `kill()` so the watcher thread suppresses the synthetic
    /// `terminal:exit` event.
    killed: Arc<AtomicBool>,
    killer: SharedKiller,
    gate: Arc<ActivationGate>,
    workers: Arc<Mutex<SessionWorkers>>,
}

#[derive(Default)]
struct SessionWorkers {
    reader: Option<JoinHandle<()>>,
    waiter: Option<JoinHandle<()>>,
}

/// `spawn` return shape.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpawnResult {
    #[specta(type = f64)]
    pub id: u64,
}

/// Starts the writer thread that owns the PTY input handle and returns the
/// queue feeding it. Every blocking `write_all` happens on this thread.
fn spawn_input_writer(mut writer: Box<dyn Write + Send>) -> std::io::Result<Sender<Vec<u8>>> {
    let (tx, rx) = bounded::<Vec<u8>>(INPUT_QUEUE_CHUNKS);
    std::thread::Builder::new()
        .name("tynd-terminal-write".into())
        .spawn(move || {
            for chunk in rx {
                if writer
                    .write_all(&chunk)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    // Child gone or pipe closed: dropping `rx` makes later
                    // `write` calls fail fast instead of queueing.
                    break;
                }
            }
        })?;
    Ok(tx)
}

/// Queues `bytes` for the writer thread without ever blocking past
/// `timeout`, preserving order.
fn enqueue_input(input: &Sender<Vec<u8>>, bytes: &[u8], timeout: Duration) -> PluginResult<()> {
    let deadline = Instant::now() + timeout;
    for chunk in bytes.chunks(INPUT_CHUNK_BYTES) {
        input
            .send_deadline(chunk.to_vec(), deadline)
            .map_err(|error| match error {
                crossbeam_channel::SendTimeoutError::Timeout(_) => PluginError::Custom(
                    "terminal.write: input buffer full (the process is not reading stdin)".into(),
                ),
                crossbeam_channel::SendTimeoutError::Disconnected(_) => {
                    PluginError::Custom("terminal.write: session input is closed".into())
                },
            })?;
    }
    Ok(())
}

/// End-of-session sequencing for the waiter thread, after the child exited.
///
/// `release` removes the session (dropping the PTY master). On Windows ConPTY
/// keeps the output pipe open until the master is closed, so the master must
/// be released *before* waiting for the reader; elsewhere the slave closing
/// gives EOF on its own, and the session is released after the drain. The
/// drain wait is bounded so a grandchild holding the tty cannot suppress the
/// exit event forever. Returns whether the reader finished within `timeout`.
fn finish_session(
    release_master_first: bool,
    reader_done: &Receiver<()>,
    timeout: Duration,
    release: impl FnOnce(),
) -> bool {
    if release_master_first {
        release();
        wait_reader_drained(reader_done, timeout)
    } else {
        let drained = wait_reader_drained(reader_done, timeout);
        release();
        drained
    }
}

fn wait_reader_drained(reader_done: &Receiver<()>, timeout: Duration) -> bool {
    match reader_done.recv_timeout(timeout) {
        Ok(()) | Err(RecvTimeoutError::Disconnected) => true,
        Err(RecvTimeoutError::Timeout) => false,
    }
}

fn check_process(
    cx: &PluginCx,
    shell: &str,
    argv: &[String],
    cwd: Option<&String>,
    env: Option<&HashMap<String, String>>,
) -> PluginResult<Option<String>> {
    let payload = json!({
        "binary": shell,
        "args": argv,
        "cwd": cwd,
        "env": env,
    });
    // Fail closed: a host without the security dispatcher must never turn a
    // missing hook into permission to spawn an arbitrary process.
    let checked = cx.os_call("security", "checkProcess", payload)?;
    match checked.get("cwd") {
        Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(cwd)) => Ok(Some(cwd.clone())),
        _ => Err(PluginError::Custom(
            "security.checkProcess: host returned an invalid resolved cwd".to_string(),
        )),
    }
}

struct SessionThreadContext {
    id: u64,
    sessions: Arc<Registry<Session>>,
    killed: Arc<AtomicBool>,
    killer: SharedKiller,
    gate: Arc<ActivationGate>,
    workers: Arc<Mutex<SessionWorkers>>,
}

fn start_waiter_thread(
    cx: &PluginCx,
    context: &SessionThreadContext,
    child: Box<dyn Child + Send + Sync>,
    reader_done: Receiver<()>,
) -> PluginResult<()> {
    // Keep ownership outside the closure until the thread starts so spawn
    // failure can still kill and reap the child.
    let child_slot = Arc::new(Mutex::new(Some(child)));
    let waiter_child = Arc::clone(&child_slot);
    let waiter_emit = cx.emitter();
    let sessions = Arc::clone(&context.sessions);
    let waiter_killed = Arc::clone(&context.killed);
    let waiter_gate = Arc::clone(&context.gate);
    let id = context.id;
    let waiter_result = std::thread::Builder::new()
        .name(format!("tynd-terminal-wait-{id}"))
        .spawn(move || {
            waiter_gate.wait();
            let code = waiter_child.lock().take().and_then(|mut child| {
                child.wait().ok().map(|status| {
                    if status.success() {
                        0u32
                    } else {
                        status.exit_code()
                    }
                })
            });
            // Reader sends only after its final data event. Waiting here makes
            // `terminal:exit` a terminal event for PTY output (bounded).
            let drain_timeout = if waiter_killed.load(Ordering::Acquire) {
                KILLED_DRAIN_TIMEOUT
            } else {
                READER_DRAIN_TIMEOUT
            };
            let drained = finish_session(cfg!(windows), &reader_done, drain_timeout, || {
                sessions.remove(id);
            });
            if !drained {
                tracing::warn!(
                    id,
                    "terminal: reader still open after child exit; emitting exit"
                );
            }
            if !waiter_killed.load(Ordering::Acquire) {
                waiter_emit("terminal:exit", json!({ "id": id, "code": code }));
            }
        });
    match waiter_result {
        Ok(waiter) => context.workers.lock().waiter = Some(waiter),
        Err(error) => {
            context.sessions.remove(id);
            context.gate.activate();
            let _ = context.killer.lock().kill();
            if let Some(mut child) = child_slot.lock().take() {
                let _ = child.wait();
            }
            return Err(PluginError::Custom(format!(
                "terminal.spawn: failed to start waiter: {error}"
            )));
        },
    }
    Ok(())
}

fn start_reader_thread(
    cx: &PluginCx,
    context: &SessionThreadContext,
    reader: Box<dyn Read + Send>,
    reader_done: SyncSender<()>,
) -> PluginResult<()> {
    let emit = cx.emitter();
    let reader_gate = Arc::clone(&context.gate);
    let reader_killed = Arc::clone(&context.killed);
    let id = context.id;
    let reader_result = std::thread::Builder::new()
        .name(format!("{READER_THREAD_PREFIX}{id}"))
        .spawn(move || {
            reader_gate.wait();
            let mut reader = reader;
            let mut buf = vec![0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    // Killed sessions emit no more output; this also lets a
                    // reader held open by a grandchild on the tty stop at its
                    // next wake-up instead of streaming into a dead session.
                    Ok(_) if reader_killed.load(Ordering::Acquire) => break,
                    Ok(n) => {
                        let encoded = STANDARD.encode(&buf[..n]);
                        emit("terminal:data", json!({ "id": id, "data": encoded }));
                    },
                }
            }
            let _ = reader_done.send(());
        });
    match reader_result {
        Ok(reader) => context.workers.lock().reader = Some(reader),
        Err(error) => {
            context.killed.store(true, Ordering::Release);
            context.sessions.remove(id);
            context.gate.activate();
            let _ = context.killer.lock().kill();
            return Err(PluginError::Custom(format!(
                "terminal.spawn: failed to start reader: {error}"
            )));
        },
    }
    Ok(())
}
#[plugin(name = "terminal")]
pub struct TerminalPlugin {
    // `Arc` so the PTY-exit-waiter thread owns its own handle.
    sessions: Arc<Registry<Session>>,
}

impl std::fmt::Debug for TerminalPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalPlugin").finish_non_exhaustive()
    }
}

impl Default for TerminalPlugin {
    fn default() -> Self {
        Self {
            sessions: Arc::new(Registry::new(MAX_SESSIONS)),
        }
    }
}

#[plugin_impl]
impl TerminalPlugin {
    /// Spawn a new PTY-backed shell session. Every field is optional with
    /// a platform-sensible default (login shell from `$SHELL` /
    /// `%COMSPEC%`, 80x24, current cwd).
    #[command]
    pub fn spawn(
        &self,
        cx: &PluginCx,
        shell: Option<String>,
        args: Option<Vec<String>>,
        cwd: Option<String>,
        env: Option<HashMap<String, String>>,
        cols: Option<u16>,
        rows: Option<u16>,
    ) -> PluginResult<SpawnResult> {
        let shell = shell.unwrap_or_else(default_shell);
        let argv = args.unwrap_or_default();
        let cols = cols.unwrap_or(80);
        let rows = rows.unwrap_or(24);

        // Capability gate — defer the binary/args policy check to the host
        // (same pattern as the sql plugin's `open`, which defers its path
        // check to the host's `checkFs`). The host can deny based on
        // allowlists, env, etc.
        // Missing dispatchers and configured policy denials both fail closed.
        let cwd = check_process(cx, &shell, &argv, cwd.as_ref(), env.as_ref())?;

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PluginError::Custom(format!("terminal.spawn: openpty: {e}")))?;

        let mut cmd = CommandBuilder::new(&shell);
        cmd.args(&argv);
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }
        if let Some(env_map) = env {
            for (k, v) in env_map {
                cmd.env(k, v);
            }
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PluginError::Custom(format!("terminal.spawn: spawn: {e}")))?;
        drop(pair.slave);

        // portable-pty does not kill on drop: any early return after a
        // successful spawn must reap the child itself.
        let mut child = child;
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PluginError::Custom(format!(
                    "terminal.spawn: clone reader: {e}"
                )));
            },
        };
        let writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PluginError::Custom(format!(
                    "terminal.spawn: take writer: {e}"
                )));
            },
        };
        let input = match spawn_input_writer(writer) {
            Ok(input) => input,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PluginError::Custom(format!(
                    "terminal.spawn: failed to start writer: {error}"
                )));
            },
        };

        // Take an independent killer *before* moving `child` into the
        // waiter thread, so kill() never locks on the live Child.
        let killer: SharedKiller = Arc::new(Mutex::new(child.clone_killer()));

        let killed = Arc::new(AtomicBool::new(false));
        let gate = Arc::new(ActivationGate::default());
        let workers = Arc::new(Mutex::new(SessionWorkers::default()));
        let id = match self.sessions.insert(Session {
            master: pair.master,
            input,
            killed: Arc::clone(&killed),
            killer: Arc::clone(&killer),
            gate: Arc::clone(&gate),
            workers: Arc::clone(&workers),
        }) {
            Ok(id) => id,
            Err(error) => {
                let _ = killer.lock().kill();
                let _ = child.wait();
                return Err(error);
            },
        };

        let thread_context = SessionThreadContext {
            id,
            sessions: Arc::clone(&self.sessions),
            killed,
            killer,
            gate,
            workers,
        };
        let (reader_done_tx, reader_done_rx) = sync_channel(1);
        start_waiter_thread(cx, &thread_context, child, reader_done_rx)?;
        start_reader_thread(cx, &thread_context, reader, reader_done_tx)?;

        Ok(SpawnResult { id })
    }

    /// Release a reserved session after frontend data and exit listeners are
    /// installed. Idempotent while the session remains live.
    #[command]
    pub fn activate(&self, id: u64) -> PluginResult<()> {
        self.sessions
            .with_arg(id, "sessionId", "terminal.activate", |session| {
                session.gate.activate();
                Ok(())
            })
    }

    /// Push base64-encoded bytes into the PTY master (stdin).
    #[command]
    pub fn write(&self, id: u64, data: String) -> PluginResult<()> {
        let bytes = STANDARD
            .decode(data.as_bytes())
            .map_err(|e| PluginError::Custom(format!("terminal.write: invalid base64: {e}")))?;

        // Clone the queue handle and release the session lock before
        // queueing: the actual PTY write happens on the writer thread.
        let input = self
            .sessions
            .with_arg(id, "sessionId", "terminal.write", |session| {
                Ok(session.input.clone())
            })?;
        enqueue_input(&input, &bytes, INPUT_ENQUEUE_TIMEOUT)
    }

    /// Sends `TIOCSWINSZ` (or the Windows ConPTY equivalent).
    #[command]
    pub fn resize(&self, id: u64, cols: Option<u16>, rows: Option<u16>) -> PluginResult<()> {
        let cols = cols.unwrap_or(80);
        let rows = rows.unwrap_or(24);

        self.sessions
            .with_arg(id, "sessionId", "terminal.resize", |session| {
                session
                    .master
                    .resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .map_err(|e| PluginError::Custom(format!("terminal.resize: {e}")))?;
                Ok(())
            })
    }

    /// Idempotent. Suppresses the synthetic `terminal:exit` event.
    #[command]
    pub fn kill(&self, id: u64) -> PluginResult<()> {
        // Snapshot killer + flip `killed` *before* removing — otherwise
        // the waiter could fire a synthetic exit before we suppress it.
        // `kill` is idempotent so missing-id snapshots are ignored.
        let snapshot: PluginResult<(SharedKiller, Arc<ActivationGate>)> =
            self.sessions.with(id, |session| {
                session.killed.store(true, Ordering::Release);
                Ok((Arc::clone(&session.killer), Arc::clone(&session.gate)))
            });

        if let Ok((killer, gate)) = snapshot {
            // Release a reserved session so waiter and reader can terminate.
            gate.activate();
            // Plain `lock`: the killer mutex is only held for the duration of
            // a kill, so skipping on contention would leave the child alive.
            if let Err(e) = killer.lock().kill() {
                tracing::warn!(id, error = %e, "terminal.kill: SIGKILL failed (child may already be gone)");
            }
        }

        self.sessions.remove(id);
        Ok(())
    }

    /// Every active session id.
    #[command]
    pub fn list(&self) -> PluginResult<Vec<u64>> {
        Ok(self.sessions.ids())
    }

    /// Native host lifecycle hook. Kill every child before the process drops
    /// PTY handles so shells cannot outlive a graceful application exit.
    pub fn plugin_shutdown(&self, _cx: &PluginCx) -> PluginResult<()> {
        let mut workers = Vec::new();
        let sessions = self
            .sessions
            .snapshot(|id, session| Ok((id, Arc::clone(&session.workers))));
        for (id, session_workers) in sessions {
            self.kill(id)?;
            workers.push(session_workers);
        }

        // Signal every child first, then share one global deadline across all
        // joins. A pathological session cannot multiply the shutdown timeout
        // by MAX_SESSIONS.
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut unfinished = Vec::new();
        for session_workers in workers {
            let handles = {
                let mut workers = session_workers.lock();
                [workers.reader.take(), workers.waiter.take()]
            };
            for handle in handles.into_iter().flatten() {
                let name = handle
                    .thread()
                    .name()
                    .unwrap_or("terminal-worker")
                    .to_string();
                let remaining = deadline.saturating_duration_since(Instant::now());
                match join_with_timeout(handle, remaining) {
                    JoinOutcome::Finished(()) => {},
                    JoinOutcome::Panicked => unfinished.push(format!("{name} panicked")),
                    JoinOutcome::TimedOut(_) if name.starts_with(READER_THREAD_PREFIX) => {
                        // A grandchild still holding the tty keeps a blocked
                        // `read` alive after the shell was killed (Unix). The
                        // thread holds no resource we can release and ends
                        // with that process, so it is not a shutdown failure.
                        tracing::warn!(thread = %name, "terminal: reader still blocked after kill; detaching");
                    },
                    JoinOutcome::TimedOut(_) => {
                        unfinished.push(format!("{name} did not stop within 2s"));
                    },
                }
            }
        }
        if !unfinished.is_empty() {
            return Err(PluginError::Custom(format!(
                "terminal.shutdown: {}",
                unfinished.join("; ")
            )));
        }
        Ok(())
    }
}

fn default_shell() -> String {
    #[cfg(target_os = "windows")]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use super::*;

    /// Writer that records bytes, optionally blocking on a gate first.
    struct GatedWriter {
        gate: Option<Receiver<()>>,
        sink: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for GatedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if let Some(gate) = &self.gate {
                let _ = gate.recv();
            }
            self.sink.lock().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn check_process_fails_closed_without_security_dispatcher() {
        let cx = PluginCx::test();
        assert!(check_process(&cx, "sh", &[], None, None).is_err());
    }

    #[test]
    fn input_reaches_writer_in_order() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let tx = spawn_input_writer(Box::new(GatedWriter {
            gate: None,
            sink: Arc::clone(&sink),
        }))
        .unwrap();
        let payload: Vec<u8> = (0..30_000u32).map(|n| (n % 251) as u8).collect();
        enqueue_input(&tx, &payload, Duration::from_secs(2)).unwrap();
        drop(tx);
        let deadline = Instant::now() + Duration::from_secs(2);
        while sink.lock().len() < payload.len() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*sink.lock(), payload);
    }

    #[test]
    fn stuck_child_never_blocks_the_caller_beyond_the_timeout() {
        let (_gate_tx, gate_rx) = channel::<()>();
        let sink = Arc::new(Mutex::new(Vec::new()));
        let tx = spawn_input_writer(Box::new(GatedWriter {
            gate: Some(gate_rx),
            sink,
        }))
        .unwrap();
        let big = vec![0u8; INPUT_CHUNK_BYTES * (INPUT_QUEUE_CHUNKS + 8)];
        let started = Instant::now();
        let result = enqueue_input(&tx, &big, Duration::from_millis(150));
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn write_after_writer_exit_fails_fast() {
        struct FailingWriter;
        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("closed"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let tx = spawn_input_writer(Box::new(FailingWriter)).unwrap();
        let _ = enqueue_input(&tx, b"x", Duration::from_secs(1));
        let deadline = Instant::now() + Duration::from_secs(2);
        let err = loop {
            match enqueue_input(&tx, b"y", Duration::from_millis(50)) {
                Err(error) => break error,
                Ok(()) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                },
                Ok(()) => panic!("writer exit was never observed"),
            }
        };
        assert!(err.to_string().contains("closed"));
    }

    #[test]
    fn windows_order_releases_master_before_waiting_for_reader() {
        let (done_tx, done_rx) = sync_channel::<()>(1);
        let released = Arc::new(AtomicBool::new(false));
        let released_by_release = Arc::clone(&released);
        // The reader only finishes once the master has been released, like
        // ConPTY closing its output pipe.
        let reader = std::thread::spawn({
            let released = Arc::clone(&released);
            move || {
                while !released.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                let _ = done_tx.send(());
            }
        });
        let drained = finish_session(true, &done_rx, Duration::from_secs(2), move || {
            released_by_release.store(true, Ordering::Release);
        });
        assert!(drained);
        reader.join().unwrap();
    }

    #[test]
    fn unix_order_waits_for_reader_then_releases() {
        let (done_tx, done_rx) = sync_channel::<()>(1);
        let released = Arc::new(AtomicBool::new(false));
        done_tx.send(()).unwrap();
        let released_flag = Arc::clone(&released);
        let drained = finish_session(false, &done_rx, Duration::from_secs(1), move || {
            released_flag.store(true, Ordering::Release);
        });
        assert!(drained);
        assert!(released.load(Ordering::Acquire));
    }

    #[test]
    fn drain_wait_is_bounded_and_still_releases() {
        let (_done_tx, done_rx) = sync_channel::<()>(1);
        let released = Arc::new(AtomicBool::new(false));
        let released_flag = Arc::clone(&released);
        let started = Instant::now();
        let drained = finish_session(false, &done_rx, Duration::from_millis(50), move || {
            released_flag.store(true, Ordering::Release);
        });
        assert!(!drained);
        assert!(released.load(Ordering::Acquire));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn disconnected_reader_counts_as_drained() {
        let (done_tx, done_rx) = sync_channel::<()>(1);
        drop(done_tx);
        assert!(wait_reader_drained(&done_rx, Duration::from_millis(10)));
    }
}

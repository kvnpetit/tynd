//! systemd-logind DBus watchers owned by one supervisor. The supervisor
//! verifies all three subscriptions before readiness, reconnects after a
//! worker exits, and performs potentially blocking zbus close off the host
//! shutdown thread.
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

use serde_json::json;
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::OwnedObjectPath,
};

use super::{join_workers, MONITOR_START_TIMEOUT};
use crate::ipc::events;

const SUPERVISOR_POLL: Duration = Duration::from_millis(50);
const RECONNECT_DELAY: Duration = Duration::from_millis(250);

pub(super) struct Monitor {
    stopping: Arc<AtomicBool>,
    join: std::thread::JoinHandle<()>,
}

impl Monitor {
    pub(super) fn shutdown(self) {
        self.stopping.store(true, Ordering::Release);
        join_workers(vec![self.join], "power[linux]");
    }
}

impl super::MonitorHandle for Monitor {
    fn shutdown(self) {
        Monitor::shutdown(self);
    }

    fn is_alive(&self) -> bool {
        !self.join.is_finished()
    }
}

pub(super) fn spawn_watcher() -> Result<Monitor, String> {
    let stopping = Arc::new(AtomicBool::new(false));
    let worker_stopping = Arc::clone(&stopping);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let join = std::thread::Builder::new()
        .name("tynd-power-linux-supervisor".into())
        .spawn(move || supervise(worker_stopping, ready_tx))
        .map_err(|error| format!("power[linux]: supervisor spawn failed: {error}"))?;

    match ready_rx.recv_timeout(MONITOR_START_TIMEOUT) {
        Ok(Ok(())) => Ok(Monitor { stopping, join }),
        Ok(Err(error)) => {
            stopping.store(true, Ordering::Release);
            join_workers(vec![join], "power[linux]");
            Err(error)
        },
        Err(error) => {
            stopping.store(true, Ordering::Release);
            drop(ready_rx);
            join_workers(vec![join], "power[linux]");
            Err(format!("power[linux]: watcher readiness failed: {error}"))
        },
    }
}

enum GenerationEnd {
    Stopped,
    Disconnected,
}

fn supervise(stopping: Arc<AtomicBool>, ready: mpsc::SyncSender<Result<(), String>>) {
    let mut startup = Some(ready);
    loop {
        if stopping.load(Ordering::Acquire) {
            return;
        }

        match run_generation(&stopping, &mut startup) {
            Ok(GenerationEnd::Stopped) => return,
            Ok(GenerationEnd::Disconnected) => {
                crate::tynd_warn!("power[linux]: logind watcher disconnected; reconnecting");
            },
            Err(error) => {
                if let Some(ready) = startup.take() {
                    let _ = ready.send(Err(error));
                    return;
                }
                crate::tynd_warn!("power[linux]: reconnect failed: {error}");
            },
        }

        if wait_for_stop(&stopping, RECONNECT_DELAY) {
            return;
        }
    }
}

/// Signal workers per generation: sleep, shutdown, lock, unlock.
const WORKERS: usize = 4;

fn run_generation(
    stopping: &Arc<AtomicBool>,
    startup: &mut Option<mpsc::SyncSender<Result<(), String>>>,
) -> Result<GenerationEnd, String> {
    let connection = Connection::system().map_err(|error| format!("system bus: {error}"))?;
    let session_path = resolve_session(&connection)?;
    if stopping.load(Ordering::Acquire) {
        let _ = connection.close();
        return Ok(GenerationEnd::Stopped);
    }

    let (worker_ready_tx, worker_ready_rx) = mpsc::sync_channel(WORKERS);
    let mut joins = Vec::with_capacity(WORKERS);

    if let Err(error) = spawn_generation_workers(
        &connection,
        session_path,
        stopping,
        &mut joins,
        worker_ready_tx,
    ) {
        close_generation(connection, stopping, joins);
        return Err(error);
    }

    let deadline = Instant::now() + MONITOR_START_TIMEOUT;
    for _ in 0..WORKERS {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match worker_ready_rx.recv_timeout(remaining) {
            Ok(Ok(())) => {},
            Ok(Err(error)) => {
                close_generation(connection, stopping, joins);
                return Err(error);
            },
            Err(error) => {
                close_generation(connection, stopping, joins);
                return Err(format!("worker readiness failed: {error}"));
            },
        }
    }

    if let Some(ready) = startup.take() {
        if ready.send(Ok(())).is_err() {
            stopping.store(true, Ordering::Release);
        }
    }

    while !stopping.load(Ordering::Acquire) && joins.iter().all(|worker| !worker.is_finished()) {
        std::thread::sleep(SUPERVISOR_POLL);
    }
    let disconnected = !stopping.load(Ordering::Acquire);
    close_generation(connection, stopping, joins);
    Ok(if disconnected {
        GenerationEnd::Disconnected
    } else {
        GenerationEnd::Stopped
    })
}

/// Spawn the four signal workers of one generation. On error, the workers
/// already spawned stay in `joins` so the caller can close the generation.
fn spawn_generation_workers(
    connection: &Connection,
    session_path: OwnedObjectPath,
    stopping: &Arc<AtomicBool>,
    joins: &mut Vec<std::thread::JoinHandle<()>>,
    worker_ready_tx: mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let sleep_connection = connection.clone();
    spawn_worker(
        joins,
        "tynd-power-linux-sleep",
        Arc::clone(stopping),
        worker_ready_tx.clone(),
        move |ready| {
            run_manager_signal(
                sleep_connection,
                "PrepareForSleep",
                "power:suspend",
                Some("power:resume"),
                ready,
            )
        },
    )?;

    let shutdown_connection = connection.clone();
    spawn_worker(
        joins,
        "tynd-power-linux-shutdown",
        Arc::clone(stopping),
        worker_ready_tx.clone(),
        move |ready| {
            run_manager_signal(
                shutdown_connection,
                "PrepareForShutdown",
                "power:shutdown",
                None,
                ready,
            )
        },
    )?;

    let lock_connection = connection.clone();
    let lock_path = session_path.clone();
    spawn_worker(
        joins,
        "tynd-power-linux-lock",
        Arc::clone(stopping),
        worker_ready_tx.clone(),
        move |ready| run_session_signal(lock_connection, lock_path, "Lock", "power:lock", ready),
    )?;

    let unlock_connection = connection.clone();
    spawn_worker(
        joins,
        "tynd-power-linux-unlock",
        Arc::clone(stopping),
        worker_ready_tx,
        move |ready| {
            run_session_signal(
                unlock_connection,
                session_path,
                "Unlock",
                "power:unlock",
                ready,
            )
        },
    )
}

fn spawn_worker<F>(
    joins: &mut Vec<std::thread::JoinHandle<()>>,
    name: &'static str,
    stopping: Arc<AtomicBool>,
    ready: mpsc::SyncSender<Result<(), String>>,
    task: F,
) -> Result<(), String>
where
    F: FnOnce(&mpsc::SyncSender<Result<(), String>>) -> Result<(), String> + Send + 'static,
{
    let join = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            if let Err(error) = task(&ready) {
                let _ = ready.send(Err(error.clone()));
                if !stopping.load(Ordering::Acquire) {
                    crate::tynd_warn!("power[linux]: {name} exited: {error}");
                }
            }
        })
        .map_err(|error| format!("power[linux]: spawn {name}: {error}"))?;
    joins.push(join);
    Ok(())
}

fn close_generation(
    connection: Connection,
    stopping: &AtomicBool,
    joins: Vec<std::thread::JoinHandle<()>>,
) {
    if let Err(error) = connection.close() {
        if !stopping.load(Ordering::Acquire) {
            crate::tynd_warn!("power[linux]: close system bus failed: {error}");
        }
    }
    join_workers(joins, "power[linux]-generation");
}

fn wait_for_stop(stopping: &AtomicBool, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    while !stopping.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(SUPERVISOR_POLL);
    }
    stopping.load(Ordering::Acquire)
}

fn resolve_session(connection: &Connection) -> Result<OwnedObjectPath, String> {
    let manager = Proxy::new(
        connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .map_err(|error| format!("Manager proxy: {error}"))?;
    let pid = std::process::id();
    manager
        .call("GetSessionByPID", &(pid))
        .map_err(|error| format!("GetSessionByPID({pid}): {error}"))
}

/// logind `Manager` signal carrying a `bool` (`true` = about to happen,
/// `false` = done): emits `on_true` / `on_false`.
fn run_manager_signal(
    connection: Connection,
    signal: &'static str,
    on_true: &'static str,
    on_false: Option<&'static str>,
    ready: &mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let proxy = Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .map_err(|error| format!("Manager proxy: {error}"))?;
    let signals = proxy
        .receive_signal(signal)
        .map_err(|error| format!("receive_signal {signal}: {error}"))?;
    ready
        .send(Ok(()))
        .map_err(|_| format!("{signal} readiness receiver closed"))?;
    for message in signals {
        match message.body().deserialize::<bool>() {
            Ok(true) => events::emit(on_true, &json!({})),
            Ok(false) => {
                if let Some(event) = on_false {
                    events::emit(event, &json!({}));
                }
            },
            Err(error) => {
                crate::tynd_warn!("power[linux]: {signal} body: {error}");
            },
        }
    }
    Ok(())
}

fn run_session_signal(
    connection: Connection,
    session_path: OwnedObjectPath,
    signal: &'static str,
    event: &'static str,
    ready: &mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let proxy = Proxy::new(
        &connection,
        "org.freedesktop.login1",
        session_path.as_str(),
        "org.freedesktop.login1.Session",
    )
    .map_err(|error| format!("Session proxy ({signal}): {error}"))?;
    let signals = proxy
        .receive_signal(signal)
        .map_err(|error| format!("receive_signal {signal}: {error}"))?;
    ready
        .send(Ok(()))
        .map_err(|_| format!("{signal} readiness receiver closed"))?;
    for _message in signals {
        events::emit(event, &json!({}));
    }
    Ok(())
}

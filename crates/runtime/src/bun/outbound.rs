//! Bun stdin transport: coalesce calls, encode frames, and reject calls when pipe dies.

use std::{
    io::Write,
    process::ChildStdin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use crossbeam_channel as mpsc;
use tynd_protocol::runtime::{BackendCall, BackendEvent, BackendReturnPayload};

use super::wire::{extract_raw_call_id, push_call_line};
pub(super) fn spawn_stdin_forwarder(
    call_rx: mpsc::Receiver<BackendCall>,
    stdin_slot: Arc<Mutex<Option<ChildStdin>>>,
    event_tx: mpsc::Sender<BackendEvent>,
    reload_in_progress: Arc<AtomicBool>,
) -> std::io::Result<()> {
    let packaged_smoke = std::env::var_os("TYND_PACKAGED_SMOKE").is_some();
    std::thread::Builder::new()
        .name("tynd-bun-stdin".into())
        .spawn(move || {
            run_stdin_forwarder(
                &call_rx,
                &stdin_slot,
                &event_tx,
                &reload_in_progress,
                packaged_smoke,
            );
        })
        .map(|_| ())
}

fn run_stdin_forwarder(
    call_rx: &mpsc::Receiver<BackendCall>,
    stdin_slot: &Mutex<Option<ChildStdin>>,
    event_tx: &mpsc::Sender<BackendEvent>,
    reload_in_progress: &AtomicBool,
    packaged_smoke: bool,
) {
    let mut burst = Vec::with_capacity(8 * 1024);
    while let Ok(call) = call_rx.recv() {
        burst.clear();
        // Ids of calls actually encoded into `burst`. A call whose encoding
        // failed was already answered with an error and must not be rejected a
        // second time if the write fails.
        let mut call_ids = Vec::new();
        append_call_or_report(&mut burst, &mut call_ids, call, event_tx);
        while let Ok(extra) = call_rx.try_recv() {
            append_call_or_report(&mut burst, &mut call_ids, extra, event_tx);
            if burst.len() >= 1024 * 1024 {
                break;
            }
        }
        if burst.is_empty() {
            continue;
        }
        write_call_burst(
            stdin_slot,
            event_tx,
            reload_in_progress,
            packaged_smoke,
            &burst,
            &call_ids,
        );
    }
}

fn write_call_burst(
    stdin_slot: &Mutex<Option<ChildStdin>>,
    event_tx: &mpsc::Sender<BackendEvent>,
    reload_in_progress: &AtomicBool,
    packaged_smoke: bool,
    burst: &[u8],
    call_ids: &[String],
) {
    let mut guard = stdin_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(stdin) = guard.as_mut() {
        if let Err(error) = stdin.write_all(burst) {
            let reloading = reload_in_progress.load(Ordering::SeqCst);
            let rejection = if reloading {
                "BACKEND_DEAD: backend runtime pipe closed during reload"
            } else {
                "BACKEND_DEAD: backend runtime stdin closed unexpectedly"
            };
            reject_backend_calls(event_tx, call_ids, rejection);
            if !reloading && !packaged_smoke {
                tynd_host::tynd_error!("Bun stdin write failed — IPC channel is dead: {error}");
                let _ = event_tx.send(BackendEvent::Error {
                    message:
                        "Backend runtime stdin closed unexpectedly. Restart the host to recover."
                            .to_string(),
                });
            }
            // Drop the broken stdin so subsequent bursts don't re-trigger the same error.
            *guard = None;
        }
        return;
    }

    let message = if reload_in_progress.load(Ordering::SeqCst) {
        "BACKEND_DEAD: backend runtime is reloading"
    } else {
        "BACKEND_DEAD: backend runtime is unavailable"
    };
    reject_backend_calls(event_tx, call_ids, message);
}

/// Append the `\n`-terminated JSON frame of one `BackendCall` to the
/// coalescing buffer. A call that cannot be encoded is answered with an error
/// instead.
fn append_call_or_report(
    out: &mut Vec<u8>,
    encoded_ids: &mut Vec<String>,
    call: BackendCall,
    event_tx: &mpsc::Sender<BackendEvent>,
) {
    let call_id = backend_call_id(&call);
    match push_call_line(out, call) {
        Ok(()) => encoded_ids.extend(call_id),
        Err(error) => {
            tynd_host::tynd_error!("Bun IPC outbound frame rejected: {error}");
            if let Some(id) = call_id {
                let _ = event_tx.send(BackendEvent::Return {
                    id,
                    payload: BackendReturnPayload::Err(error),
                });
            }
        },
    }
}

fn backend_call_id(call: &BackendCall) -> Option<String> {
    match call {
        BackendCall::Typed { id, .. } => Some(id.clone()),
        BackendCall::Raw(body) => extract_raw_call_id(body).map(str::to_string),
        // The host answers the webview request with a 503 on a `Return` error.
        BackendCall::Protocol(request) => Some(request.id.clone()),
        _ => None,
    }
}

fn reject_backend_calls(event_tx: &mpsc::Sender<BackendEvent>, call_ids: &[String], message: &str) {
    for id in call_ids {
        let _ = event_tx.send(BackendEvent::Return {
            id: id.clone(),
            payload: BackendReturnPayload::Err(message.to_string()),
        });
    }
}

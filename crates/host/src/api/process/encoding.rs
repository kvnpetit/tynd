use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use parking_lot::Mutex;

#[derive(Clone, Copy)]
pub(super) enum OutputEncoding {
    Utf8,
    Base64,
}

/// A running output reader plus the bytes it has collected so far. The buffer
/// is shared so a reader still blocked on a pipe held by a background
/// descendant (`server &`) can be abandoned without losing what the direct
/// child already wrote.
pub(super) struct OutputReader {
    handle: thread::JoinHandle<Result<(), String>>,
    buffer: Arc<Mutex<Vec<u8>>>,
}

pub(super) fn read_limited_output<R: Read + Send + 'static>(
    mut reader: R,
    stream: &'static str,
    max_bytes: usize,
    exceeded: Arc<AtomicBool>,
) -> std::io::Result<OutputReader> {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::clone(&buffer);
    let handle = thread::Builder::new()
        .name(format!("tynd-process-{stream}"))
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) => return Ok(()),
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(format!("{stream} read failed: {e}")),
                };
                let mut out = shared.lock();
                if out.len().saturating_add(n) > max_bytes {
                    exceeded.store(true, Ordering::Release);
                    return Err(format!("{stream} exceeded {max_bytes}-byte cap"));
                }
                out.extend_from_slice(&buf[..n]);
            }
        })?;
    Ok(OutputReader { handle, buffer })
}

fn collect(reader: OutputReader, stream: &str) -> Result<Vec<u8>, String> {
    reader
        .handle
        .join()
        .map_err(|_| format!("{stream} reader panicked"))??;
    Ok(std::mem::take(&mut *reader.buffer.lock()))
}

pub(super) fn join_output(reader: Option<OutputReader>, stream: &str) -> Result<Vec<u8>, String> {
    match reader {
        Some(reader) => collect(reader, stream),
        None => Ok(Vec::new()),
    }
}

/// Collect a reader's output, waiting for pipe EOF until `deadline`. A reader
/// only finishes at EOF, and a descendant that inherited the pipe (`server &`,
/// a forked daemon) keeps it open after the direct child is gone. Past the
/// deadline the call still succeeds with what was read so far (the direct
/// child already exited); the thread is detached and ends when the descendant
/// closes the pipe. Never waiting unbounded keeps the call-pool worker free.
pub(super) fn join_output_until(
    reader: Option<OutputReader>,
    stream: &str,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    while !reader.handle.is_finished() {
        if Instant::now() >= deadline {
            return Ok(reader.buffer.lock().clone());
        }
        thread::sleep(Duration::from_millis(5));
    }
    collect(reader, stream)
}

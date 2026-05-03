//! Lite-only: isolated QuickJS runtime per worker on its own OS thread.
//! Full uses `Bun.Worker` from TS instead. Input/output ship as JSON.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, OnceLock,
};

use dashmap::DashMap;
use rquickjs::{Context, Function, Runtime};
use serde_json::{json, Value};

use crate::{ipc_error::IpcError, tynd};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

enum Msg {
    Run {
        input: String,
        reply: mpsc::Sender<Result<String, String>>,
    },
    Stop,
}

type Workers = DashMap<u64, mpsc::Sender<Msg>>;

fn workers() -> &'static Workers {
    static W: OnceLock<Workers> = OnceLock::new();
    W.get_or_init(DashMap::new)
}

/// Marker for the `#[tynd::handler]` dispatch table — worker state
/// lives in the file-level `workers()` DashMap.
#[derive(Debug, Default)]
pub struct WorkersApi;

#[tynd::handler(api = "workers")]
impl WorkersApi {
    /// Spawn a new QuickJS worker that evaluates `script` (which
    /// must produce a function). Returns `{ id }` for use in `run` /
    /// `terminate`.
    pub fn spawn(script: String) -> Result<Value, String> {
        spawn_impl(script)
    }

    /// Invoke the worker's function with `input` (JSON-serialised
    /// before crossing the QuickJS boundary). Blocks until the
    /// worker returns or the runtime crashes.
    pub fn run(id: u64, input: Option<Value>) -> Result<Value, String> {
        run_impl(id, input.unwrap_or(Value::Null))
    }

    /// Stop the worker thread for `id`. No-op when the id is not
    /// known.
    pub fn terminate(id: u64) {
        if let Some((_, sender)) = workers().remove(&id) {
            let _ = sender.send(Msg::Stop);
        }
    }

    /// List every active worker id.
    pub fn list() -> Vec<Value> {
        workers()
            .iter()
            .map(|r| Value::Number((*r.key()).into()))
            .collect()
    }
}

fn spawn_impl(script: String) -> Result<Value, String> {

    let (tx, rx) = mpsc::channel::<Msg>();
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

    std::thread::spawn(move || {
        let rt = match Runtime::new() {
            Ok(r) => r,
            Err(e) => {
                let _ = ready_tx.send(Err(format!("runtime init: {e}")));
                return;
            },
        };
        let ctx = match Context::full(&rt) {
            Ok(c) => c,
            Err(e) => {
                let _ = ready_tx.send(Err(format!("context init: {e}")));
                return;
            },
        };

        let init = ctx.with(|ctx| -> Result<(), String> {
            let wrapped = format!(
                "globalThis.__tynd_worker_fn__ = ({script});\nif (typeof \
                 globalThis.__tynd_worker_fn__ !== 'function') throw new Error('worker script \
                 must evaluate to a function');"
            );
            ctx.eval::<(), _>(wrapped).map_err(|e| e.to_string())
        });

        if let Err(e) = init {
            let _ = ready_tx.send(Err(e));
            return;
        }
        let _ = ready_tx.send(Ok(()));

        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Stop => break,
                Msg::Run { input, reply } => {
                    let result = ctx.with(|ctx| -> Result<String, String> {
                        let call_src = "(function(i){var \
                                        r=globalThis.__tynd_worker_fn__(JSON.parse(i));return \
                                        JSON.stringify(r===undefined?null:r);})";
                        let f: Function<'_> = ctx.eval(call_src).map_err(|e| e.to_string())?;
                        let out: rquickjs::String<'_> =
                            f.call((input,)).map_err(|e| e.to_string())?;
                        out.to_string().map_err(|e| e.to_string())
                    });
                    let _ = reply.send(result);

                    while rt.is_job_pending() {
                        match rt.execute_pending_job() {
                            Ok(true) => {},
                            _ => break,
                        }
                    }
                },
            }
        }
    });

    ready_rx.recv().map_err(|e| {
        IpcError::new("workers", "spawn", format!("thread died: {e}")).into_string()
    })??;

    workers().insert(id, tx);
    Ok(json!({ "id": id }))
}

fn run_impl(id: u64, input: Value) -> Result<Value, String> {
    let input_json = serde_json::to_string(&input).map_err(|e| e.to_string())?;

    let sender = workers()
        .get(&id)
        .map(|r| r.value().clone())
        .ok_or_else(|| {
            IpcError::new("workers", "run", format!("worker {id} not found")).into_string()
        })?;

    let (reply_tx, reply_rx) = mpsc::channel();
    sender
        .send(Msg::Run {
            input: input_json,
            reply: reply_tx,
        })
        .map_err(|e| IpcError::new("workers", "run", format!("send: {e}")).into_string())?;

    let result_json = reply_rx
        .recv()
        .map_err(|e| IpcError::new("workers", "run", format!("worker died: {e}")).into_string())??;
    let parsed: Value = serde_json::from_str(&result_json).unwrap_or(Value::Null);
    Ok(parsed)
}


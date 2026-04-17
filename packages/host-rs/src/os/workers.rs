//! Lite-only: isolated QuickJS runtime per worker on its own OS thread.
//! Full uses `Bun.Worker` from TS instead. Input/output ship as JSON.

use dashmap::DashMap;
use rquickjs::{Context, Function, Runtime};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::OnceLock;

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

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "spawn" => spawn(args),
        "run" => run(args),
        "terminate" => terminate(args),
        "list" => list(),
        _ => Err(format!("workers.{method}: unknown method")),
    }
}

fn spawn(args: &Value) -> Result<Value, String> {
    let script = args
        .get("script")
        .and_then(Value::as_str)
        .ok_or_else(|| "workers.spawn: missing 'script'".to_string())?
        .to_string();

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
                "globalThis.__tynd_worker_fn__ = ({script});\nif (typeof globalThis.__tynd_worker_fn__ !== 'function') throw new Error('worker script must evaluate to a function');"
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
                        let call_src =
                            "(function(i){var r=globalThis.__tynd_worker_fn__(JSON.parse(i));return JSON.stringify(r===undefined?null:r);})";
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

    ready_rx
        .recv()
        .map_err(|e| format!("workers.spawn: thread died: {e}"))??;

    workers().insert(id, tx);
    Ok(json!({ "id": id }))
}

fn run(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "workers.run: missing 'id'".to_string())?;
    let input = args.get("input").cloned().unwrap_or(Value::Null);
    let input_json = serde_json::to_string(&input).map_err(|e| e.to_string())?;

    let sender = workers()
        .get(&id)
        .map(|r| r.value().clone())
        .ok_or_else(|| format!("workers.run: worker {id} not found"))?;

    let (reply_tx, reply_rx) = mpsc::channel();
    sender
        .send(Msg::Run {
            input: input_json,
            reply: reply_tx,
        })
        .map_err(|e| format!("workers.run: send: {e}"))?;

    let result_json = reply_rx
        .recv()
        .map_err(|e| format!("workers.run: worker died: {e}"))??;
    let parsed: Value = serde_json::from_str(&result_json).unwrap_or(Value::Null);
    Ok(parsed)
}

fn terminate(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "workers.terminate: missing 'id'".to_string())?;
    if let Some((_, sender)) = workers().remove(&id) {
        let _ = sender.send(Msg::Stop);
    }
    Ok(Value::Null)
}

#[allow(clippy::unnecessary_wraps)] // dispatch expects Result — uniform return shape
fn list() -> Result<Value, String> {
    let ids: Vec<Value> = workers()
        .iter()
        .map(|r| Value::Number((*r.key()).into()))
        .collect();
    Ok(Value::Array(ids))
}

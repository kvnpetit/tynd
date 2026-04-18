//! Native bridge for `fetch`.
//!
//! `__tynd_fetch_start__(id, url, optsJson)` spawns a thread that performs
//! the HTTP request via ureq, decodes the body into ~16 KiB chunks, and
//! delivers them back to the QuickJS context as "fetch:meta" /
//! "fetch:chunk" / "fetch:end" `JsMsg::PolyfillEvent` messages. The JS
//! polyfill installs `globalThis.__tynd_polyfill_event__` to receive them.
//!
//! Abort: per-request `AtomicBool` checked between reads.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rquickjs::{Ctx, Function, Object};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::quickjs::JsMsg;

fn registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static REG: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Deserialize)]
struct Opts {
    #[serde(default = "default_method")]
    method: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(rename = "bodyB64", default)]
    body_b64: String,
}

fn default_method() -> String {
    "GET".into()
}

const CHUNK: usize = 16 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 60_000;

pub(crate) fn register<'js>(
    ctx: &Ctx<'js>,
    g: &Object<'js>,
    js_tx: &mpsc::Sender<JsMsg>,
) -> rquickjs::Result<()> {
    {
        let tx = js_tx.clone();
        g.set(
            "__tynd_fetch_start__",
            Function::new(
                ctx.clone(),
                move |id: String, url: String, opts_json: String| {
                    let cancel = Arc::new(AtomicBool::new(false));
                    registry()
                        .lock()
                        .unwrap()
                        .insert(id.clone(), cancel.clone());
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        run_fetch(&id, &url, &opts_json, &cancel, &tx);
                    });
                },
            )?,
        )?;
    }

    g.set(
        "__tynd_fetch_abort__",
        Function::new(ctx.clone(), |id: String| {
            if let Some(flag) = registry().lock().unwrap().get(&id) {
                flag.store(true, Ordering::SeqCst);
            }
        })?,
    )?;

    Ok(())
}

fn run_fetch(
    id: &str,
    url: &str,
    opts_json: &str,
    cancel: &Arc<AtomicBool>,
    js_tx: &mpsc::Sender<JsMsg>,
) {
    let opts: Opts = match serde_json::from_str(opts_json) {
        Ok(o) => o,
        Err(e) => return end_error(id, format!("invalid fetch opts: {e}"), js_tx),
    };

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(DEFAULT_TIMEOUT_MS))
        .build();

    let mut req = agent.request(&opts.method, url);
    for (k, v) in &opts.headers {
        req = req.set(k, v);
    }

    let res = if opts.body_b64.is_empty() {
        req.call()
    } else {
        let body_bytes = match STANDARD.decode(opts.body_b64.as_bytes()) {
            Ok(b) => b,
            Err(e) => return end_error(id, format!("invalid body base64: {e}"), js_tx),
        };
        req.send_bytes(&body_bytes)
    };

    let response = match res {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            // Error-status responses are still iterable — forward like a normal response.
            deliver_response(id, code, r, cancel, js_tx);
            return;
        },
        Err(e) => return end_error(id, format!("network: {e}"), js_tx),
    };

    let status = response.status();
    deliver_response(id, status, response, cancel, js_tx);
}

fn deliver_response(
    id: &str,
    status: u16,
    response: ureq::Response,
    cancel: &Arc<AtomicBool>,
    js_tx: &mpsc::Sender<JsMsg>,
) {
    let status_text = response.status_text().to_string();
    let mut headers = serde_json::Map::new();
    for name in response.headers_names() {
        if let Some(v) = response.header(&name) {
            headers.insert(name.to_lowercase(), Value::String(v.to_string()));
        }
    }

    send(
        js_tx,
        "fetch:meta",
        json!({
            "id": id,
            "status": status,
            "statusText": status_text,
            "headers": Value::Object(headers),
        }),
    );

    let mut reader = response.into_reader();
    let mut buf = [0u8; CHUNK];
    loop {
        if cancel.load(Ordering::SeqCst) {
            end_error(id, "aborted", js_tx);
            return;
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => send(
                js_tx,
                "fetch:chunk",
                json!({
                    "id": id,
                    "chunk": STANDARD.encode(&buf[..n]),
                }),
            ),
            Err(e) => return end_error(id, format!("read: {e}"), js_tx),
        }
    }

    send(js_tx, "fetch:end", json!({ "id": id }));
    registry().lock().unwrap().remove(id);
}

fn end_error(id: &str, msg: impl AsRef<str>, js_tx: &mpsc::Sender<JsMsg>) {
    send(
        js_tx,
        "fetch:end",
        json!({ "id": id, "error": msg.as_ref() }),
    );
    registry().lock().unwrap().remove(id);
}

fn send(js_tx: &mpsc::Sender<JsMsg>, name: &'static str, data: Value) {
    let _ = js_tx.send(JsMsg::PolyfillEvent {
        name: name.to_string(),
        data,
    });
}

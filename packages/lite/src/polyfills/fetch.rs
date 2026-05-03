//! Native bridge for `fetch`.
//!
//! `__tynd_fetch_start__(id, url, optsJson)` spawns a thread that performs
//! the HTTP request via ureq, decodes the body into ~16 KiB chunks, and
//! delivers them back to the QuickJS context as "fetch:meta" /
//! "fetch:chunk" / "fetch:end" `JsMsg::PolyfillEvent` messages. The JS
//! polyfill installs `globalThis.__tynd_polyfill_event__` to receive them.
//!
//! Abort: per-request `AtomicBool` checked between reads.
//!
//! V1 (B27): per-fetch command channel switched from `std::sync::mpsc`
//! to `crossbeam_channel` to match the shared `Sessions<C>` helper —
//! the HTTP-side `JsMsg` channel back to QuickJS stays on `mpsc` (the
//! rquickjs runtime expects that exact type).
//!
//! M1 (ureq 3 migration):
//!   * `AgentBuilder::new()` → `Agent::config_builder().build().new_agent()`
//!   * `agent.request(method, url)` is gone; we hand-build an
//!     `http::Request` and run it through `agent.run(...)` so the
//!     verb (chosen at runtime by the JS caller) stays dynamic.
//!   * Status-based error handling collapsed: ureq 3 has no
//!     `Error::Status(code, response)` variant — instead we set
//!     `http_status_as_error(false)` on the agent so 4xx/5xx flow
//!     through the normal `Ok(response)` path, then forward to the
//!     JS layer like before. The semantics match the 2.x code's
//!     "error-status responses are still iterable" branch.

use std::{
    collections::HashMap,
    io::Read,
    sync::{mpsc, OnceLock},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD, Engine};
use rquickjs::{Ctx, Function, Object};
use serde::Deserialize;
use serde_json::{json, Value};

use super::_session::Sessions;
use crate::quickjs::JsMsg;

/// Per-fetch worker command. The previous implementation used an
/// `Arc<AtomicBool>` flag flipped from the abort handler; routing
/// through the same `Sessions<Cmd>` helper as the WebSocket bridge
/// folds the two cancel paths into one — and gives us a place to add
/// future commands (pause, throttle, …) without redoing the
/// registration plumbing.
pub(crate) enum Cmd {
    Abort,
}

fn sessions() -> &'static Sessions<Cmd> {
    static REG: OnceLock<Sessions<Cmd>> = OnceLock::new();
    REG.get_or_init(Sessions::new)
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
                    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<Cmd>();
                    sessions().insert(id.clone(), cmd_tx);
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        run_fetch(&id, &url, &opts_json, &cmd_rx, &tx);
                        sessions().remove(&id);
                    });
                },
            )?,
        )?;
    }

    g.set(
        "__tynd_fetch_abort__",
        Function::new(ctx.clone(), |id: String| {
            sessions().send(&id, Cmd::Abort);
        })?,
    )?;

    Ok(())
}

fn run_fetch(
    id: &str,
    url: &str,
    opts_json: &str,
    cmd_rx: &crossbeam_channel::Receiver<Cmd>,
    js_tx: &mpsc::Sender<JsMsg>,
) {
    let opts: Opts = match serde_json::from_str(opts_json) {
        Ok(o) => o,
        Err(e) => return end_error(id, format!("invalid fetch opts: {e}"), js_tx),
    };

    // ureq 3: `http_status_as_error(false)` keeps 4xx/5xx out of the
    // `Err` arm so the polyfill can forward an error status response
    // body the same way it forwards a 2xx — `fetch()`'s contract is
    // that non-2xx responses are still iterable.
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_millis(DEFAULT_TIMEOUT_MS)))
        .build()
        .new_agent();

    // Build the request through the `http` crate so the verb stays
    // dynamic — ureq 3's typed `agent.get/post/...` shadow it at
    // compile time.
    let mut http_builder = ureq::http::Request::builder()
        .method(opts.method.as_str())
        .uri(url);
    for (k, v) in &opts.headers {
        http_builder = http_builder.header(k, v);
    }

    let body_bytes = if opts.body_b64.is_empty() {
        Vec::new()
    } else {
        match STANDARD.decode(opts.body_b64.as_bytes()) {
            Ok(b) => b,
            Err(e) => return end_error(id, format!("invalid body base64: {e}"), js_tx),
        }
    };

    let req_result = if body_bytes.is_empty() {
        http_builder.body(ureq::SendBody::none())
    } else {
        http_builder.body(ureq::SendBody::from_owned_reader(std::io::Cursor::new(
            body_bytes,
        )))
    };
    let req = match req_result {
        Ok(r) => r,
        Err(e) => return end_error(id, format!("invalid request: {e}"), js_tx),
    };

    let response = match agent.run(req) {
        Ok(r) => r,
        // ureq 3 collapses the 2.x `Status(code, resp)` variant into
        // the normal Ok path (because we set http_status_as_error =
        // false above). Anything that lands here is a real network /
        // protocol error.
        Err(e) => return end_error(id, format!("network: {e}"), js_tx),
    };

    let status = response.status().as_u16();
    deliver_response(id, status, response, cmd_rx, js_tx);
}

fn deliver_response(
    id: &str,
    status: u16,
    mut response: ureq::http::Response<ureq::Body>,
    cmd_rx: &crossbeam_channel::Receiver<Cmd>,
    js_tx: &mpsc::Sender<JsMsg>,
) {
    let status_text = response
        .status()
        .canonical_reason()
        .unwrap_or("")
        .to_string();
    let mut headers = serde_json::Map::new();
    for (name, value) in response.headers() {
        if let Ok(v) = value.to_str() {
            headers.insert(name.as_str().to_lowercase(), Value::String(v.to_string()));
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

    let mut reader = response.body_mut().as_reader();
    let mut buf = [0u8; CHUNK];
    loop {
        // Abort is signalled via `Sessions::send(id, Cmd::Abort)` from
        // the JS-side abort callback. The receiver is non-blocking so
        // the read loop only checks it between chunks — keeping the
        // hot path identical to the previous `AtomicBool` poll.
        if let Ok(Cmd::Abort) = cmd_rx.try_recv() {
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
    // `sessions().remove` is called by the spawn closure when
    // `run_fetch` returns; nothing to do here.
}

fn end_error(id: &str, msg: impl AsRef<str>, js_tx: &mpsc::Sender<JsMsg>) {
    send(
        js_tx,
        "fetch:end",
        json!({ "id": id, "error": msg.as_ref() }),
    );
}

fn send(js_tx: &mpsc::Sender<JsMsg>, name: &'static str, data: Value) {
    let _ = js_tx.send(JsMsg::PolyfillEvent {
        name: name.to_string(),
        data,
    });
}

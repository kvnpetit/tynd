// rquickjs types (Function, Object, Value, String) are parameterized by ctx lifetime;
// elided everywhere here to keep call sites readable — context is obvious.
#![allow(elided_lifetimes_in_paths)]
use rquickjs::{Context, Function, Object, Runtime};
use serde_json::Value;
use std::collections::{BinaryHeap, HashSet};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tynd_host::{
    runtime::{BackendBridge, BackendCall, BackendConfig, BackendEvent, MenuItemDef, TrayConfig},
    tynd_log,
};

enum JsMsg {
    Call(BackendCall),
    TimerFire(u32),
}

enum TimerCmd {
    Set { id: u32, ms: u32, once: bool },
    Clear(u32),
}

/// Dev-mode handle to hot-reload the QuickJS backend without tearing down the
/// host process or the WebView. Mirrors the full-mode `ReloadHandle`.
#[derive(Clone)]
pub(crate) struct ReloadHandle {
    bundle_path: String,
    /// The forwarder thread writes to this slot. On reload we swap its contents
    /// so the next message goes to the new JS thread.
    js_tx_slot: Arc<Mutex<mpsc::Sender<JsMsg>>>,
    event_tx: mpsc::Sender<BackendEvent>,
}

impl ReloadHandle {
    pub(crate) fn reload(&self) {
        let bundle_code = match std::fs::read_to_string(&self.bundle_path) {
            Ok(s) => s,
            Err(e) => {
                let _ = self.event_tx.send(BackendEvent::Error {
                    message: format!("Cannot read bundle '{}': {e}", self.bundle_path),
                });
                return;
            },
        };

        // Create a new js channel pair. Spawning the new thread with its own
        // receiver; we install the sender in the slot so the forwarder routes
        // future BackendCalls to the new thread. Dropping the old sender (by
        // overwriting it) causes the old JS thread's recv() to return Err and
        // exit cleanly.
        let (new_js_tx, new_js_rx) = mpsc::channel::<JsMsg>();
        let (cfg_tx, cfg_rx) = mpsc::sync_channel::<BackendConfig>(1);
        let js_tx_for_thread = new_js_tx.clone();
        let event_tx = self.event_tx.clone();

        std::thread::spawn(move || {
            js_thread_main(bundle_code, js_tx_for_thread, new_js_rx, cfg_tx, event_tx);
        });

        // Block until the new bundle signals it's ready — if it fails, surface
        // the error in the overlay and leave the old JS thread running? No:
        // we've already dropped its sender. Accept the dead state and let the
        // user fix the bundle.
        match cfg_rx.recv() {
            Ok(_) => {
                // Swap the sender last so no message racing to the old thread.
                *self.js_tx_slot.lock().unwrap() = new_js_tx;
                let _ = self.event_tx.send(BackendEvent::Reload);
            },
            Err(_) => {
                let _ = self.event_tx.send(BackendEvent::Error {
                    message: "Backend reload failed: bundle did not call app.start()".into(),
                });
            },
        }
    }
}

/// Start the QuickJS backend.
///
/// - Loads `bundle_code` in an embedded QuickJS runtime
/// - Blocks until the bundle sets `globalThis.__tynd_config__`
/// - Returns a `BackendBridge` plus a `ReloadHandle` for dev-mode hot reload
pub(crate) fn start(
    bundle_path: &str,
    frontend_dir: Option<String>,
    dev_url: Option<String>,
    icon_path: Option<String>,
) -> (BackendBridge, ReloadHandle) {
    let bundle_code = std::fs::read_to_string(bundle_path).unwrap_or_else(|e| {
        tynd_log!("Cannot read bundle '{bundle_path}': {e}");
        tynd_log!("Build first: tynd build");
        std::process::exit(1);
    });

    let (js_tx, js_rx) = mpsc::channel::<JsMsg>();
    let (cfg_tx, cfg_rx) = mpsc::sync_channel::<BackendConfig>(1);
    let (event_tx, event_rx) = mpsc::channel::<BackendEvent>();
    let (call_tx, call_rx) = mpsc::channel::<BackendCall>();

    // The JS sender lives behind a mutex so ReloadHandle can swap it atomically.
    let js_tx_slot: Arc<Mutex<mpsc::Sender<JsMsg>>> = Arc::new(Mutex::new(js_tx.clone()));

    // Forwarder: BackendCall -> current JS thread (via the slot)
    {
        let slot = js_tx_slot.clone();
        std::thread::spawn(move || {
            while let Ok(call) = call_rx.recv() {
                let tx = slot.lock().unwrap().clone();
                let _ = tx.send(JsMsg::Call(call));
            }
        });
    }

    let js_tx_for_thread = js_tx.clone();
    let event_tx_for_thread = event_tx.clone();
    std::thread::spawn(move || {
        js_thread_main(
            bundle_code,
            js_tx_for_thread,
            js_rx,
            cfg_tx,
            event_tx_for_thread,
        );
    });

    let mut config = cfg_rx.recv().unwrap_or_else(|_| {
        tynd_log!("JS thread died before sending config — check your backend calls app.start()");
        std::process::exit(1);
    });

    if frontend_dir.is_some() {
        config.frontend_dir = frontend_dir;
    }
    if dev_url.is_some() {
        config.dev_url = dev_url;
    }
    if icon_path.is_some() {
        config.icon_path = icon_path;
    }

    let bridge = BackendBridge {
        config,
        call_tx,
        event_rx,
    };
    let reload = ReloadHandle {
        bundle_path: bundle_path.to_string(),
        js_tx_slot,
        event_tx,
    };
    (bridge, reload)
}

// Thread entry fn — all channels must be owned (moved into thread scope).
#[allow(clippy::needless_pass_by_value)]
fn js_thread_main(
    bundle_code: String,
    js_tx: mpsc::Sender<JsMsg>,
    js_rx: mpsc::Receiver<JsMsg>,
    config_tx: mpsc::SyncSender<BackendConfig>,
    event_tx: mpsc::Sender<BackendEvent>,
) {
    let rt = Runtime::new().expect("QuickJS Runtime::new failed");
    let ctx = Context::full(&rt).expect("QuickJS Context::full failed");

    // Timer subsystem runs on a single thread with a priority queue, not one
    // OS thread per setTimeout/setInterval call.
    let (timer_tx, timer_rx) = mpsc::sync_channel::<TimerCmd>(64);
    start_timer_thread(timer_rx, js_tx.clone());

    {
        let event_tx_emit = event_tx.clone();
        let timer_tx_set = timer_tx.clone();
        let timer_tx_clr = timer_tx.clone();

        let result = ctx.with(|ctx| -> rquickjs::Result<()> {
            let g = ctx.globals();

            g.set("__tynd_lite__", true)?;

            // console -> eprintln
            g.set(
                "__tynd_log__",
                Function::new(ctx.clone(), |level: String, msg: String| {
                    eprintln!("[{level}] {msg}");
                })?,
            )?;

            // __tynd_emit__(name, payloadJson) — called by app.start() emit
            {
                let tx = event_tx_emit.clone();
                g.set(
                    "__tynd_emit__",
                    Function::new(ctx.clone(), move |name: String, payload_json: String| {
                        let payload = serde_json::from_str(&payload_json).unwrap_or(Value::Null);
                        let _ = tx.send(BackendEvent::Emit { name, payload });
                    })?,
                )?;
            }

            // __tynd_set_interval__(id, ms, once) — called by JS setTimeout/setInterval
            {
                let tx = timer_tx_set.clone();
                g.set(
                    "__tynd_set_interval__",
                    Function::new(ctx.clone(), move |id: u32, ms: u32, once: bool| {
                        let _ = tx.send(TimerCmd::Set { id, ms, once });
                    })?,
                )?;
            }

            // __tynd_clear_interval__(id) — called by JS clearTimeout/clearInterval
            {
                let tx = timer_tx_clr.clone();
                g.set(
                    "__tynd_clear_interval__",
                    Function::new(ctx.clone(), move |id: u32| {
                        let _ = tx.send(TimerCmd::Clear(id));
                    })?,
                )?;
            }

            ctx.eval::<(), _>(JS_GLOBALS)?;
            ctx.eval::<(), _>(bundle_code.as_bytes())?;
            Ok(())
        });

        if let Err(e) = result {
            let msg = format!("Bundle evaluation failed: {e}");
            tynd_log!("{msg}");
            let _ = event_tx.send(BackendEvent::Error { message: msg });
            // Don't exit — let the outer (reload) code handle recovery.
            // config_tx is dropped implicitly; the initial `start()` receiver
            // will see the close and exit; ReloadHandle.reload() surfaces the
            // error via BackendEvent::Error.
            return;
        }
    }

    // Read config set by app.start()
    let config: BackendConfig = ctx.with(|ctx| {
        let json: Option<String> = ctx.globals().get("__tynd_config__").unwrap_or(None);
        json.and_then(|s| parse_config(&s)).unwrap_or_default()
    });
    let _ = config_tx.send(config);

    while let Ok(msg) = js_rx.recv() {
        match msg {
            JsMsg::Call(call) => {
                let (id, fn_name, args) = match call {
                    BackendCall::Typed { id, fn_name, args } => (id, fn_name, args),
                    // Raw JSON from the IPC fast-path — parse it here
                    BackendCall::Raw(json) => {
                        let Ok(v) = serde_json::from_str::<Value>(&json) else {
                            continue;
                        };
                        let id = v["id"].as_str().unwrap_or("").to_string();
                        let fn_name = v["fn"].as_str().unwrap_or("").to_string();
                        let args = v["args"].as_array().cloned().unwrap_or_default();
                        (id, fn_name, args)
                    },
                };

                let result = ctx.with(|ctx| {
                    if fn_name.starts_with("__tynd_") {
                        call_global(&ctx, &fn_name)
                    } else {
                        call_module_fn(&ctx, &fn_name, &args)
                    }
                });

                let evt = match result {
                    Ok(value) => BackendEvent::Return {
                        id,
                        ok: true,
                        value,
                    },
                    Err(error) => BackendEvent::Return {
                        id,
                        ok: false,
                        value: Value::String(error),
                    },
                };
                let _ = event_tx.send(evt);
            },

            JsMsg::TimerFire(id) => {
                let _ = ctx.with(|ctx| -> rquickjs::Result<()> {
                    let fire: Function = ctx.globals().get("__tynd_fire_timer__")?;
                    fire.call::<_, ()>((id,))
                });
            },
        }

        // Drive Promise microtasks after each message
        while rt.is_job_pending() {
            match rt.execute_pending_job() {
                Ok(true) => {},
                Ok(false) => break,
                Err(e) => {
                    tynd_log!("Job error: {e}");
                },
            }
        }
    }
}

// Single timer thread with a min-heap ordered by fire_at. `recv_timeout` sleeps
// until the next deadline without busy-waiting, and avoids one OS thread per
// setTimeout/setInterval call.
fn start_timer_thread(cmd_rx: mpsc::Receiver<TimerCmd>, js_tx: mpsc::Sender<JsMsg>) {
    std::thread::spawn(move || {
        #[derive(Eq, PartialEq)]
        struct Entry {
            fire_at: Instant,
            id: u32,
            ms: u32,
            once: bool,
        }

        // Ord reversed so BinaryHeap (max-heap) behaves as a min-heap on fire_at
        impl Ord for Entry {
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                o.fire_at.cmp(&self.fire_at)
            }
        }
        impl PartialOrd for Entry {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }

        let mut heap: BinaryHeap<Entry> = BinaryHeap::with_capacity(32);
        let mut cancelled: HashSet<u32> = HashSet::new();
        // Track IDs currently in the heap to avoid unbounded growth of `cancelled`
        let mut active: HashSet<u32> = HashSet::new();

        loop {
            let now = Instant::now();

            // Fire all timers whose deadline has passed
            while heap.peek().is_some_and(|e| e.fire_at <= now) {
                let e = heap.pop().unwrap();
                if cancelled.contains(&e.id) {
                    cancelled.remove(&e.id);
                    active.remove(&e.id);
                    continue;
                }
                if js_tx.send(JsMsg::TimerFire(e.id)).is_err() {
                    return;
                }
                if e.once {
                    active.remove(&e.id); // timeout done, no longer active
                } else {
                    // Re-schedule interval using fire_at + dur to prevent drift
                    let dur = Duration::from_millis(e.ms as u64);
                    heap.push(Entry {
                        fire_at: e.fire_at + dur,
                        id: e.id,
                        ms: e.ms,
                        once: false,
                    });
                    // active stays — interval is still scheduled
                }
            }

            // Sleep until the next deadline or a new command arrives
            let timeout = heap.peek().map_or(Duration::from_secs(3600), |e| {
                e.fire_at.saturating_duration_since(Instant::now())
            });

            match cmd_rx.recv_timeout(timeout) {
                Ok(TimerCmd::Set { id, ms, once }) => {
                    let fire_at = Instant::now() + Duration::from_millis(ms as u64);
                    if active.contains(&id) {
                        cancelled.insert(id);
                    } else {
                        active.insert(id);
                    }
                    heap.push(Entry {
                        fire_at,
                        id,
                        ms,
                        once,
                    });
                },
                Ok(TimerCmd::Clear(id)) => {
                    if active.contains(&id) {
                        cancelled.insert(id);
                    }
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {},
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

fn call_global(ctx: &rquickjs::Ctx, fn_name: &str) -> Result<Value, String> {
    let g = ctx.globals();
    let f: Function = match g.get(fn_name) {
        Ok(f) => f,
        Err(_) => return Ok(Value::Null), // lifecycle handler not set
    };
    match f.call::<_, ()>(()) {
        Ok(()) => Ok(Value::Null),
        Err(e) => Err(format!("{fn_name}: {e}")),
    }
}

fn call_module_fn(ctx: &rquickjs::Ctx, fn_name: &str, args: &[Value]) -> Result<Value, String> {
    let g = ctx.globals();
    let module: Option<Object> = g.get("__tynd_mod__").ok();
    let Some(module) = module else {
        return Err(format!("Module not loaded — cannot call '{fn_name}'"));
    };

    // Existence check — dispatch happens via string-based eval below so the
    // function can be JSON-spread-applied without per-arg type conversion.
    if module.get::<_, Function>(fn_name).is_err() {
        return Err(format!("Unknown function '{fn_name}'"));
    }

    // Convert serde_json::Value[] to rquickjs arguments via JSON round-trip
    let args_json = serde_json::to_string(args).unwrap_or_else(|_| "[]".into());
    let args_str: rquickjs::String = ctx
        .eval::<rquickjs::Value, _>(format!("JSON.stringify({args_json})"))
        .map_err(|e| e.to_string())?
        .try_into_string()
        .map_err(|_| "args roundtrip failed".to_string())?;

    // Parse the JSON inside the ctx, then spread-call
    let do_call: Function = ctx
        .eval(format!(
            "(function(m, a) {{ return JSON.stringify(m.{fn_name}.apply(m, JSON.parse(a))); }})"
        ))
        .map_err(|e| e.to_string())?;

    let result_str: rquickjs::String = do_call
        .call((module, args_str))
        .map_err(|e| e.to_string())?;

    let s = result_str.to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

fn parse_config(json: &str) -> Option<BackendConfig> {
    let v: Value = serde_json::from_str(json).ok()?;
    let window = v
        .get("window")
        .and_then(|w| serde_json::from_value(w.clone()).ok())
        .unwrap_or_default();
    let menu: Vec<MenuItemDef> = v
        .get("menu")
        .and_then(|m| serde_json::from_value(m.clone()).ok())
        .unwrap_or_default();
    let tray: Option<TrayConfig> = v
        .get("tray")
        .and_then(|t| serde_json::from_value(t.clone()).ok());
    let dev_url = v.get("devUrl").and_then(|u| u.as_str().map(String::from));
    let frontend_dir = v
        .get("frontendDir")
        .and_then(|d| d.as_str().map(String::from));
    Some(BackendConfig {
        window,
        dev_url,
        frontend_dir,
        icon_path: None,
        menu,
        tray,
    })
}

// JS globals installed before the user bundle runs.
const JS_GLOBALS: &str = r"
globalThis.globalThis = globalThis;
(function () {
  function fmt(a) {
    try { return typeof a === 'string' ? a : JSON.stringify(a); }
    catch (_) { return String(a); }
  }
  function join(args) {
    var out = '';
    for (var i = 0; i < args.length; i++) { if (i) out += ' '; out += fmt(args[i]); }
    return out;
  }
  globalThis.console = {
    log:   function () { __tynd_log__('log',   join(arguments)); },
    info:  function () { __tynd_log__('info',  join(arguments)); },
    warn:  function () { __tynd_log__('warn',  join(arguments)); },
    error: function () { __tynd_log__('error', join(arguments)); },
    debug: function () { __tynd_log__('debug', join(arguments)); },
  };
})();

(function () {
  var nextId = 1;
  var handlers = {};
  globalThis.setTimeout = function (fn, ms) {
    var id = nextId++; handlers[id] = { fn: fn, once: true };
    __tynd_set_interval__(id, ms || 0, true);
    return id;
  };
  globalThis.setInterval = function (fn, ms) {
    var id = nextId++; handlers[id] = { fn: fn, once: false };
    __tynd_set_interval__(id, ms || 0, false);
    return id;
  };
  globalThis.clearTimeout = function (id) {
    delete handlers[id]; __tynd_clear_interval__(id);
  };
  globalThis.clearInterval = globalThis.clearTimeout;
  // Fired by Rust timer thread
  globalThis.__tynd_fire_timer__ = function (id) {
    var h = handlers[id]; if (!h) return;
    if (h.once) delete handlers[id];
    try { h.fn(); } catch (e) { console.error('timer ' + id + ': ' + e); }
  };
})();

// Store user module exports on globalThis for RPC dispatch
globalThis.__tynd_mod__ = globalThis;
";

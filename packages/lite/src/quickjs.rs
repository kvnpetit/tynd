use rquickjs::{Context, Function, Object, Runtime};
use serde_json::Value;
use std::collections::{BinaryHeap, HashSet};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vorn_host::{
    runtime::{BackendBridge, BackendCall, BackendConfig, BackendEvent, MenuItemDef, TrayConfig},
    vorn_log,
};

enum JsMsg {
    Call(BackendCall),
    TimerFire(u32),
}

enum TimerCmd {
    Set { id: u32, ms: u32, once: bool },
    Clear(u32),
}

/// Start the QuickJS backend.
///
/// - Loads `bundle_code` in an embedded QuickJS runtime
/// - Blocks until the bundle sets `globalThis.__vorn_config__`
/// - Returns a `BackendBridge` ready for `run_app()`
pub fn start(
    bundle_path: &str,
    frontend_dir: Option<String>,
    dev_url: Option<String>,
    icon_path: Option<String>,
) -> BackendBridge {
    let bundle_code = std::fs::read_to_string(bundle_path).unwrap_or_else(|e| {
        vorn_log!("Cannot read bundle '{bundle_path}': {e}");
        vorn_log!("Build first: vorn build");
        std::process::exit(1);
    });

    // call_tx is sent from the WebView IPC handler — unbounded to never block the WebView thread.
    // js_tx relays calls to the QuickJS thread — also unbounded for same reason.
    // event_tx is QuickJS → main thread; unbounded so the JS thread never blocks.
    let (js_tx, js_rx) = mpsc::channel::<JsMsg>();
    let (cfg_tx, cfg_rx) = mpsc::sync_channel::<BackendConfig>(1);
    let (event_tx, event_rx) = mpsc::channel::<BackendEvent>();
    let (call_tx, call_rx) = mpsc::channel::<BackendCall>();

    // Forward BackendCalls → JsMsg::Call
    {
        let js_tx = js_tx.clone();
        std::thread::spawn(move || {
            while let Ok(call) = call_rx.recv() {
                let _ = js_tx.send(JsMsg::Call(call));
            }
        });
    }

    let js_tx_for_thread = js_tx.clone();
    std::thread::spawn(move || {
        js_thread_main(bundle_code, js_tx_for_thread, js_rx, cfg_tx, event_tx);
    });

    // Block until bundle sets __vorn_config__
    let mut config = cfg_rx.recv().unwrap_or_else(|_| {
        vorn_log!("JS thread died before sending config — check your backend calls app.start()");
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

    BackendBridge {
        config,
        call_tx,
        event_rx,
    }
}

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

            g.set("__vorn_lite__", true)?;

            // console → eprintln
            g.set(
                "__vorn_log__",
                Function::new(ctx.clone(), |level: String, msg: String| {
                    eprintln!("[{level}] {msg}");
                })?,
            )?;

            // __vorn_emit__(name, payloadJson) — called by app.start() emit
            {
                let tx = event_tx_emit.clone();
                g.set(
                    "__vorn_emit__",
                    Function::new(ctx.clone(), move |name: String, payload_json: String| {
                        let payload = serde_json::from_str(&payload_json).unwrap_or(Value::Null);
                        let _ = tx.send(BackendEvent::Emit { name, payload });
                    })?,
                )?;
            }

            // __vorn_set_interval__(id, ms, once) — called by JS setTimeout/setInterval
            {
                let tx = timer_tx_set.clone();
                g.set(
                    "__vorn_set_interval__",
                    Function::new(ctx.clone(), move |id: u32, ms: u32, once: bool| {
                        let _ = tx.send(TimerCmd::Set { id, ms, once });
                    })?,
                )?;
            }

            // __vorn_clear_interval__(id) — called by JS clearTimeout/clearInterval
            {
                let tx = timer_tx_clr.clone();
                g.set(
                    "__vorn_clear_interval__",
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
            vorn_log!("Bundle evaluation failed: {e}");
            std::process::exit(1);
        }
    }

    // Read config set by app.start()
    let config: BackendConfig = ctx.with(|ctx| {
        let json: Option<String> = ctx.globals().get("__vorn_config__").unwrap_or(None);
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
                    if fn_name.starts_with("__vorn_") {
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
                    let fire: Function = ctx.globals().get("__vorn_fire_timer__")?;
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
                    vorn_log!("Job error: {e}");
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
                if !e.once {
                    // Re-schedule interval using fire_at + dur to prevent drift
                    let dur = Duration::from_millis(e.ms as u64);
                    heap.push(Entry {
                        fire_at: e.fire_at + dur,
                        id: e.id,
                        ms: e.ms,
                        once: false,
                    });
                    // active stays — interval is still scheduled
                } else {
                    active.remove(&e.id); // timeout done, no longer active
                }
            }

            // Sleep until the next deadline or a new command arrives
            let timeout = heap
                .peek()
                .map(|e| e.fire_at.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::from_secs(3600));

            match cmd_rx.recv_timeout(timeout) {
                Ok(TimerCmd::Set { id, ms, once }) => {
                    let dur = Duration::from_millis(ms as u64);
                    heap.push(Entry {
                        fire_at: Instant::now() + dur,
                        id,
                        ms,
                        once,
                    });
                    active.insert(id);
                },
                Ok(TimerCmd::Clear(id)) => {
                    // Only cancel if the timer is still active — prevents unbounded set growth
                    if active.contains(&id) {
                        cancelled.insert(id);
                    }
                },
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}, // timer deadline reached
            }
        }
    });
}

/// Call an exported function via the pre-defined `__vorn_do_call__` helper.
///
/// Performance: 1 global lookup + 2 rquickjs::String allocs per call.
/// Eliminates:
///   - `ctx.eval(...)` per call (was recompiling a JS function every time)
///   - 5 extra property lookups (__vorn_mod__, JSON, stringify, etc.)
fn call_module_fn(ctx: &rquickjs::Ctx<'_>, fn_name: &str, args: &[Value]) -> Result<Value, String> {
    let do_call: Function = ctx
        .globals()
        .get("__vorn_do_call__")
        .map_err(|e| format!("__vorn_do_call__ missing: {e}"))?;

    let fn_js = rquickjs::String::from_str(ctx.clone(), fn_name).map_err(|e| e.to_string())?;
    let args_json = if args.is_empty() {
        "[]".to_string()
    } else {
        serde_json::to_string(args).unwrap_or_else(|_| "[]".into())
    };
    let args_js = rquickjs::String::from_str(ctx.clone(), &args_json).map_err(|e| e.to_string())?;

    let result_str: rquickjs::String = do_call
        .call((fn_js, args_js))
        .map_err(|e| format!("'{fn_name}': {e}"))?;

    let s = result_str.to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&s).map_err(|e| format!("result parse: {e}"))
}

/// Call a no-arg lifecycle function on `globalThis` (on_ready, on_close).
fn call_global(ctx: &rquickjs::Ctx<'_>, fn_name: &str) -> Result<Value, String> {
    let fn_val: rquickjs::Value = ctx
        .globals()
        .get(fn_name)
        .map_err(|e| format!("global '{fn_name}' not found: {e}"))?;

    if fn_val.is_undefined() {
        return Ok(Value::Null);
    }
    if !fn_val.is_function() {
        return Ok(Value::Null);
    }

    let func: Function = fn_val.into_function().unwrap();
    let raw = func
        .call::<_, rquickjs::Value>(())
        .map_err(|e| e.to_string())?;

    if raw.is_undefined() || raw.is_null() {
        return Ok(Value::Null);
    }
    let json_obj: Object = ctx.globals().get("JSON").map_err(|e| e.to_string())?;
    let stringify: Function = json_obj.get("stringify").map_err(|e| e.to_string())?;
    let result: Option<rquickjs::String> = stringify.call((raw,)).map_err(|e| e.to_string())?;
    let s = result
        .and_then(|s| s.to_string().ok())
        .unwrap_or_else(|| "null".into());
    serde_json::from_str(&s).map_err(|e| format!("result JSON parse: {e}"))
}

fn parse_config(s: &str) -> Option<BackendConfig> {
    #[derive(serde::Deserialize)]
    struct W {
        window: Option<vorn_host::window::WindowConfig>,
        #[serde(default)]
        menu: Vec<MenuItemDef>,
        tray: Option<TrayConfig>,
        #[serde(rename = "devUrl")]
        dev_url: Option<String>,
        #[serde(rename = "frontendDir")]
        frontend_dir: Option<String>,
        #[serde(rename = "iconPath")]
        icon_path: Option<String>,
    }
    let w: W = serde_json::from_str(s).ok()?;
    Some(BackendConfig {
        window: w.window.unwrap_or_default(),
        menu: w.menu,
        tray: w.tray,
        dev_url: w.dev_url,
        frontend_dir: w.frontend_dir,
        icon_path: w.icon_path,
    })
}

const JS_GLOBALS: &str = r#"
(function () {
  "use strict";

  function __fmt__(a) {
    if (a === null) return "null";
    if (a === undefined) return "undefined";
    if (typeof a === "object" || (typeof a === "function" && a !== null)) {
      try { return JSON.stringify(a); } catch(e) { return String(a); }
    }
    return String(a);
  }
  globalThis.console = {
    log:   function() { __vorn_log__("LOG",   Array.prototype.slice.call(arguments).map(__fmt__).join(" ")); },
    info:  function() { __vorn_log__("INFO",  Array.prototype.slice.call(arguments).map(__fmt__).join(" ")); },
    warn:  function() { __vorn_log__("WARN",  Array.prototype.slice.call(arguments).map(__fmt__).join(" ")); },
    error: function() { __vorn_log__("ERROR", Array.prototype.slice.call(arguments).map(__fmt__).join(" ")); },
    debug: function() { __vorn_log__("DEBUG", Array.prototype.slice.call(arguments).map(__fmt__).join(" ")); },
    trace: function() { __vorn_log__("TRACE", Array.prototype.slice.call(arguments).map(__fmt__).join(" ")); },
  };

  // Safe process stub for Bun-targeting bundles running in QuickJS
  if (typeof globalThis.process === "undefined") {
    globalThis.process = {
      env: {},
      argv: [],
      platform: "quickjs",
      versions: {},
      exit: function() {},
      stdout: { write: function(s) { if (typeof __vorn_log__ !== "undefined") __vorn_log__("LOG", String(s)); } },
      stderr: { write: function(s) { if (typeof __vorn_log__ !== "undefined") __vorn_log__("ERROR", String(s)); } },
      stdin:  { setEncoding: function() {}, on: function() {}, off: function() {}, removeListener: function() {} },
    };
  }

  var _timers = {}, _seq = 0;

  globalThis.setInterval = function(cb, ms) {
    var id = ++_seq; _timers[id] = cb;
    __vorn_set_interval__(id, ms | 0, false); return id;
  };
  globalThis.setTimeout = function(cb, ms) {
    var id = ++_seq; _timers[id] = function() { delete _timers[id]; cb(); };
    __vorn_set_interval__(id, ms | 0, true); return id;
  };
  globalThis.clearInterval = function(id) { delete _timers[id]; __vorn_clear_interval__(id); };
  globalThis.clearTimeout  = globalThis.clearInterval;

  globalThis.__vorn_fire_timer__ = function(id) {
    var cb = _timers[id]; if (cb) cb();
  };

  // Pre-defined call dispatcher — used by Rust instead of ctx.eval() per call.
  // Eliminates JS recompilation overhead on every backend function call.
  // Returns a JSON string (or throws on error), never undefined.
  globalThis.__vorn_do_call__ = function(fn_name, args_json) {
    var mod = globalThis.__vorn_mod__;
    if (!mod) throw new Error("__vorn_mod__ not set — did app.start() run?");
    var fn = mod[fn_name];
    if (typeof fn !== "function") throw new Error('"' + fn_name + '" is not an exported function');
    var args = JSON.parse(args_json);
    var result = fn.apply(null, args);
    if (result !== null && result !== undefined && typeof result === "object" && typeof result.then === "function") {
      throw new Error('"' + fn_name + '" is async — async backend functions are not supported in lite mode. Use synchronous functions only.');
    }
    if (result === undefined || result === null) return "null";
    return JSON.stringify(result);
  };
})();
"#;

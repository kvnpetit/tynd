//! The QuickJS worker thread. Owns the `Runtime` + `Context`, registers
//! every native bridge (timer, emit, yield, return, polyfills), evals the
//! user bundle, then drives the message loop that multiplexes frontend
//! calls, timer fires, and polyfill events.

#![allow(elided_lifetimes_in_paths)]

use rquickjs::{Context, Function, Runtime};
use serde_json::Value;
use std::sync::mpsc;
use tynd_host::{
    runtime::{BackendCall, BackendConfig, BackendEvent, MenuItemDef, TrayConfig},
    tynd_log,
};

use super::timer::{start_timer_thread, TimerCmd};
use super::JsMsg;
use crate::polyfills;

const JS_GLOBALS: &str = include_str!("globals.js");

#[allow(clippy::needless_pass_by_value)]
pub(super) fn js_thread_main(
    bundle_code: String,
    js_tx: mpsc::Sender<JsMsg>,
    js_rx: mpsc::Receiver<JsMsg>,
    config_tx: mpsc::SyncSender<BackendConfig>,
    event_tx: mpsc::Sender<BackendEvent>,
) {
    let rt = Runtime::new().expect("QuickJS Runtime::new failed");
    let ctx = Context::full(&rt).expect("QuickJS Context::full failed");

    let (timer_tx, timer_rx) = mpsc::sync_channel::<TimerCmd>(64);
    start_timer_thread(timer_rx, js_tx.clone());

    if !install_globals(&ctx, &event_tx, &js_tx, &timer_tx, &bundle_code) {
        return;
    }

    let config = read_config(&ctx);
    let _ = config_tx.send(config);

    run_message_loop(&ctx, &js_rx, &event_tx, &rt);
}

/// Register every native bridge, eval `JS_GLOBALS`, install polyfills, and
/// finally eval the user bundle. Returns `false` (and emits a `BackendEvent::
/// Error`) if the bundle throws — the caller must then drop out without
/// sending a config so the outer `start` fails visibly.
fn install_globals(
    ctx: &Context,
    event_tx: &mpsc::Sender<BackendEvent>,
    js_tx: &mpsc::Sender<JsMsg>,
    timer_tx: &mpsc::SyncSender<TimerCmd>,
    bundle_code: &str,
) -> bool {
    let result = ctx.with(|ctx| -> rquickjs::Result<()> {
        let g = ctx.globals();

        g.set("__tynd_lite__", true)?;

        g.set(
            "__tynd_log__",
            Function::new(ctx.clone(), |level: String, msg: String| {
                eprintln!("[{level}] {msg}");
            })?,
        )?;

        {
            let tx = event_tx.clone();
            g.set(
                "__tynd_emit__",
                Function::new(ctx.clone(), move |name: String, payload_json: String| {
                    let payload = serde_json::from_str(&payload_json).unwrap_or(Value::Null);
                    let _ = tx.send(BackendEvent::Emit { name, payload });
                })?,
            )?;
        }

        {
            let tx = event_tx.clone();
            g.set(
                "__tynd_yield__",
                Function::new(ctx.clone(), move |id: String, value_json: String| {
                    let value = serde_json::from_str(&value_json).unwrap_or(Value::Null);
                    let _ = tx.send(BackendEvent::Yield { id, value });
                })?,
            )?;
        }

        {
            let tx = event_tx.clone();
            g.set(
                "__tynd_return__",
                Function::new(
                    ctx.clone(),
                    move |id: String, ok: bool, value_json: String| {
                        let value = serde_json::from_str(&value_json).unwrap_or(Value::Null);
                        let _ = tx.send(BackendEvent::Return { id, ok, value });
                    },
                )?,
            )?;
        }

        {
            let tx = timer_tx.clone();
            g.set(
                "__tynd_set_interval__",
                Function::new(ctx.clone(), move |id: u32, ms: u32, once: bool| {
                    let _ = tx.send(TimerCmd::Set { id, ms, once });
                })?,
            )?;
        }

        {
            let tx = timer_tx.clone();
            g.set(
                "__tynd_clear_interval__",
                Function::new(ctx.clone(), move |id: u32| {
                    let _ = tx.send(TimerCmd::Clear(id));
                })?,
            )?;
        }

        ctx.eval::<(), _>(JS_GLOBALS)?;
        polyfills::install(&ctx, event_tx, js_tx)?;
        ctx.eval::<(), _>(bundle_code.as_bytes())?;
        Ok(())
    });

    if let Err(e) = result {
        let msg = format!("Bundle evaluation failed: {e}");
        tynd_log!("{msg}");
        let _ = event_tx.send(BackendEvent::Error { message: msg });
        return false;
    }
    true
}

fn read_config(ctx: &Context) -> BackendConfig {
    ctx.with(|ctx| {
        let json: Option<String> = ctx.globals().get("__tynd_config__").unwrap_or(None);
        json.and_then(|s| parse_config(&s)).unwrap_or_default()
    })
}

fn run_message_loop(
    ctx: &Context,
    js_rx: &mpsc::Receiver<JsMsg>,
    event_tx: &mpsc::Sender<BackendEvent>,
    rt: &Runtime,
) {
    while let Ok(msg) = js_rx.recv() {
        match msg {
            JsMsg::Call(call) => handle_call(ctx, call, event_tx),
            JsMsg::TimerFire(id) => fire_timer(ctx, id),
            JsMsg::PolyfillEvent { name, data } => deliver_polyfill_event(ctx, &name, &data),
        }

        // Drive Promise microtasks after each message.
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

fn handle_call(ctx: &Context, call: BackendCall, event_tx: &mpsc::Sender<BackendEvent>) {
    match call {
        BackendCall::Typed { id, fn_name, args } => {
            if fn_name.starts_with("__tynd_") {
                let result = ctx.with(|ctx| call_global(&ctx, &fn_name));
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
            } else {
                dispatch_call(ctx, &id, &fn_name, &args, event_tx);
            }
        },
        BackendCall::Raw(json) => {
            let Ok(v) = serde_json::from_str::<Value>(&json) else {
                return;
            };
            let id = v["id"].as_str().unwrap_or("").to_string();
            let fn_name = v["fn"].as_str().unwrap_or("").to_string();
            let args = v["args"].as_array().cloned().unwrap_or_default();
            dispatch_call(ctx, &id, &fn_name, &args, event_tx);
        },
        BackendCall::Cancel { id } => {
            let _ = ctx.with(|ctx| -> rquickjs::Result<()> {
                let g = ctx.globals();
                if let Ok(f) = g.get::<_, Function>("__tynd_cancel__") {
                    let _ = f.call::<_, ()>((id,));
                }
                Ok(())
            });
        },
    }
}

fn fire_timer(ctx: &Context, id: u32) {
    let _ = ctx.with(|ctx| -> rquickjs::Result<()> {
        let fire: Function = ctx.globals().get("__tynd_fire_timer__")?;
        fire.call::<_, ()>((id,))
    });
}

fn deliver_polyfill_event(ctx: &Context, name: &str, data: &Value) {
    let _ = ctx.with(|ctx| -> rquickjs::Result<()> {
        let g = ctx.globals();
        let Ok(deliver) = g.get::<_, Function>("__tynd_polyfill_event__") else {
            return Ok(());
        };
        let data_json = serde_json::to_string(data).unwrap_or_else(|_| "null".into());
        let _ = deliver.call::<_, ()>((name.to_string(), data_json));
        Ok(())
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

/// Dispatch a user-module call through the JS-side `__tynd_call__` helper.
/// That helper handles async values and async iterables, invoking the native
/// `__tynd_yield__` / `__tynd_return__` callbacks which push BackendEvents.
/// Any synchronous error (e.g. unknown function) is reported as a Return event.
fn dispatch_call(
    rt_ctx: &Context,
    id: &str,
    fn_name: &str,
    args: &[Value],
    event_tx: &mpsc::Sender<BackendEvent>,
) {
    let args_json = serde_json::to_string(args).unwrap_or_else(|_| "[]".into());
    let result: Result<(), String> = rt_ctx.with(|ctx| {
        let call_fn: Function = ctx
            .globals()
            .get("__tynd_call__")
            .map_err(|e| format!("__tynd_call__ missing: {e}"))?;
        call_fn
            .call::<_, ()>((id.to_string(), fn_name.to_string(), args_json))
            .map_err(|e| e.to_string())
    });
    if let Err(error) = result {
        let _ = event_tx.send(BackendEvent::Return {
            id: id.to_string(),
            ok: false,
            value: Value::String(error),
        });
    }
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

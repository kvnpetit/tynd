mod bridge;
pub(crate) mod catalog;
mod discovery;
mod prompts;
mod resources;
mod runtime;
mod server;
mod tools;

use std::{
    io::{self, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::args::{McpCallArgs, McpCli, McpCommand, McpTapArgs};
use runtime::{connect_newest, sanitize_json, Runtime, RuntimeConfig};

const MAX_SCREENSHOT_BASE64_BYTES: usize = 16 * 1024 * 1024;

const ALLOWED_RAW_METHODS: [&str; 34] = [
    "app.info",
    "app.quit",
    "app.restart",
    "app.logs.tail",
    "window.list",
    "window.info",
    "window.manage",
    "screenshot.window",
    "screenshot.thumbnail",
    "webview.snapshot",
    "webview.find_element",
    "webview.console_logs",
    "webview.network_requests",
    "webview.click",
    "webview.type",
    "webview.fill",
    "webview.press_key",
    "webview.scroll",
    "webview.hover",
    "webview.drag",
    "webview.evaluate",
    "ipc.recent",
    "ipc.failed",
    "ipc.replay",
    "ipc.tap_start",
    "ipc.tap_stop",
    "plugin.list",
    "plugin.docs",
    "plugin.command_catalog",
    "capability.list",
    "capability.check",
    "capability.suggest_grant",
    "runtime.snapshot",
    "unsubscribe",
];

pub fn run(cli: McpCli) -> Result<()> {
    init_logging();
    let base = RuntimeConfig {
        allow_eval: cli.allow_eval,
        allow_launch: cli.allow_launch,
        auto_switch: cli.auto_switch,
        allow_non_loopback: cli.allow_non_loopback_discovery,
        app_id: cli.app_id.clone(),
        rescan_interval: Duration::from_millis(cli.rescan_interval_ms),
        connect_timeout: Duration::from_millis(cli.connect_timeout_ms),
    };
    match cli.command {
        None | Some(McpCommand::Serve(_)) => {
            let mut runtime = Runtime::new(base);
            server::serve(&mut runtime)
        },
        Some(McpCommand::Call(arguments)) => run_call(base, arguments),
        Some(McpCommand::Tap(arguments)) => run_tap(base, arguments),
    }
}

fn run_call(mut config: RuntimeConfig, arguments: McpCallArgs) -> Result<()> {
    if !ALLOWED_RAW_METHODS.contains(&arguments.tool.as_str()) {
        bail!("unknown tool: {}", arguments.tool);
    }
    if arguments.tool == "webview.evaluate" && !config.allow_eval {
        bail!("webview.evaluate is disabled. Pass `--allow-eval` to enable JS evaluation.");
    }
    config.app_id = arguments.app_id.or(config.app_id);
    config.connect_timeout = Duration::from_millis(arguments.connect_timeout_ms);
    let payload = match arguments.json_args {
        Some(raw) => serde_json::from_str(&raw)
            .with_context(|| "failed to parse JSON args; quote the JSON object passed to --args")?,
        None => Value::Null,
    };
    let (_entry, mut client) = connect_newest(&config)?;
    let result = client.request(&arguments.tool, payload)?;
    let encoded = serde_json::to_vec(&result).context("encode raw bridge result")?;
    if encoded.len() > arguments.max_bytes {
        bail!(
            "raw bridge result exceeds cap ({} > {} bytes)",
            encoded.len(),
            arguments.max_bytes
        );
    }
    let safe = sanitize_json(result);
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &safe)?;
    stdout.write_all(b"\n")?;
    Ok(())
}

fn run_tap(mut config: RuntimeConfig, arguments: McpTapArgs) -> Result<()> {
    config.app_id = arguments.app_id.or(config.app_id);
    config.connect_timeout = Duration::from_millis(arguments.connect_timeout_ms);
    let (entry, mut client) = connect_newest(&config)?;
    let running = Arc::new(AtomicBool::new(true));
    let signal_flag = Arc::clone(&running);
    ctrlc::set_handler(move || signal_flag.store(false, Ordering::SeqCst))
        .context("install Ctrl+C handler")?;

    let started = client.request("ipc.tap_start", Value::Null)?;
    let subscription_id = started
        .get("subscriptionId")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("ipc.tap_start: host did not return a numeric subscriptionId"))?;
    eprintln!(
        "ipc tap -> {} (pid {}); press Ctrl+C to stop",
        entry.app_id, entry.pid
    );

    client.set_notification_polling(true)?;
    let tap_result = (|| -> Result<()> {
        let mut stdout = io::BufWriter::new(io::stdout().lock());
        while running.load(Ordering::SeqCst) {
            match client.read_notification() {
                Ok(Some(notification)) if notification.method == "ipc.frame" => {
                    let payload = unwrap_tap_payload(notification.params);
                    serde_json::to_writer(&mut stdout, &sanitize_json(payload))?;
                    stdout.write_all(b"\n")?;
                    stdout.flush()?;
                },
                Ok(Some(_)) => {},
                Ok(None) => break,
                Err(error) if running.load(Ordering::SeqCst) && is_timeout(&error) => {
                    std::thread::sleep(Duration::from_millis(25));
                },
                Err(error) if !running.load(Ordering::SeqCst) => {
                    tracing::debug!("tap stopped after read error: {error:#}");
                    break;
                },
                Err(error) => return Err(error),
            }
        }
        Ok(())
    })();

    let polling_result = client.set_notification_polling(false);
    let unsubscribe_result = client.request("unsubscribe", json!({ "id": subscription_id }));
    tap_result?;
    polling_result?;
    if let Err(error) = unsubscribe_result {
        tracing::debug!("failed to unsubscribe tap: {error:#}");
    }
    Ok(())
}

fn unwrap_tap_payload(params: Value) -> Value {
    if params.get("subscriptionId").is_some() {
        params.get("payload").cloned().unwrap_or(params)
    } else {
        params
    }
}

fn is_timeout(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|source| {
            matches!(
                source.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            )
        })
    })
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .try_init();
}

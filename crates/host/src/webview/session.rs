//! Webview session commands of `tyndWebview`: script evaluation with a
//! result, browsing-data reset, PDF export, page capture, audio mute and
//! printing (dialog or silent) with printer enumeration.
//!
//! Every command targets the calling webview, or another one through `label`
//! when the caller shows app content. Results are posted back through
//! [`UserEvent::OsResult`] because evaluation, PDF export and capture
//! complete asynchronously on the UI thread.

use serde_json::{json, Map, Value};
use tao::event_loop::EventLoopProxy;
use wry::WebView;

use crate::app::UserEvent;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "windows")]
use windows as platform;

/// Window-API methods handled here.
const METHODS: &[&str] = &[
    "evaluateScript",
    "clearBrowsingData",
    "printToPdf",
    "capturePage",
    "setAudioMuted",
    "isAudioMuted",
    "printPage",
    "getPrinters",
];

/// Longest printer name accepted by `printPage`.
const MAX_PRINTER_NAME: usize = 256;

/// Largest script accepted by `evaluateScript`.
const MAX_SCRIPT_BYTES: usize = 1024 * 1024;

pub(crate) fn is_session_method(method: &str) -> bool {
    METHODS.contains(&method)
}

/// Delivers a session result to the calling webview.
type Deliver = Box<dyn FnOnce(Result<Value, String>) + Send>;

/// Deferred IPC reply to the calling webview. Dropping it unanswered (the
/// engine discarded a callback, or the webview was destroyed before it fired)
/// still settles the call with an error, so the caller's promise never hangs.
pub(crate) struct Reply {
    method: String,
    deliver: Option<Deliver>,
}

impl Reply {
    pub(crate) fn new(
        proxy: &EventLoopProxy<UserEvent>,
        label: &str,
        id: &str,
        method: &str,
    ) -> Self {
        let proxy = proxy.clone();
        let label = label.to_owned();
        let id = id.to_owned();
        Self::from_fn(
            method,
            Box::new(move |result| {
                let (ok, value) = match result {
                    Ok(value) => (true, value),
                    Err(error) => (false, Value::String(error)),
                };
                if proxy
                    .send_event(UserEvent::OsResult {
                        label,
                        id,
                        ok,
                        value,
                    })
                    .is_err()
                {
                    crate::tynd_warn!("webview session reply dropped: event loop closed");
                }
            }),
        )
    }

    fn from_fn(method: &str, deliver: Deliver) -> Self {
        Self {
            method: method.to_owned(),
            deliver: Some(deliver),
        }
    }

    fn send(mut self, result: Result<Value, String>) {
        if let Some(deliver) = self.deliver.take() {
            deliver(result);
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if let Some(deliver) = self.deliver.take() {
            deliver(Err(format!(
                "{}: webview went away before answering",
                self.method
            )));
        }
    }
}

/// Run a session `method` for the webview `caller`. `caller_origin` is the
/// calling frame's origin (`None` = app content). `lookup` resolves a label
/// to its live webview.
pub(crate) fn dispatch<'a>(
    method: &str,
    caller: &str,
    caller_origin: Option<&str>,
    args: &Value,
    lookup: impl Fn(&str) -> Option<&'a WebView>,
    reply: Reply,
) {
    let target = match resolve_target(method, args, caller, caller_origin) {
        Ok(target) => target,
        Err(error) => return reply.send(Err(error)),
    };
    let Some(webview) = lookup(target) else {
        return reply.send(Err(format!("{method}: webview '{target}' not found")));
    };
    match method {
        "evaluateScript" => evaluate_script(webview, args, reply),
        "clearBrowsingData" => match ClearScope::from_args(args) {
            Ok(scope) => platform::clear_data(webview, scope, reply),
            Err(error) => reply.send(Err(error)),
        },
        "printToPdf" => match PdfOptions::from_args(args) {
            Ok(options) => platform::print_to_pdf(webview, &options, reply),
            Err(error) => reply.send(Err(error)),
        },
        "capturePage" => platform::capture_page(webview, reply),
        "setAudioMuted" => match args.get("muted").and_then(Value::as_bool) {
            Some(muted) => reply.send(platform::set_muted(webview, muted).map(|()| Value::Null)),
            None => reply.send(Err("setAudioMuted: 'muted' must be a boolean".into())),
        },
        "isAudioMuted" => reply.send(platform::is_muted(webview).map(Value::Bool)),
        "printPage" => match PrintRequest::from_args(args, caller_origin) {
            Ok(PrintRequest { silent: false, .. }) => reply.send(
                webview
                    .print()
                    .map(|()| Value::Null)
                    .map_err(|error| format!("print: {error}")),
            ),
            Ok(PrintRequest { printer, .. }) => {
                platform::print_silently(webview, printer.as_deref(), reply);
            },
            Err(error) => reply.send(Err(error)),
        },
        "getPrinters" => reply.send(platform::printers()),
        _ => reply.send(Err(format!("{method}: unknown webview method"))),
    }
}

/// Label targeted by a call: the caller itself, or `args.label`. Only app
/// content may reach another webview.
fn resolve_target<'a>(
    method: &str,
    args: &'a Value,
    caller: &'a str,
    caller_origin: Option<&str>,
) -> Result<&'a str, String> {
    let target = match args.get("label") {
        None | Some(Value::Null) => caller,
        Some(Value::String(label)) if !label.is_empty() => label.as_str(),
        Some(_) => return Err(format!("{method}: 'label' must be a non-empty string")),
    };
    if target != caller && caller_origin.is_some() {
        return Err(format!(
            "{method}: a webview that shows remote content can only target itself"
        ));
    }
    Ok(target)
}

// ---------------------------------------------------------------------------
// evaluateScript
// ---------------------------------------------------------------------------

fn parse_script(args: &Value) -> Result<&str, String> {
    let script = args
        .get("script")
        .and_then(Value::as_str)
        .filter(|script| !script.trim().is_empty())
        .ok_or("evaluateScript: 'script' must be a non-empty string")?;
    if script.len() > MAX_SCRIPT_BYTES {
        return Err(format!(
            "evaluateScript: script exceeds {MAX_SCRIPT_BYTES} bytes"
        ));
    }
    Ok(script)
}

/// Wrap an expression so the page returns a JSON envelope:
/// `{"ok":true,"value":...}` or `{"ok":false,"error":"..."}`. The script is
/// inlined (not `eval`ed) so a page CSP without `unsafe-eval` still runs it.
fn evaluation_wrapper(script: &str) -> String {
    format!(
        "(function(){{try{{var __tynd_v=(\n{script}\n);\
         return JSON.stringify({{ok:true,value:__tynd_v===undefined?null:__tynd_v}});}}\
         catch(e){{return JSON.stringify({{ok:false,error:String(e&&e.message||e)}});}}}})()"
    )
}

/// Decode what the engine returned for [`evaluation_wrapper`]. Engines hand
/// back the JSON serialization of the returned string; a bare envelope object
/// is accepted too.
fn decode_evaluation(raw: &str) -> Result<Value, String> {
    let outer: Value = serde_json::from_str(raw)
        .map_err(|_| "evaluateScript: the page returned no result (syntax error?)".to_owned())?;
    let envelope = match outer {
        Value::String(inner) => serde_json::from_str(&inner)
            .map_err(|error| format!("evaluateScript: malformed result: {error}"))?,
        other => other,
    };
    let Value::Object(mut envelope) = envelope else {
        return Err("evaluateScript: the page returned no result (syntax error?)".into());
    };
    if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(envelope.remove("value").unwrap_or(Value::Null))
    } else {
        let error = envelope
            .remove("error")
            .and_then(|error| error.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown error".into());
        Err(format!("evaluateScript: {error}"))
    }
}

fn evaluate_script(webview: &WebView, args: &Value, reply: Reply) {
    let script = match parse_script(args) {
        Ok(script) => evaluation_wrapper(script),
        Err(error) => return reply.send(Err(error)),
    };
    // wry drops the callback when evaluation fails before it runs; the
    // shared slot lets the error path answer instead.
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(reply)));
    let callback_slot = std::sync::Arc::clone(&slot);
    let result = webview.evaluate_script_with_callback(&script, move |raw| {
        if let Some(reply) = take_reply(&callback_slot) {
            reply.send(decode_evaluation(&raw));
        }
    });
    if let Err(error) = result {
        if let Some(reply) = take_reply(&slot) {
            reply.send(Err(format!("evaluateScript: {error}")));
        }
    }
}

fn take_reply(slot: &std::sync::Mutex<Option<Reply>>) -> Option<Reply> {
    slot.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

// ---------------------------------------------------------------------------
// printToPdf
// ---------------------------------------------------------------------------

/// Validated `printToPdf` options. Lengths are inches. WKWebView renders one
/// continuous page and ignores them.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
struct PdfOptions {
    landscape: bool,
    print_background: bool,
    scale: f64,
    /// `(width, height)`; `None` = engine default (US Letter).
    page_size: Option<(f64, f64)>,
    /// `(top, right, bottom, left)`; `None` = engine default.
    margins: Option<[f64; 4]>,
}

const PAGE_SIZES: &[(&str, f64, f64)] = &[
    ("A3", 11.69, 16.54),
    ("A4", 8.27, 11.69),
    ("A5", 5.83, 8.27),
    ("Letter", 8.5, 11.0),
    ("Legal", 8.5, 14.0),
    ("Tabloid", 11.0, 17.0),
];

impl PdfOptions {
    fn from_args(args: &Value) -> Result<Self, String> {
        let empty = Map::new();
        let options = match args.get("options") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(options)) => options,
            Some(_) => return Err("printToPdf: options must be an object".into()),
        };
        let flag = |key: &str| match options.get(key) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(value)) => Ok(*value),
            Some(_) => Err(format!("printToPdf: '{key}' must be a boolean")),
        };
        let scale = match options.get("scale") {
            None | Some(Value::Null) => 1.0,
            Some(value) => number_in(value, "scale", 0.1, 2.0)?,
        };
        Ok(Self {
            landscape: flag("landscape")?,
            print_background: flag("printBackground")?,
            scale,
            page_size: parse_page_size(options.get("pageSize"))?,
            margins: parse_margins(options.get("margins"))?,
        })
    }

    /// Parameters of the DevTools `Page.printToPDF` command.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    fn to_cdp(&self) -> Value {
        let mut params = json!({
            "landscape": self.landscape,
            "printBackground": self.print_background,
            "scale": self.scale,
        });
        if let Some((width, height)) = self.page_size {
            params["paperWidth"] = json!(width);
            params["paperHeight"] = json!(height);
        }
        if let Some([top, right, bottom, left]) = self.margins {
            params["marginTop"] = json!(top);
            params["marginRight"] = json!(right);
            params["marginBottom"] = json!(bottom);
            params["marginLeft"] = json!(left);
        }
        params
    }
}

fn number_in(value: &Value, key: &str, min: f64, max: f64) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|number| number.is_finite() && (min..=max).contains(number))
        .ok_or_else(|| format!("printToPdf: '{key}' must be a number between {min} and {max}"))
}

fn parse_page_size(raw: Option<&Value>) -> Result<Option<(f64, f64)>, String> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(name)) => PAGE_SIZES
            .iter()
            .find(|(candidate, _, _)| candidate == name)
            .map(|&(_, width, height)| Some((width, height)))
            .ok_or_else(|| {
                let names: Vec<&str> = PAGE_SIZES.iter().map(|(name, _, _)| *name).collect();
                format!(
                    "printToPdf: unknown pageSize '{name}' (expected {} or {{ width, height }} in inches)",
                    names.join(", ")
                )
            }),
        Some(Value::Object(size)) => {
            let dimension = |key: &str| {
                size.get(key)
                    .ok_or_else(|| format!("printToPdf: pageSize.{key} is required"))
                    .and_then(|value| number_in(value, &format!("pageSize.{key}"), 1.0, 100.0))
            };
            Ok(Some((dimension("width")?, dimension("height")?)))
        },
        Some(_) => Err("printToPdf: pageSize must be a name or { width, height }".into()),
    }
}

fn parse_margins(raw: Option<&Value>) -> Result<Option<[f64; 4]>, String> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(value @ Value::Number(_)) => {
            let all = number_in(value, "margins", 0.0, 10.0)?;
            Ok(Some([all; 4]))
        },
        Some(Value::Object(margins)) => {
            let side = |key: &str| {
                margins.get(key).map_or(Ok(0.0), |value| {
                    number_in(value, &format!("margins.{key}"), 0.0, 10.0)
                })
            };
            Ok(Some([
                side("top")?,
                side("right")?,
                side("bottom")?,
                side("left")?,
            ]))
        },
        Some(_) => {
            Err("printToPdf: margins must be a number or { top, right, bottom, left }".into())
        },
    }
}

/// Extract `data` (base64) from a DevTools response.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn cdp_data(method: &str, raw: &str) -> Result<Value, String> {
    let parsed: Value = serde_json::from_str(raw)
        .map_err(|error| format!("{method}: malformed result: {error}"))?;
    parsed
        .get("data")
        .and_then(Value::as_str)
        .map(|data| Value::String(data.to_owned()))
        .ok_or_else(|| format!("{method}: the engine returned no data"))
}

// ---------------------------------------------------------------------------
// clearBrowsingData
// ---------------------------------------------------------------------------

/// Browsing-data categories that `clearBrowsingData` can select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DataKinds(u8);

impl DataKinds {
    const COOKIES: u8 = 1;
    const CACHE: u8 = 1 << 1;
    const LOCAL_STORAGE: u8 = 1 << 2;
    const INDEXED_DB: u8 = 1 << 3;
    const SERVICE_WORKERS: u8 = 1 << 4;

    /// Option key of every category.
    const KEYS: &[(&str, u8)] = &[
        ("cookies", Self::COOKIES),
        ("cache", Self::CACHE),
        ("localStorage", Self::LOCAL_STORAGE),
        ("indexedDb", Self::INDEXED_DB),
        ("serviceWorkers", Self::SERVICE_WORKERS),
    ];

    fn has(self, kind: u8) -> bool {
        self.0 & kind != 0
    }
}

/// What `clearBrowsingData` removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClearScope {
    /// Every kind of data the engine stores (history, autofill, storage...).
    Everything,
    /// Only the selected categories.
    Only(DataKinds),
}

impl ClearScope {
    /// No category selected (or `all: true`) clears everything.
    fn from_args(args: &Value) -> Result<Self, String> {
        let flag = |key: &str| match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(value)) => Ok(Some(*value)),
            Some(_) => Err(format!("clearBrowsingData: '{key}' must be a boolean")),
        };
        let all = flag("all")?;
        let mut kinds = 0;
        for &(key, kind) in DataKinds::KEYS {
            if flag(key)? == Some(true) {
                kinds |= kind;
            }
        }
        match (all, kinds) {
            (Some(true), _) | (None, 0) => Ok(Self::Everything),
            (Some(false), 0) => Err(
                "clearBrowsingData: nothing to clear (pass all: true or select cookies, cache, localStorage, indexedDb or serviceWorkers)"
                    .into(),
            ),
            (_, kinds) => Ok(Self::Only(DataKinds(kinds))),
        }
    }
}

// ---------------------------------------------------------------------------
// printPage / getPrinters
// ---------------------------------------------------------------------------

/// Validated `printPage` arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PrintRequest {
    /// Print without the OS dialog.
    silent: bool,
    /// Named printer for a silent print; `None` = the OS default printer.
    printer: Option<String>,
}

impl PrintRequest {
    /// Silent printing is reserved to app content: a remote page must not
    /// spool jobs without the user seeing a dialog.
    fn from_args(args: &Value, caller_origin: Option<&str>) -> Result<Self, String> {
        let silent = match args.get("silent") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err("print: 'silent' must be a boolean".into()),
        };
        let printer = match args.get("printer") {
            None | Some(Value::Null) => None,
            Some(Value::String(name))
                if !name.trim().is_empty()
                    && name.chars().count() <= MAX_PRINTER_NAME
                    && !name.chars().any(char::is_control) =>
            {
                Some(name.clone())
            },
            Some(_) => {
                return Err(format!(
                    "print: 'printer' must be a printer name (1..{MAX_PRINTER_NAME} characters, no control characters)"
                ))
            },
        };
        if printer.is_some() && !silent {
            return Err(
                "print: 'printer' needs silent: true (the print dialog lets the user pick the printer)"
                    .into(),
            );
        }
        if silent && caller_origin.is_some() {
            return Err("print: silent printing is only available to app content".into());
        }
        Ok(Self { silent, printer })
    }
}

#[cfg(not(target_os = "windows"))]
fn unsupported(method: &str) -> String {
    let os = if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    };
    format!("{method}: not supported on {os}")
}

#[cfg(test)]
mod tests;

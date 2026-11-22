use serde_json::json;
use tynd_plugin_sdk::{PluginResult, UriResponse};

use super::*;

/// Registry, hook chain and setup registry are process-global: every test
/// registers plugins under names unique to it and only inspects those.
struct Probe {
    name: &'static str,
    events: Arc<Mutex<Vec<HostEvent>>>,
    panic_on_event: bool,
}

impl Probe {
    fn new(name: &'static str) -> (Arc<Self>, Arc<Mutex<Vec<HostEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let probe = Arc::new(Self {
            name,
            events: Arc::clone(&events),
            panic_on_event: false,
        });
        (probe, events)
    }
}

impl TyndPlugin for Probe {
    fn name(&self) -> &'static str {
        self.name
    }

    fn init(&self, cx: &PluginCx) -> PluginResult<()> {
        cx.register_init_script(format!("window.__{} = true;", self.name))?;
        cx.register_uri_scheme(
            format!("{}-scheme", self.name.replace("ws2-", "ws2")),
            Arc::new(|_| UriResponse {
                status: 204,
                ..UriResponse::default()
            }),
        )?;
        Ok(())
    }

    fn dispatch(&self, cx: &PluginCx, method: &str, _args: Value) -> PluginResult<Value> {
        match method {
            "caller" => Ok(cx
                .caller()
                .map_or(Value::Null, |c| json!([c.window, c.webview, c.origin]))),
            _ => Ok(json!("ok")),
        }
    }

    fn on_event(&self, _cx: &PluginCx, event: &HostEvent) {
        assert!(!self.panic_on_event, "probe panic");
        self.events.lock().push(event.clone());
    }
}

fn setup(plugin: Arc<dyn TyndPlugin>) {
    install_emit(Arc::new(|_, _| {}));
    assert!(register(plugin));
}

#[test]
fn caller_context_reaches_plugin_dispatch_only_inside_scope() {
    let (probe, _) = Probe::new("ws2-caller");
    setup(probe);

    assert_eq!(dispatch("ws2-caller:caller", Value::Null), Ok(Value::Null));
    let seen = with_call_context("main", "child", Some("https://example.com"), || {
        dispatch("ws2-caller:caller", Value::Null)
    });
    assert_eq!(seen, Ok(json!(["main", "child", "https://example.com"])));
    assert_eq!(dispatch("ws2-caller:caller", Value::Null), Ok(Value::Null));
}

#[test]
fn before_dispatch_chain_audits_and_rejects() {
    let (probe, _) = Probe::new("ws2-hooked");
    setup(probe);
    type Seen = Vec<(String, String, Option<CallerInfo>)>;
    let seen: Arc<Mutex<Seen>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    add_before_dispatch(Arc::new(move |info| {
        if info.plugin == "ws2-hooked" {
            sink.lock().push((
                info.plugin.to_string(),
                info.method.to_string(),
                info.caller.clone(),
            ));
        }
        Ok(())
    }));
    add_before_dispatch(Arc::new(|info| {
        if info.plugin == "ws2-hooked" && info.method == "blocked" {
            return Err("rate limited".to_string());
        }
        assert!(
            !(info.plugin == "ws2-hooked" && info.method == "explode"),
            "hook bug"
        );
        Ok(())
    }));

    assert_eq!(dispatch("ws2-hooked:fine", Value::Null), Ok(json!("ok")));
    assert_eq!(
        dispatch("ws2-hooked:blocked", Value::Null),
        Err("rate limited".to_string())
    );
    let explode = dispatch("ws2-hooked:explode", Value::Null).unwrap_err();
    assert!(
        explode.contains("before-dispatch hook panicked"),
        "{explode}"
    );

    let calls = seen.lock();
    let methods: Vec<_> = calls.iter().map(|(_, m, _)| m.as_str()).collect();
    // First hook runs for every call, even ones the second hook rejects.
    assert_eq!(methods, ["fine", "blocked", "explode"]);
    assert!(calls.iter().all(|(_, _, caller)| caller.is_none()));
}

#[test]
fn empty_chain_is_a_no_op() {
    let (probe, _) = Probe::new("ws2-nohooks");
    setup(probe);
    assert_eq!(dispatch("ws2-nohooks:x", Value::Null), Ok(json!("ok")));
}

#[test]
fn broadcast_delivers_to_plugins_and_survives_a_panicking_one() {
    let (good, good_events) = Probe::new("ws2-listener");
    let bad = Arc::new(Probe {
        name: "ws2-panicker",
        events: Arc::new(Mutex::new(Vec::new())),
        panic_on_event: true,
    });
    setup(bad);
    setup(good);

    let event = HostEvent::WindowFocus {
        label: "ws2-focus-label".to_string(),
        focused: true,
    };
    broadcast_event(&event);
    broadcast_event(&HostEvent::Navigation {
        window: "w".into(),
        webview: "v".into(),
        url: "https://ws2.example/".into(),
    });

    let got = good_events.lock().clone();
    assert!(got.contains(&event));
    assert!(got.iter().any(|e| matches!(
        e,
        HostEvent::Navigation { url, .. } if url == "https://ws2.example/"
    )));
}

#[test]
fn plugin_init_registers_init_script_and_uri_scheme() {
    let (probe, _) = Probe::new("ws2-setup");
    setup(probe);

    assert!(setup::init_script_entries()
        .iter()
        .any(|entry| entry.script == "window.__ws2-setup = true;"));
    let handler = setup::uri_scheme_entries()
        .into_iter()
        .find(|entry| entry.name == "ws2setup-scheme")
        .map(|entry| entry.handler)
        .expect("scheme registered by plugin init");
    let response = handler(tynd_plugin_sdk::UriRequest::default());
    assert_eq!(response.status, 204);
}

use super::*;
use serde_json::json;

/// Strip the `window.f&&window.f(` prefix and `)` suffix, then parse the
/// argument list as a JSON array so literals round-trip through serde.
fn parse_args(call: &str, name: &str) -> Value {
    let prefix = format!("window.{name}&&window.{name}(");
    let inner = call
        .strip_prefix(&prefix)
        .unwrap_or_else(|| panic!("unexpected prefix in {call}"))
        .strip_suffix(')')
        .expect("closing paren");
    serde_json::from_str(&format!("[{inner}]")).expect("args are valid JSON")
}

const NASTY: &str = "quote\" back\\slash \n nl </script> \u{2028}\u{2029} \u{1f600}";

#[test]
fn js_str_round_trips_and_escapes_line_terminators() {
    let encoded = js_str(NASTY);
    assert!(!encoded.contains('\u{2028}'));
    assert!(!encoded.contains('\u{2029}'));
    assert!(encoded.contains("\\u2028"));
    let decoded: String = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, NASTY);
}

#[test]
fn js_val_escapes_nested_line_terminators() {
    let v = json!({ "k\u{2028}": ["a\u{2029}b", 1, null, true] });
    let encoded = js_val(&v);
    assert!(!encoded.contains('\u{2028}') && !encoded.contains('\u{2029}'));
    assert_eq!(serde_json::from_str::<Value>(&encoded).unwrap(), v);
}

#[test]
fn eval_resolve_round_trips() {
    let call = eval_resolve("id-1", true, NASTY);
    assert_eq!(
        parse_args(&call, "__tynd_resolve__"),
        json!(["id-1", true, NASTY])
    );
    let call = eval_resolve("id-2", false, "boom");
    assert_eq!(
        parse_args(&call, "__tynd_resolve__"),
        json!(["id-2", false, "boom"])
    );
}

#[test]
fn eval_yield_and_dispatch_round_trip() {
    assert_eq!(
        parse_args(&eval_yield("s", NASTY), "__tynd_yield__"),
        json!(["s", NASTY])
    );
    assert_eq!(
        parse_args(&eval_dispatch("evt", NASTY), "__tynd_dispatch__"),
        json!(["evt", NASTY])
    );
}

#[test]
fn eval_calls_guard_against_missing_shim() {
    for call in [
        eval_resolve("a", true, "b"),
        eval_yield("a", "b"),
        eval_dispatch("a", "b"),
        eval_os_result("a", true, &json!(1)),
        eval_os_event("a", &json!(1)),
    ] {
        assert!(call.starts_with("window.__tynd_"), "{call}");
        assert!(call.contains("&&window.__tynd_"), "{call}");
    }
}

#[test]
fn eval_os_result_and_event_round_trip() {
    let data = json!({ "path": "C:\\x\u{2028}y", "n": [1, 2] });
    assert_eq!(
        parse_args(&eval_os_result("r", false, &data), "__tynd_os_result__"),
        json!(["r", false, data.clone()])
    );
    assert_eq!(
        parse_args(&eval_os_event("e", &data), "__tynd_os_event__"),
        json!(["e", data])
    );
}

#[test]
fn eval_yield_batch_matches_pairs() {
    let pairs = vec![
        ("a".to_string(), "1".to_string()),
        ("a".to_string(), NASTY.to_string()),
        ("b\u{2028}".to_string(), "3".to_string()),
    ];
    let call = eval_yield_batch(&pairs);
    assert!(!call.contains('\u{2028}'));
    let args = parse_args(&call, "__tynd_yield_batch__");
    assert_eq!(
        args,
        json!([[["a", "1"], ["a", NASTY], ["b\u{2028}", "3"]]])
    );
}

#[test]
fn eval_yield_batch_empty() {
    assert_eq!(
        eval_yield_batch(&[]),
        "window.__tynd_yield_batch__&&window.__tynd_yield_batch__([])"
    );
}

#[test]
fn error_overlay_embeds_payload() {
    let script = eval_show_error_overlay("first line\nsecond line");
    assert!(!script.contains("__TYND_OVERLAY_PAYLOAD__"));
    assert!(script.contains("BACKEND_ERROR"));
    assert!(script.contains("first line"));
    assert_ne!(eval_hide_error_overlay(), "");
}

#[test]
fn backend_dead_escapes_message() {
    let call = eval_backend_dead(NASTY);
    let inner = call
        .strip_prefix("if(window.__tynd_backend_dead__)window.__tynd_backend_dead__(")
        .and_then(|s| s.strip_suffix(");"))
        .expect("shape");
    assert_eq!(serde_json::from_str::<String>(inner).unwrap(), NASTY);
}

#[test]
fn shim_constants_are_nonempty() {
    assert_ne!(JS_DEVALUE, "");
    assert_ne!(JS_SHIM, "");
    assert!(JS_DEV_FLAG.contains("__tynd_dev__"));
}

#[test]
fn frame_script_drives_only_window_frame_methods() {
    for attribute in [
        "data-tynd-drag",
        "data-tynd-no-drag",
        "data-tynd-resize",
        "data-tynd-drag-file",
    ] {
        assert!(JS_FRAME.contains(attribute), "{attribute}");
    }
    for method in [
        "\"frameState\"",
        "\"startDragging\"",
        "\"startResizeDragging\"",
        "\"toggleMaximize\"",
        "\"startDrag\"",
    ] {
        assert!(JS_FRAME.contains(method), "{method}");
    }
}

#[test]
fn shim_has_no_default_os_call_deadline_and_settles_cancelled_calls() {
    // Dialogs, downloads and process runs outlive any fixed deadline: only a
    // caller-provided `timeoutMs` may arm a timer.
    assert!(!JS_SHIM.contains("60000"));
    assert!(JS_SHIM.contains("opts.timeoutMs"));
    // A cancelled call must be settled locally with the CANCELLED envelope.
    assert!(JS_SHIM.contains("\"CANCELLED\""));
}

#[test]
fn shim_bridge_is_frozen_and_not_reassignable() {
    assert!(JS_SHIM.contains("Object.freeze(bridge)"));
    assert!(JS_SHIM.contains("configurable: false"));
    assert!(JS_SHIM.contains("writable: false"));
    assert!(!JS_SHIM.contains("window.__tynd__ = {"));
}

#[test]
fn ipc_shim_fills_the_token_and_app_origins() {
    let token = new_ipc_token();
    assert_eq!(token.len(), IPC_TOKEN_LEN);
    assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_ne!(token, new_ipc_token());

    let script = ipc_shim(&token, Some("http://localhost:5173"));
    assert!(!script.contains("__TYND_IPC_TOKEN__"));
    assert!(!script.contains("__TYND_APP_ORIGINS__"));
    assert!(script.contains(&format!("var _remoteToken = \"{token}\";")));
    let windows = if cfg!(target_os = "windows") {
        "\"http://tynd.localhost\","
    } else {
        ""
    };
    assert!(script.contains(&format!(
        "var _appOrigins = [{windows}\"http://localhost:5173\"];"
    )));
    assert!(
        ipc_shim(&token, None).contains(if cfg!(target_os = "windows") {
            "var _appOrigins = [\"http://tynd.localhost\"];"
        } else {
            "var _appOrigins = [];"
        })
    );
}

#[test]
fn shim_posts_only_through_the_token_sink_and_refuses_foreign_frames() {
    // Every message goes through `_post`, which prefixes the token.
    assert!(!JS_SHIM.contains("window.ipc.postMessage("));
    assert!(JS_SHIM.contains("_apply(_send, _sink, [_token + message])"));
    assert!(JS_SHIM.contains("__tynd_page_ready"));
    // The subframe check runs before anything is installed.
    let guard = JS_SHIM
        .find("if (!_isTop && !_isAppLocation(_loc)) return;")
        .unwrap();
    assert!(guard < JS_SHIM.find("var _remoteToken").unwrap());
    assert!(
        guard
            < JS_SHIM
                .find("Object.defineProperty(window, \"__tynd__\"")
                .unwrap()
    );
}

#[test]
fn ipc_token_is_required_and_stripped() {
    let token = "ab".repeat(IPC_TOKEN_LEN / 2);
    let mut body = format!("{token}{{\"type\":\"call\"}}");
    assert!(take_ipc_token(&mut body, &token));
    assert_eq!(body, "{\"type\":\"call\"}");

    for forged in [
        String::new(),
        "{\"type\":\"call\"}".to_owned(),
        token[..IPC_TOKEN_LEN - 1].to_owned(),
        format!("{}{{}}", "ac".repeat(IPC_TOKEN_LEN / 2)),
        format!("\u{e9}{}", &token[2..]),
    ] {
        let mut body = forged.clone();
        assert!(!take_ipc_token(&mut body, &token), "{forged}");
        assert_eq!(body, forged);
    }
}

#[test]
fn shim_never_bakes_in_the_app_token() {
    // Only one token placeholder: the remote token. An app document starts
    // without a token and asks the host for the app one.
    assert_eq!(JS_SHIM.matches(TOKEN_PLACEHOLDER).count(), 1);
    assert!(JS_SHIM.contains("var _token = _appMode ? null : _remoteToken;"));
    assert!(JS_SHIM.contains("_apply(_send, _sink, [_remoteToken + \"?\" + _appTokenId])"));
    assert!(JS_SHIM.contains(&format!("\"{}\"", tokens::APP_TOKEN_ID_PREFIX)));
    // The receiver cannot be replaced by a page script.
    let receiver = JS_SHIM
        .find("Object.defineProperty(window, \"__tynd_os_result__\"")
        .unwrap();
    assert!(JS_SHIM[receiver..].contains("writable: false"));
}

#[test]
fn app_token_answers_are_location_guarded() {
    let id = format!("{}abc", tokens::APP_TOKEN_ID_PREFIX);
    let script = eval_os_result(&id, true, &json!("secret"));
    assert!(
        script.starts_with("(function(l){if(l&&(l.protocol===\"tynd:\"||l.href===\"about:blank\"")
    );
    assert!(script.ends_with("})(window.location)"));
    assert!(!script.contains("indexOf"));
    assert!(script.contains("window.__tynd_os_result__&&window.__tynd_os_result__("));

    let guarded = app_location_guard("f()", Some("http://localhost:5173"));
    assert!(guarded.contains("||l.origin===\"http://localhost:5173\""));
    assert_eq!(
        guarded.contains("l.origin===\"http://tynd.localhost\""),
        cfg!(target_os = "windows")
    );

    // Ordinary OS results stay unguarded.
    let plain = eval_os_result("os_1", true, &json!(1));
    assert_eq!(
        parse_args(&plain, "__tynd_os_result__"),
        json!(["os_1", true, 1])
    );
}

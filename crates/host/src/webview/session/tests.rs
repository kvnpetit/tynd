use serde_json::json;

use super::*;

#[test]
fn session_methods_are_recognised() {
    for method in METHODS {
        assert!(is_session_method(method), "{method}");
    }
    // `window.print()` (dialog only) stays a plain window command.
    assert!(!is_session_method("print"));
    assert!(!is_session_method("clearWebviewData"));
}

#[test]
fn target_defaults_to_the_caller() {
    let args = json!({});
    assert_eq!(
        resolve_target("capturePage", &args, "main", None).unwrap(),
        "main"
    );
    let args = json!({ "label": null });
    assert_eq!(
        resolve_target("capturePage", &args, "main", Some("https://x.test")).unwrap(),
        "main"
    );
}

#[test]
fn only_app_content_targets_another_webview() {
    let args = json!({ "label": "preview" });
    assert_eq!(
        resolve_target("evaluateScript", &args, "main", None).unwrap(),
        "preview"
    );
    let error =
        resolve_target("evaluateScript", &args, "main", Some("https://x.test")).unwrap_err();
    assert!(error.contains("can only target itself"), "{error}");
    let own = json!({ "label": "main" });
    assert_eq!(
        resolve_target("evaluateScript", &own, "main", Some("https://x.test")).unwrap(),
        "main"
    );
    for bad in [json!({ "label": "" }), json!({ "label": 3 })] {
        let error = resolve_target("capturePage", &bad, "main", None).unwrap_err();
        assert_eq!(error, "capturePage: 'label' must be a non-empty string");
    }
}

#[test]
fn script_must_be_a_bounded_non_empty_string() {
    assert_eq!(
        parse_script(&json!({ "script": "document.title" })).unwrap(),
        "document.title"
    );
    for bad in [json!({}), json!({ "script": "  " }), json!({ "script": 1 })] {
        assert!(parse_script(&bad).is_err(), "{bad}");
    }
    let huge = "1".repeat(MAX_SCRIPT_BYTES + 1);
    let error = parse_script(&json!({ "script": huge })).unwrap_err();
    assert!(error.contains("exceeds"), "{error}");
}

#[test]
fn wrapper_inlines_the_expression_on_its_own_lines() {
    let wrapped = evaluation_wrapper("1 + 1 // trailing comment");
    assert!(
        wrapped.contains("(\n1 + 1 // trailing comment\n)"),
        "{wrapped}"
    );
    assert!(!wrapped.contains("eval("), "{wrapped}");
}

#[test]
fn decodes_string_wrapped_and_bare_envelopes() {
    let inner = json!({ "ok": true, "value": { "a": [1, 2] } }).to_string();
    let raw = serde_json::to_string(&inner).unwrap();
    assert_eq!(decode_evaluation(&raw).unwrap(), json!({ "a": [1, 2] }));
    assert_eq!(
        decode_evaluation(r#"{"ok":true,"value":"x"}"#).unwrap(),
        json!("x")
    );
    assert_eq!(decode_evaluation(r#"{"ok":true}"#).unwrap(), Value::Null);
}

#[test]
fn decodes_page_errors_and_missing_results() {
    let inner = json!({ "ok": false, "error": "boom is not defined" }).to_string();
    let raw = serde_json::to_string(&inner).unwrap();
    assert_eq!(
        decode_evaluation(&raw).unwrap_err(),
        "evaluateScript: boom is not defined"
    );
    for raw in ["null", "", "42"] {
        let error = decode_evaluation(raw).unwrap_err();
        assert!(error.contains("no result"), "{raw}: {error}");
    }
}

#[test]
fn pdf_options_default_and_map_to_devtools_parameters() {
    let options = PdfOptions::from_args(&json!({})).unwrap();
    assert_eq!(
        options.to_cdp(),
        json!({ "landscape": false, "printBackground": false, "scale": 1.0 })
    );

    let options = PdfOptions::from_args(&json!({ "options": {
        "landscape": true,
        "printBackground": true,
        "scale": 0.5,
        "pageSize": "A4",
        "margins": { "top": 1, "left": 0.5 },
    }}))
    .unwrap();
    assert_eq!(
        options.to_cdp(),
        json!({
            "landscape": true,
            "printBackground": true,
            "scale": 0.5,
            "paperWidth": 8.27,
            "paperHeight": 11.69,
            "marginTop": 1.0,
            "marginRight": 0.0,
            "marginBottom": 0.0,
            "marginLeft": 0.5,
        })
    );

    let options = PdfOptions::from_args(&json!({ "options": {
        "pageSize": { "width": 4, "height": 6 },
        "margins": 0.25,
    }}))
    .unwrap();
    assert_eq!(options.page_size, Some((4.0, 6.0)));
    assert_eq!(options.margins, Some([0.25; 4]));
}

#[test]
fn pdf_options_reject_out_of_range_and_malformed_values() {
    let cases = [
        (json!("A4"), "options must be an object"),
        (
            json!({ "landscape": "yes" }),
            "'landscape' must be a boolean",
        ),
        (json!({ "scale": 3 }), "'scale' must be a number between"),
        (json!({ "pageSize": "B5" }), "unknown pageSize 'B5'"),
        (
            json!({ "pageSize": { "width": 0.5, "height": 6 } }),
            "pageSize.width",
        ),
        (
            json!({ "pageSize": { "width": 4 } }),
            "pageSize.height is required",
        ),
        (json!({ "pageSize": 4 }), "pageSize must be"),
        (json!({ "margins": -1 }), "'margins' must be"),
        (json!({ "margins": { "top": 11 } }), "margins.top"),
        (json!({ "margins": "1in" }), "margins must be"),
    ];
    for (options, expected) in cases {
        let error = PdfOptions::from_args(&json!({ "options": options })).unwrap_err();
        assert!(error.contains(expected), "{options}: {error}");
    }
}

#[test]
fn devtools_data_is_extracted() {
    assert_eq!(
        cdp_data("capturePage", r#"{"data":"iVBORw0"}"#).unwrap(),
        json!("iVBORw0")
    );
    assert!(cdp_data("capturePage", "{}").is_err());
    assert!(cdp_data("capturePage", "not json").is_err());
}

#[test]
fn clear_scope_defaults_to_everything() {
    assert_eq!(
        ClearScope::from_args(&json!({})).unwrap(),
        ClearScope::Everything
    );
    assert_eq!(
        ClearScope::from_args(&json!({ "all": true, "cookies": true })).unwrap(),
        ClearScope::Everything
    );
    assert_eq!(
        ClearScope::from_args(&json!({ "cookies": false, "label": "main" })).unwrap(),
        ClearScope::Everything
    );
}

#[test]
fn clear_scope_selects_categories() {
    let scope = ClearScope::from_args(&json!({
        "cookies": true,
        "indexedDb": true,
        "cache": false,
    }))
    .unwrap();
    let ClearScope::Only(kinds) = scope else {
        panic!("expected a selection, got {scope:?}");
    };
    assert!(kinds.has(DataKinds::COOKIES));
    assert!(kinds.has(DataKinds::INDEXED_DB));
    assert!(!kinds.has(DataKinds::CACHE));
    assert!(!kinds.has(DataKinds::LOCAL_STORAGE));
    assert!(!kinds.has(DataKinds::SERVICE_WORKERS));
    for &(key, kind) in DataKinds::KEYS {
        let scope = ClearScope::from_args(&json!({ key: true, "all": false })).unwrap();
        assert_eq!(scope, ClearScope::Only(DataKinds(kind)), "{key}");
    }
}

#[test]
fn clear_scope_rejects_malformed_flags_and_empty_selection() {
    let error = ClearScope::from_args(&json!({ "cookies": "yes" })).unwrap_err();
    assert_eq!(error, "clearBrowsingData: 'cookies' must be a boolean");
    let error = ClearScope::from_args(&json!({ "all": 1 })).unwrap_err();
    assert_eq!(error, "clearBrowsingData: 'all' must be a boolean");
    let error = ClearScope::from_args(&json!({ "all": false })).unwrap_err();
    assert!(error.contains("nothing to clear"), "{error}");
}

#[test]
fn print_request_defaults_to_the_dialog() {
    let request = PrintRequest::from_args(&json!({}), Some("https://x.test")).unwrap();
    assert_eq!(
        request,
        PrintRequest {
            silent: false,
            printer: None
        }
    );
    let request =
        PrintRequest::from_args(&json!({ "silent": true, "printer": "Office" }), None).unwrap();
    assert_eq!(
        request,
        PrintRequest {
            silent: true,
            printer: Some("Office".into())
        }
    );
}

#[test]
fn print_request_validates_printer_and_origin() {
    let long = "p".repeat(MAX_PRINTER_NAME + 1);
    let cases = [
        (json!({ "silent": "yes" }), "'silent' must be a boolean"),
        (
            json!({ "silent": true, "printer": "" }),
            "'printer' must be",
        ),
        (json!({ "silent": true, "printer": 3 }), "'printer' must be"),
        (
            json!({ "silent": true, "printer": "a
b" }),
            "'printer' must be",
        ),
        (
            json!({ "silent": true, "printer": long }),
            "'printer' must be",
        ),
        (json!({ "printer": "Office" }), "needs silent: true"),
    ];
    for (args, expected) in cases {
        let error = PrintRequest::from_args(&args, None).unwrap_err();
        assert!(error.contains(expected), "{args}: {error}");
    }
    let error =
        PrintRequest::from_args(&json!({ "silent": true }), Some("https://x.test")).unwrap_err();
    assert!(error.contains("only available to app content"), "{error}");
}

fn recording_reply(method: &str) -> (Reply, std::sync::mpsc::Receiver<Result<Value, String>>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let reply = Reply::from_fn(
        method,
        Box::new(move |result| {
            tx.send(result).expect("test receiver alive");
        }),
    );
    (reply, rx)
}

#[test]
fn a_dropped_reply_answers_with_an_error() {
    let (reply, rx) = recording_reply("capturePage");
    drop(reply);
    assert_eq!(
        rx.try_recv().expect("drop must answer"),
        Err("capturePage: webview went away before answering".to_owned())
    );
}

#[test]
fn a_sent_reply_answers_exactly_once() {
    let (reply, rx) = recording_reply("printToPdf");
    reply.send(Ok(json!("done")));
    assert_eq!(rx.try_recv().expect("send answers"), Ok(json!("done")));
    assert!(
        rx.try_recv().is_err(),
        "drop after send must not answer again"
    );
}

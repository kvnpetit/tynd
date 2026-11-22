use serde_json::json;

use super::*;

#[test]
fn rejected_calls_settle_the_backend_call_map() {
    // OsResult would settle the OS-call map and leave a backend call pending.
    let event = call_rejection_event("7", "reserved function name");
    assert!(matches!(
        event,
        UserEvent::Backend(BackendEvent::Return {
            ref id,
            payload: BackendReturnPayload::Err(ref message),
        }) if id == "7" && message == "reserved function name"
    ));
}

#[test]
fn ctx_is_injected_with_server_side_identity() {
    let mut envelope = json!({ "type": "call", "id": "1", "fn": "greet", "args": [] });
    inject_call_context(&mut envelope, "main", "panel", Some("https://example.com"));
    assert_eq!(
        envelope["ctx"],
        json!({ "window": "main", "webview": "panel", "origin": "https://example.com" })
    );
    assert_eq!(envelope["fn"], "greet");
}

#[test]
fn ctx_uses_null_origin_for_the_bundled_app() {
    let mut envelope = json!({ "type": "call", "id": "1", "fn": "greet" });
    inject_call_context(&mut envelope, "main", "main", None);
    assert_eq!(envelope["ctx"]["origin"], Value::Null);
}

#[test]
fn client_supplied_ctx_is_overwritten() {
    let mut envelope = json!({
        "type": "call", "id": "1", "fn": "greet",
        "ctx": { "window": "admin", "webview": "admin", "origin": "https://evil.test", "extra": 1 },
    });
    inject_call_context(&mut envelope, "main", "main", None);
    assert_eq!(
        envelope["ctx"],
        json!({ "window": "main", "webview": "main", "origin": null })
    );
}

#[test]
fn renderer_cannot_choose_reserved_call_ids() {
    assert!(valid_call_id("7"));
    assert!(valid_call_id("3f2b9c1e-0000-4000-8000-000000000000"));
    assert!(!valid_call_id(""));
    assert!(!valid_call_id(&"x".repeat(129)));
    assert!(!valid_call_id("__tynd_on_close__"));
    assert!(!valid_call_id("__tynd_fe_emit__"));
    assert!(!valid_call_id("__tynd_"));
    // Only a leading prefix is reserved.
    assert!(valid_call_id("my__tynd_x"));
}

#[test]
fn renderer_cannot_call_reserved_function_names() {
    assert!(is_forbidden_reserved_fn("__tynd_on_close__"));
    assert!(is_forbidden_reserved_fn("__tynd_plugin_methods__"));
    assert!(!is_forbidden_reserved_fn("__tynd_fe_emit__"));
    assert!(!is_forbidden_reserved_fn("greet"));
}

#[test]
fn parsed_envelope_round_trips_with_injected_ctx() {
    let body = r#"{"type":"call","id":"7","fn":"f","args":[1],"ctx":{"window":"spoof"}}"#;
    let mut value = parse_ipc_body(body).expect("valid envelope");
    inject_call_context(&mut value, "w", "v", None);
    let forwarded: Value = serde_json::from_str(&serialize_ipc_envelope(&value).unwrap()).unwrap();
    assert_eq!(forwarded["ctx"]["window"], "w");
    assert_eq!(forwarded["args"], json!([1]));
}

mod acl {
    use tynd_capabilities::{Capability, CapabilityFile, Resolved, Target};

    use super::super::*;

    fn capability(windows: &[&str], allow: &[&str], deny: &[&str]) -> Vec<Capability> {
        serde_json::from_value::<CapabilityFile>(serde_json::json!([{
            "identifier": "cap",
            "windows": windows,
            "permissions": [{
                "identifier": "p",
                "commands": { "allow": allow, "deny": deny },
            }],
        }]))
        .expect("capability file")
        .into_capabilities()
    }

    fn resolved(caps: &[Capability]) -> Resolved {
        Resolved::resolve(
            &std::collections::BTreeMap::new(),
            caps.iter()
                .map(|c| (c.identifier.clone(), c.clone()))
                .collect(),
            Target::current(),
        )
        .expect("resolve")
    }

    #[test]
    fn reserved_internal_functions_skip_the_backend_acl() {
        assert!(is_reserved_backend_fn("__tynd_fe_emit__"));
        assert!(!is_reserved_backend_fn("__tynd_plugin_methods__"));
        assert!(!is_reserved_backend_fn("__tynd_admin__"));
        assert!(!is_reserved_backend_fn("greet"));
        assert!(!is_reserved_backend_fn("__tynd_"));
        assert!(!is_reserved_backend_fn("__tynd_fe_emit"));
        assert!(!is_reserved_backend_fn("my__tynd_x__"));
    }

    #[test]
    fn frontend_emit_survives_declared_backend_permissions() {
        let policy = capability(&["main"], &["backend:greet"], &[]);
        let resolved = resolved(&policy);
        // Mirrors denied_backend_command: reserved names never reach the policy.
        let allowed = |name: &str| {
            is_reserved_backend_fn(name)
                || backend_access_allowed(false, true, false, || {
                    resolved.is_allowed(&backend_command(name), Some("main"), Some("main"), None)
                })
        };
        assert!(allowed("__tynd_fe_emit__"));
        assert!(allowed("greet"));
        assert!(!allowed("secret"));
    }

    #[test]
    fn reserved_calls_keep_fail_closed_and_the_remote_origin_rule() {
        // App-local caller: no grant needed, the policy is not consulted.
        assert!(reserved_access_allowed(false, false, || panic!(
            "policy must not be consulted"
        )));
        // Fail-closed authority denies every caller.
        assert!(!reserved_access_allowed(true, false, || true));
        assert!(!reserved_access_allowed(true, true, || true));
        // Remote origin needs an explicit grant.
        assert!(!reserved_access_allowed(false, true, || false));
        assert!(reserved_access_allowed(false, true, || true));
    }

    #[test]
    fn user_exports_named_like_reserved_names_stay_gated() {
        let policy = capability(&["main"], &["backend:greet"], &[]);
        let resolved = resolved(&policy);
        let allowed = |name: &str| {
            if is_reserved_backend_fn(name) {
                return true;
            }
            backend_access_allowed(false, true, false, || {
                resolved.is_allowed(&backend_command(name), Some("main"), Some("main"), None)
            })
        };
        assert!(!allowed("__tynd_admin__"));
    }

    #[test]
    fn backend_command_name() {
        assert_eq!(backend_command("greet"), "backend:greet");
    }

    #[test]
    fn remote_origin_needs_a_grant_even_when_nothing_is_declared() {
        // Backend: no `backend:` declaration, remote caller, no grant.
        assert!(!backend_access_allowed(false, false, true, || false));
        assert!(backend_access_allowed(false, false, true, || true));
        // Native namespaces behave the same.
        for (api, method) in [
            ("window", "create"),
            ("window", "setAlwaysOnTop"),
            ("tray", "setIcon"),
            ("menu", "setItem"),
        ] {
            assert!(!native_access_allowed(
                api,
                method,
                false,
                false,
                true,
                || false
            ));
            assert!(native_access_allowed(
                api,
                method,
                false,
                false,
                true,
                || true
            ));
        }
        // The bundled app keeps the default-allow shortcut.
        assert!(backend_access_allowed(false, false, false, || panic!(
            "policy must not be consulted"
        )));
        assert!(native_access_allowed(
            "window",
            "create",
            false,
            false,
            false,
            || panic!("policy must not be consulted")
        ));
        // A fail-closed authority still wins over a grant.
        assert!(!backend_access_allowed(true, false, true, || true));
        assert!(!native_access_allowed(
            "window",
            "create",
            true,
            false,
            true,
            || true
        ));
    }

    #[test]
    fn plugin_wire_names_that_do_not_parse_are_not_plugin_commands() {
        assert!(plugin_command_from_wire("$plugin$vault$read").is_some());
        assert!(plugin_command_from_wire("$plugin$vault").is_none());
        assert!(plugin_command_from_wire("$plugin$$read").is_none());
        assert!(plugin_command_from_wire("$plugin$vault$a$b").is_none());
    }

    #[test]
    fn no_backend_declaration_allows_everything() {
        assert!(backend_access_allowed(false, false, false, || panic!(
            "policy must not be consulted"
        )));
    }

    #[test]
    fn fail_closed_denies_backend_calls_even_without_declarations() {
        assert!(!backend_access_allowed(true, false, false, || panic!(
            "policy must not be consulted"
        )));
        assert!(!backend_access_allowed(true, true, false, || true));
    }

    #[test]
    fn once_declared_undeclared_functions_are_denied_per_window() {
        let policy = capability(&["main"], &["backend:greet"], &[]);
        let resolved = resolved(&policy);
        let check = |name: &str, window: &str| {
            backend_access_allowed(false, true, false, || {
                resolved.is_allowed(&backend_command(name), Some(window), Some(window), None)
            })
        };
        assert!(check("greet", "main"));
        assert!(!check("greet", "other"));
        assert!(!check("secret", "main"));
    }

    #[test]
    fn window_commands_are_open_until_the_namespace_is_declared() {
        assert!(native_access_allowed(
            "window",
            "setTitle",
            false,
            false,
            false,
            || panic!("policy must not be consulted")
        ));
        assert!(native_access_allowed(
            "tray",
            "setIcon",
            false,
            false,
            false,
            || panic!("policy must not be consulted")
        ));
    }

    #[test]
    fn window_command_is_denied_by_a_restrictive_capability() {
        let policy = capability(&["main"], &["window:setTitle"], &["window:close"]);
        let resolved = resolved(&policy);
        let check = |method: &str, window: &str| {
            native_access_allowed("window", method, false, true, false, || {
                resolved.is_allowed(
                    &format!("window:{method}"),
                    Some(window),
                    Some(window),
                    None,
                )
            })
        };
        assert!(check("setTitle", "main"));
        assert!(!check("close", "main"));
        assert!(!check("minimize", "main"));
        assert!(!check("setTitle", "other"));
    }

    #[test]
    fn window_wildcard_grant_keeps_window_commands_working() {
        let policy = capability(&["main"], &["window:*"], &[]);
        let resolved = resolved(&policy);
        assert!(native_access_allowed(
            "window",
            "minimize",
            false,
            true,
            false,
            || { resolved.is_allowed("window:minimize", Some("main"), Some("main"), None) }
        ));
    }

    #[test]
    fn cookie_commands_always_need_an_explicit_grant() {
        assert!(!native_access_allowed(
            "window",
            "getWebviewCookies",
            false,
            false,
            false,
            || false
        ));
        assert!(native_access_allowed(
            "window",
            "getWebviewCookies",
            false,
            false,
            false,
            || true
        ));
        // The cookie list is specific to the window API.
        assert!(native_access_allowed(
            "tray",
            "getWebviewCookies",
            false,
            false,
            false,
            || false
        ));
    }

    #[test]
    fn fail_closed_denies_native_commands() {
        assert!(!native_access_allowed(
            "window",
            "setTitle",
            true,
            false,
            false,
            || true
        ));
        assert!(!native_access_allowed(
            "menu",
            "setItem",
            true,
            false,
            false,
            || true
        ));
    }

    #[test]
    fn cookie_grant_scoped_to_a_webview_matches_the_calling_webview() {
        let caps = serde_json::from_value::<CapabilityFile>(serde_json::json!([{
            "identifier": "cookies",
            "webviews": ["panel"],
            "permissions": [{
                "identifier": "p",
                "commands": { "allow": ["window:getWebviewCookies"] },
            }],
        }]))
        .expect("capability file")
        .into_capabilities();
        let resolved = resolved(&caps);
        assert!(resolved.is_allowed(
            "window:getWebviewCookies",
            Some("main"),
            Some("panel"),
            None
        ));
        assert!(!resolved.is_allowed("window:getWebviewCookies", Some("main"), None, None));
    }

    #[test]
    fn wildcard_allow_and_explicit_deny() {
        let policy = capability(&["main"], &["backend:*"], &["backend:danger"]);
        let resolved = resolved(&policy);
        assert!(resolved.is_allowed("backend:anything", Some("main"), Some("main"), None));
        assert!(!resolved.is_allowed("backend:danger", Some("main"), Some("main"), None));
    }

    #[test]
    fn remote_origin_is_denied_without_remote_grant() {
        let policy = capability(&["main"], &["backend:greet"], &[]);
        let resolved = resolved(&policy);
        assert!(!resolved.is_allowed(
            "backend:greet",
            Some("main"),
            Some("main"),
            Some("https://evil.test/")
        ));
    }
}

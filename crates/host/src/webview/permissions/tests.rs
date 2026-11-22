use serde_json::json;

use super::*;

#[test]
fn unset_kinds_prompt_for_app_content_and_deny_remote_content() {
    let policy = PermissionPolicy::default();
    assert_eq!(
        policy.decide(PermissionKind::Camera, true),
        PermissionDecision::Prompt
    );
    assert_eq!(
        policy.decide(PermissionKind::Camera, false),
        PermissionDecision::Deny
    );
    assert_eq!(
        policy.decide(PermissionKind::Other, false),
        PermissionDecision::Deny
    );
}

#[test]
fn configured_kinds_apply_to_every_content() {
    let raw = json!({ "camera": "allow", "geolocation": "deny", "microphone": "prompt" });
    let policy = PermissionPolicy::from_value(Some(&raw), "create").unwrap();
    assert_eq!(
        policy.decide(PermissionKind::Camera, false),
        PermissionDecision::Allow
    );
    assert_eq!(
        policy.decide(PermissionKind::Geolocation, true),
        PermissionDecision::Deny
    );
    assert_eq!(
        policy.decide(PermissionKind::Microphone, false),
        PermissionDecision::Prompt
    );
    assert_eq!(
        policy.decide(PermissionKind::Notifications, true),
        PermissionDecision::Prompt
    );
}

#[test]
fn rejects_unknown_kinds_values_and_shapes() {
    let error =
        PermissionPolicy::from_value(Some(&json!({ "webcam": "allow" })), "create").unwrap_err();
    assert!(
        error.starts_with("create: unknown permission 'webcam'"),
        "{error}"
    );
    let error =
        PermissionPolicy::from_value(Some(&json!({ "camera": "yes" })), "create").unwrap_err();
    assert!(error.contains("permissions.camera must be"), "{error}");
    let error =
        PermissionPolicy::from_value(Some(&json!({ "camera": true })), "create").unwrap_err();
    assert!(error.contains("permissions.camera must be"), "{error}");
    let error = PermissionPolicy::from_value(Some(&json!("allow")), "create").unwrap_err();
    assert_eq!(error, "create: permissions must be an object");
}

#[test]
fn absent_or_null_is_the_default_policy() {
    assert_eq!(
        PermissionPolicy::from_value(None, "create").unwrap(),
        PermissionPolicy::default()
    );
    assert_eq!(
        PermissionPolicy::from_value(Some(&Value::Null), "create").unwrap(),
        PermissionPolicy::default()
    );
    assert_eq!(
        PermissionPolicy::from_map(None, "window").unwrap(),
        PermissionPolicy::default()
    );
}

#[test]
fn primary_config_map_parses_like_create_arguments() {
    let map = BTreeMap::from([("clipboardRead".to_owned(), "allow".to_owned())]);
    let policy = PermissionPolicy::from_map(Some(&map), "window").unwrap();
    assert_eq!(
        policy.decide(PermissionKind::ClipboardRead, false),
        PermissionDecision::Allow
    );
    let bad = BTreeMap::from([("camera".to_owned(), "always".to_owned())]);
    let error = PermissionPolicy::from_map(Some(&bad), "window").unwrap_err();
    assert!(error.starts_with("window: permissions.camera"), "{error}");
}

#[test]
fn decisions_map_to_wry_responses() {
    assert_eq!(
        PermissionDecision::Allow.to_wry(),
        PermissionResponse::Allow
    );
    assert_eq!(PermissionDecision::Deny.to_wry(), PermissionResponse::Deny);
    assert_eq!(
        PermissionDecision::Prompt.to_wry(),
        PermissionResponse::Default
    );
}

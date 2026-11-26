use serde_json::{json, Value};

use super::*;

const KEY_A: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const KEY_B: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBE=";

fn parse(value: Value) -> TyndConfig {
    serde_json::from_value(value).expect("config shape should deserialize")
}

#[test]
fn single_entry_lists_resolve() {
    let config = parse(json!({ "updater": {
        "endpoints": ["https://example.com/m.json"], "publicKeys": [KEY_A]
    }}));
    let resolved = resolve_updater_config(&config).unwrap().unwrap();
    assert_eq!(resolved.endpoints, vec!["https://example.com/m.json"]);
    assert_eq!(resolved.public_keys, vec![KEY_A]);
    assert_eq!(resolved.channel, DEFAULT_UPDATER_CHANNEL);
}

#[test]
fn lists_keep_their_order() {
    let config = parse(json!({ "updater": {
        "endpoints": [
            "https://mirror1.example.com/m.json",
            "https://mirror2.example.com/m.json"
        ],
        "publicKeys": [KEY_B, KEY_A],
        "channel": "beta"
    }}));
    let resolved = resolve_updater_config(&config).unwrap().unwrap();
    assert_eq!(
        resolved.endpoints,
        vec![
            "https://mirror1.example.com/m.json",
            "https://mirror2.example.com/m.json"
        ]
    );
    assert_eq!(resolved.public_keys, vec![KEY_B, KEY_A]);
    assert_eq!(resolved.channel, "beta");
    let wire = serde_json::to_value(&resolved).unwrap();
    assert_eq!(wire["publicKeys"], json!([KEY_B, KEY_A]));
    assert_eq!(wire["checkIntervalSecs"], 43_200);
}

#[test]
fn missing_endpoint_or_key_is_an_error_not_a_panic() {
    let config = parse(json!({ "updater": { "publicKeys": [KEY_A] } }));
    assert!(resolve_updater_config(&config).is_err());
    let config = parse(json!({ "updater": { "endpoints": ["https://e.example.com"] } }));
    assert!(resolve_updater_config(&config).is_err());
    assert!(resolve_updater_config(&parse(json!({}))).unwrap().is_none());
}

#[test]
fn pubkey_pin_joins_every_key_with_commas() {
    assert_eq!(updater_pubkey_pin(&parse(json!({}))), None);
    let config = parse(json!({ "updater": {
        "endpoints": ["https://e.example.com"], "publicKeys": [KEY_B, KEY_A]
    }}));
    assert_eq!(
        updater_pubkey_pin(&config),
        Some(format!("{KEY_B},{KEY_A}"))
    );
}

#[test]
fn observability_is_absent_without_an_active_pipeline() {
    assert!(resolve_observability(&parse(json!({}))).is_none());
    let blank = parse(json!({ "observability": {
        "sentry": { "dsn": " " }, "otel": { "endpoint": "" }
    }}));
    assert!(resolve_observability(&blank).is_none());
    assert_eq!(resolve_observability_json(&blank).unwrap(), None);
}

#[test]
fn observability_json_matches_the_runtime_contract() {
    let expected = json!({
        "sentry": {
            "dsn": "https://k@o.example.com/1",
            "environment": "prod",
            "sampleRate": 0.5
        },
        "otel": { "endpoint": "http://localhost:4317" }
    });
    let config = parse(json!({ "observability": expected.clone() }));
    let json = resolve_observability_json(&config).unwrap().unwrap();
    assert!(json.len() <= MAX_OBSERVABILITY_BYTES);
    assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), expected);
}

#[test]
fn disabled_pipelines_are_pruned_from_the_json() {
    let config = parse(json!({ "observability": {
        "sentry": { "dsn": "" },
        "otel": { "endpoint": "http://localhost:4317", "serviceName": "app" }
    }}));
    let value: Value =
        serde_json::from_str(&resolve_observability_json(&config).unwrap().unwrap()).unwrap();
    assert_eq!(
        value,
        json!({ "otel": { "endpoint": "http://localhost:4317", "serviceName": "app" } })
    );
}

#[test]
fn requested_apis_join_the_core_apis_in_the_host_plan() {
    let cwd = std::env::temp_dir();
    let plan = resolve_effective_host_plan(
        &parse(json!({ "apis": ["menu", "tray", "multiwindow"] })),
        &cwd,
    )
    .unwrap();
    for api in ["menu", "tray", "multiwindow"] {
        assert!(!CORE_HOST_APIS.contains(&api));
        assert!(plan.apis.iter().any(|candidate| candidate == api));
    }
    for api in CORE_HOST_APIS {
        assert!(plan.apis.iter().any(|candidate| candidate == api));
    }
}

#[test]
fn empty_config_host_plan_holds_only_the_core_apis() {
    let plan = resolve_effective_host_plan(&parse(json!({})), &std::env::temp_dir()).unwrap();
    assert_eq!(plan.apis, ["acl", "log"]);
}

fn network_config(extra: Value) -> TyndConfig {
    let mut updater =
        json!({ "endpoints": ["https://updates.example.com/m.json"], "publicKeys": [KEY_A] });
    for (key, value) in extra.as_object().expect("object") {
        updater[key] = value.clone();
    }
    parse(json!({ "updater": updater }))
}

#[test]
fn network_is_absent_without_proxy_or_headers() {
    let config = network_config(json!({}));
    assert!(resolve_updater_network(&config, |_| None)
        .unwrap()
        .is_none());
    assert!(resolve_updater_network(&parse(json!({})), |_| None)
        .unwrap()
        .is_none());
}

#[test]
fn network_expands_env_placeholders_in_header_values() {
    let config = network_config(json!({
        "proxy": " http://proxy.corp:3128 ",
        "headers": {
            "Authorization": "Bearer ${env:UPDATE_TOKEN}",
            "X-Pair": "${env:A}:${env:B}",
            "X-Literal": "${other} $env"
        }
    }));
    let network = resolve_updater_network(&config, |name| match name {
        "UPDATE_TOKEN" => Some("s3cr\"et".to_owned()),
        "A" => Some("a".to_owned()),
        "B" => Some("b".to_owned()),
        _ => None,
    })
    .unwrap()
    .unwrap();
    assert_eq!(network.proxy.as_deref(), Some("http://proxy.corp:3128"));
    assert_eq!(network.headers["Authorization"], "Bearer s3cr\"et");
    assert_eq!(network.headers["X-Pair"], "a:b");
    assert_eq!(network.headers["X-Literal"], "${other} $env");
    let wire = serde_json::to_value(&network).unwrap();
    assert_eq!(wire["headers"]["Authorization"], "Bearer s3cr\"et");
    assert_eq!(wire["proxy"], "http://proxy.corp:3128");
    // Debug never prints a credential.
    assert!(!format!("{network:?}").contains("s3cr"));
}

#[test]
fn network_reports_an_unset_variable_by_name_only() {
    let config = network_config(json!({ "headers": { "Authorization": "${env:MISSING_TOKEN}" } }));
    let error = resolve_updater_network(&config, |_| None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("MISSING_TOKEN"), "{error}");
    assert!(error.contains("updater.headers.Authorization"), "{error}");
}

#[test]
fn network_rejects_an_expanded_value_that_breaks_the_header() {
    let config = network_config(json!({ "headers": { "X-A": "${env:V}" } }));
    let error = resolve_updater_network(&config, |_| Some("a\nb".to_owned()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("control character"), "{error}");
    assert!(!error.contains("a\nb"), "{error}");
}

#[test]
fn proxy_and_headers_stay_out_of_the_public_updater_config() {
    let config = network_config(json!({
        "proxy": "http://proxy.corp:3128",
        "headers": { "Authorization": "t" }
    }));
    let wire = serde_json::to_value(resolve_updater_config(&config).unwrap().unwrap()).unwrap();
    assert!(
        wire.get("proxy").is_none() && wire.get("headers").is_none(),
        "{wire}"
    );
}

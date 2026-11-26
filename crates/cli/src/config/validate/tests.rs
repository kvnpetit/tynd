use serde_json::{json, Value};

use super::*;

const KEY_A: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const KEY_B: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBE=";

fn parse(value: Value) -> TyndConfig {
    serde_json::from_value(value).expect("config shape should deserialize")
}

fn issues(value: Value) -> Vec<String> {
    match validate_config_at(&parse(value), None) {
        Ok(()) => Vec::new(),
        Err(error) => error
            .downcast_ref::<ConfigError>()
            .map(|error| error.issues.clone())
            .expect("validation failures are ConfigError"),
    }
}

fn has_issue(issues: &[String], needle: &str) -> bool {
    issues.iter().any(|issue| issue.contains(needle))
}

#[test]
fn empty_config_is_valid() {
    assert_eq!(issues(json!({})), [] as [String; 0]);
}

#[test]
fn new_ui_apis_are_known() {
    assert_eq!(
        issues(json!({ "apis": ["menu", "tray", "multiwindow"] })),
        [] as [String; 0]
    );
    assert!(has_issue(
        &issues(json!({ "apis": ["debug-screenshot"] })),
        "unknown API"
    ));
}

#[test]
fn updater_accepts_lists() {
    let updater = json!({
        "endpoints": [
            "https://a.example.com/{{channel}}/{{target}}-{{arch}}/{{current_version}}.json",
            "http://127.0.0.1:8080/m.json"
        ],
        "publicKeys": [KEY_A, KEY_B],
        "channel": "beta-1.x_y"
    });
    assert_eq!(issues(json!({ "updater": updater })), [] as [String; 0]);
}

#[test]
fn updater_needs_an_endpoint_and_a_key() {
    let found = issues(json!({ "updater": {} }));
    assert!(has_issue(&found, "needs a non-empty endpoints list"));
    assert!(has_issue(&found, "needs a non-empty publicKeys list"));
    let found = issues(json!({ "updater": { "endpoints": [], "publicKeys": [] } }));
    assert!(has_issue(&found, "needs a non-empty endpoints list"));
}

#[test]
fn updater_rejects_bad_endpoints() {
    let found = issues(json!({ "updater": {
        "endpoints": [
            "ftp://x.example.com/m.json",
            "https://a.example.com/{{nope}}.json",
            "https://dup.example.com/m.json",
            "https://dup.example.com/m.json",
            ""
        ],
        "publicKeys": [KEY_A]
    }}));
    assert!(has_issue(
        &found,
        "updater.endpoints.0 must be an http(s) URL"
    ));
    assert!(has_issue(
        &found,
        "updater.endpoints.1 contains an unknown placeholder"
    ));
    assert!(has_issue(&found, "contains duplicate"));
    assert!(has_issue(
        &found,
        "updater.endpoints.4 must be an http(s) URL"
    ));
}

#[test]
fn updater_rejects_bad_keys() {
    let found = issues(json!({ "updater": {
        "endpoints": ["https://example.com/m.json"],
        "publicKeys": [KEY_A, "not base64!", KEY_A]
    }}));
    assert!(has_issue(&found, "updater.publicKeys.1 must be"));
    assert!(has_issue(&found, "updater.publicKeys.2 is duplicated"));
}

#[test]
fn updater_channel_follows_the_host_rules() {
    let check = |channel: &str| {
        issues(json!({ "updater": {
            "endpoints": ["https://example.com/m.json"], "publicKeys": [KEY_A], "channel": channel
        }}))
    };
    for good in ["stable", "beta", "a.b-c_d", &"x".repeat(64)] {
        assert!(check(good).is_empty(), "{good} should be accepted");
    }
    for bad in ["", ".hidden", "has space", "slash/es", &"x".repeat(65)] {
        assert!(
            has_issue(&check(bad), "updater.channel"),
            "{bad:?} should be rejected"
        );
    }
}

#[test]
fn observability_accepts_both_pipelines_and_blanks() {
    assert_eq!(
        issues(json!({ "observability": {
            "sentry": {
                "dsn": "https://key@o1.ingest.example.com/42",
                "environment": "prod",
                "release": "1.2.3",
                "sampleRate": 0.25
            },
            "otel": { "endpoint": "http://localhost:4317", "serviceName": "my-app" }
        }})),
        [] as [String; 0]
    );
    assert_eq!(
        issues(json!({ "observability": {
            "sentry": { "dsn": "" }, "otel": { "endpoint": "  " }
        }})),
        [] as [String; 0]
    );
    assert_eq!(issues(json!({ "observability": {} })), [] as [String; 0]);
}

#[test]
fn observability_rejects_bad_values() {
    let found = issues(json!({ "observability": {
        "sentry": { "dsn": "not a url", "environment": "", "release": "", "sampleRate": 1.5 },
        "otel": { "endpoint": "grpc://x", "serviceName": "" }
    }}));
    assert!(has_issue(&found, "observability.sentry.dsn"));
    assert!(has_issue(&found, "observability.sentry.environment"));
    assert!(has_issue(&found, "observability.sentry.release"));
    assert!(has_issue(&found, "observability.sentry.sampleRate"));
    assert!(has_issue(&found, "observability.otel.endpoint"));
    assert!(has_issue(&found, "observability.otel.serviceName"));
    assert!(has_issue(
        &issues(json!({ "observability": { "sentry": { "dsn": "", "sampleRate": -0.1 } } })),
        "sampleRate"
    ));
}

#[test]
fn observability_is_size_capped() {
    let found = issues(json!({ "observability": {
        "sentry": { "dsn": "", "release": "r".repeat(5000) }
    }}));
    assert!(has_issue(&found, "larger than 4096 bytes"));
}

#[test]
fn removed_top_level_and_bundle_keys_are_unknown_fields() {
    for value in [
        json!({ "runtime": "bun" }),
        json!({ "compat": {} }),
        json!({ "frontendEntry": "frontend/main.ts" }),
        json!({ "bundle": { "identifier": "com.example.app", "nsis": {} } }),
        json!({ "bundle": { "identifier": "com.example.app", "msi": {} } }),
    ] {
        let error = serde_json::from_value::<TyndConfig>(value)
            .expect_err("removed keys must fail deserialization");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }
}

#[test]
fn singular_updater_keys_are_unknown_fields() {
    for updater in [
        json!({ "endpoint": "https://e.example.com", "publicKeys": [KEY_A] }),
        json!({ "endpoints": ["https://e.example.com"], "publicKey": KEY_A }),
    ] {
        let error = serde_json::from_value::<TyndConfig>(json!({ "updater": updater }))
            .expect_err("singular updater keys must fail deserialization");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }
}

#[test]
fn unknown_fields_stay_rejected() {
    let rejected = serde_json::from_value::<TyndConfig>(json!({
        "observability": { "sentry": { "dsn": "", "tracesSampleRate": 1 } }
    }));
    assert!(rejected.is_err());
    let rejected = serde_json::from_value::<TyndConfig>(json!({
        "updater": { "endpoints": ["https://e.example.com"], "publicKeys": [KEY_A], "channels": [] }
    }));
    assert!(rejected.is_err());
}

fn net_updater(extra: Value) -> Value {
    let mut updater =
        json!({ "endpoints": ["https://updates.example.com/m.json"], "publicKeys": [KEY_A] });
    for (key, value) in extra.as_object().expect("object") {
        updater[key] = value.clone();
    }
    json!({ "updater": updater })
}

#[test]
fn updater_accepts_proxy_and_headers() {
    let found = issues(net_updater(json!({
        "proxy": "http://proxy.corp:3128",
        "headers": { "Authorization": "Bearer ${env:UPDATE_TOKEN}", "X-Api-Key": "k" }
    })));
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn updater_rejects_bad_proxy() {
    for bad in [
        "ftp://proxy.corp",
        "socks5://proxy.corp:1080",
        "proxy.corp:3128",
        "http://proxy.corp:3128/?x=1",
        "http://proxy.corp:3128/#frag",
        "",
    ] {
        let found = issues(net_updater(json!({ "proxy": bad })));
        assert!(has_issue(&found, "updater.proxy"), "{bad:?}: {found:?}");
    }
}

#[test]
fn updater_rejects_transport_owned_and_malformed_headers() {
    for name in [
        "Host",
        "content-length",
        "Transfer-Encoding",
        "Proxy-Authorization",
        "TE",
    ] {
        let found = issues(net_updater(json!({ "headers": { name: "x" } })));
        assert!(
            has_issue(&found, "cannot be configured"),
            "{name}: {found:?}"
        );
    }
    let found = issues(net_updater(json!({ "headers": { "bad name": "x" } })));
    assert!(has_issue(&found, "not a valid header name"), "{found:?}");
    let found = issues(net_updater(
        json!({ "headers": { "X-A": "1", "x-a": "2" } }),
    ));
    assert!(has_issue(&found, "is listed twice"), "{found:?}");
    let found = issues(net_updater(
        json!({ "headers": { "X-A": "a\r\nInjected: 1" } }),
    ));
    assert!(has_issue(&found, "control character"), "{found:?}");
    assert!(
        !has_issue(&found, "Injected"),
        "the value must not be echoed: {found:?}"
    );
    let found = issues(net_updater(
        json!({ "headers": { "X-A": "x".repeat(4097) } }),
    ));
    assert!(has_issue(&found, "longer than"), "{found:?}");
}

#[test]
fn updater_rejects_malformed_env_placeholders() {
    for value in ["${env:}", "${env:1BAD}", "${env:A-B}", "tok ${env:OPEN"] {
        let found = issues(net_updater(json!({ "headers": { "X-A": value } })));
        assert!(has_issue(&found, "placeholder"), "{value:?}: {found:?}");
    }
}

#[test]
fn updater_headers_need_a_fixed_endpoint_origin() {
    let found = issues(json!({ "updater": {
        "endpoints": ["https://{{channel}}.example.com/m.json"],
        "publicKeys": [KEY_A],
        "headers": { "X-A": "1" }
    }}));
    assert!(has_issue(&found, "fixed origin"), "{found:?}");
    // One fixed mirror is enough; placeholders in the path are fine.
    let found = issues(json!({ "updater": {
        "endpoints": ["https://{{channel}}.example.com/m.json", "https://u.example.com/{{channel}}.json"],
        "publicKeys": [KEY_A],
        "headers": { "X-A": "1" }
    }}));
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn bundle_metadata_is_validated() {
    let valid = json!({ "bundle": {
        "identifier": "com.example.app",
        "licenseFile": "LICENSE",
        "macos": { "minimumSystemVersion": "15", "entitlements": "app.plist" },
        "windows": {
            "installMode": "perMachine",
            "requestedExecutionLevel": "highestAvailable",
            "msix": {
                "publisher": "CN=Example",
                "minVersion": "10.0.19041.0",
                "version": "1.2.3.0",
            },
        },
        "deb": { "desktopTemplate": "app.desktop" },
        "pkgbuild": { "depends": ["libayatana-appindicator"] },
    }});
    assert_eq!(issues(valid), [] as [String; 0]);

    let invalid = issues(json!({ "bundle": {
        "identifier": "com.example.app",
        "macos": { "minimumSystemVersion": "13.6" },
        "windows": { "msix": {
            "publisher": "Example",
            "minVersion": "10.0",
            "version": "1.70000.0.0",
        } },
        "pkgbuild": { "depends": [""] },
    }}));
    assert!(has_issue(&invalid, "bundle.macos.minimumSystemVersion"));
    assert!(has_issue(&invalid, "bundle.windows.msix.version"));
    assert!(has_issue(&invalid, "bundle.pkgbuild.depends"));
    assert!(has_issue(&invalid, "bundle.windows.msix.publisher"));
    assert!(has_issue(&invalid, "bundle.windows.msix.minVersion"));
    assert!(has_issue(
        &issues(json!({ "bundle": {
            "identifier": "com.example.app",
            "macos": { "minimumSystemVersion": "14.0.0.1" },
        }})),
        "bundle.macos.minimumSystemVersion"
    ));
}

use std::{ffi::OsString, path::Path};

use crate::config::{ResolvedUpdaterConfig, ResolvedUpdaterNetwork, TyndConfig};

use super::{
    backend_bootstrap_source, backend_import_specifier, binary_args_with_separator,
    debug_socket_app_id, ensure_dev_url_unserved, resolve_debug_socket,
};

#[test]
fn dev_url_served_by_another_process_is_rejected() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let port = listener.local_addr().expect("address").port();
    let url = format!("http://127.0.0.1:{port}");
    let error = ensure_dev_url_unserved(&url).expect_err("occupied dev URL");
    assert!(error.to_string().contains("already served"));
    drop(listener);
    ensure_dev_url_unserved(&url).expect("free dev URL");
}

#[test]
fn backend_specifier_is_relative_on_windows_paths() {
    assert_eq!(
        backend_import_specifier(
            r"\\?\C:\Users\Jane Doe\my app\.tynd\cache\generated",
            r"\\?\C:\Users\Jane Doe\my app\backend\main.ts",
        ),
        "../../../backend/main.ts"
    );
    assert_eq!(
        backend_import_specifier(
            r"C:\Projets\café\.tynd\cache\generated",
            r"c:\Projets\café\src\server\main.ts",
        ),
        "../../../src/server/main.ts"
    );
    assert_eq!(
        backend_import_specifier(r"C:\app\.tynd\cache\generated", r"D:\shared\main.ts"),
        "D:/shared/main.ts"
    );
    assert_eq!(
        backend_import_specifier(
            r"\\?\UNC\server\share\app\.tynd\cache\generated",
            r"\\?\UNC\server\share\app\main.ts",
        ),
        "../../../main.ts"
    );
    assert_eq!(
        backend_import_specifier(
            r"\\?\UNC\server\one\.tynd\cache\generated",
            r"\\?\UNC\server\two\main.ts",
        ),
        "//server/two/main.ts"
    );
}

#[test]
fn backend_specifier_is_relative_on_posix_paths() {
    assert_eq!(
        backend_import_specifier(
            "/home/jane/my app/.tynd/cache/generated",
            "/home/jane/my app/backend/main.ts",
        ),
        "../../../backend/main.ts"
    );
    assert_eq!(
        backend_import_specifier("/srv/app/gen", "/srv/app/gen/x.ts"),
        "./x.ts"
    );
    assert_eq!(
        backend_import_specifier("/a/.tynd/cache/generated", "/b/main.ts"),
        "../../../../b/main.ts"
    );
}

#[test]
fn binary_args_follow_a_separator() {
    assert_eq!(binary_args_with_separator(&[]), [] as [OsString; 0]);
    let args = binary_args_with_separator(&["--foo".into(), "-h".into()]);
    assert_eq!(
        args,
        [
            OsString::from("--"),
            OsString::from("--foo"),
            OsString::from("-h")
        ]
    );
}

#[test]
fn debug_socket_is_named_after_the_app() {
    let root = Path::new("/projects/lb1");
    let bare: TyndConfig = serde_json::from_value(serde_json::json!({})).expect("config");
    assert_eq!(debug_socket_app_id(&bare, root), "lb1");
    let bundled: TyndConfig = serde_json::from_value(serde_json::json!({
        "bundle": { "identifier": "com.example.lb1" }
    }))
    .expect("config");
    assert_eq!(debug_socket_app_id(&bundled, root), "com.example.lb1");
}

#[test]
fn keeps_a_preset_debug_socket() {
    let socket = resolve_debug_socket(false, Some(OsString::from("/tmp/tynd-test-1.sock")), "app")
        .expect("socket");
    assert_eq!(socket.path, OsString::from("/tmp/tynd-test-1.sock"));
    assert!(socket.private_dir.is_none());
}

#[test]
fn generates_a_socket_when_none_is_preset() {
    for preset in [None, Some(OsString::new())] {
        let socket = resolve_debug_socket(false, preset, "my app").expect("socket");
        let path = socket.path.to_string_lossy().into_owned();
        if cfg!(windows) {
            assert!(path.starts_with(r"\\.\pipe\tynd-my_app-"), "{path}");
            let again = resolve_debug_socket(false, None, "my app").expect("socket");
            assert_ne!(socket.path, again.path, "pipe name must be unguessable");
        } else {
            let dir = socket.private_dir.clone().expect("private dir");
            assert!(Path::new(&path).starts_with(&dir), "{path}");
            assert!(dir.is_dir());
            drop(socket);
            assert!(!dir.exists(), "private dir is removed on drop");
        }
    }
}

#[cfg(unix)]
#[test]
fn generated_socket_lives_in_a_0700_directory() {
    use std::os::unix::fs::PermissionsExt as _;

    let socket = resolve_debug_socket(false, None, "app").expect("socket");
    let dir = socket.private_dir.clone().expect("private dir");
    assert_eq!(dir.metadata().unwrap().permissions().mode() & 0o777, 0o700);
    assert!(socket.path.len() < 100, "fits sun_path");
}

#[test]
fn no_debug_disables_the_socket_even_when_preset() {
    assert!(resolve_debug_socket(true, Some(OsString::from("/tmp/x.sock")), "app").is_none());
}

fn sample_updater() -> ResolvedUpdaterConfig {
    serde_json::from_value(serde_json::json!({
        "endpoints": ["https://example.com/{{channel}}.json"],
        "publicKeys": ["PUBKEY"],
        "channel": "stable",
        "preferPatch": false,
        "allowPatch": false,
        "maxPatchSize": null,
        "fallbackToFull": true,
        "autoCheck": true,
        "checkIntervalSecs": 3600,
        "checkOnStartup": true,
        "autoDownload": false,
        "autoApply": false,
        "promptStyle": "modal",
        "requireSignature": true,
        "requireMajorPrompt": true
    }))
    .expect("updater config")
}

/// The JSON object assigned to `__TYND_UPDATER_CONFIG__` in `source`.
fn emitted_updater_config(source: &str) -> serde_json::Value {
    let line = source
        .lines()
        .find_map(|line| line.strip_prefix("Object.assign(globalThis, { __TYND_UPDATER_CONFIG__: "))
        .expect("updater config line");
    serde_json::from_str(line.trim_end_matches(" });")).expect("emitted JSON")
}

#[test]
fn bootstrap_has_no_any_casts() {
    let updater = sample_updater();
    let source = backend_bootstrap_source(&["@acme/x".to_owned()], Some(&updater), None, "1.0.0")
        .expect("source");
    assert!(!source.contains("as any"), "{source}");
    assert!(source.starts_with("import "), "{source}");
}

#[test]
fn bootstrap_injects_app_info_and_updater_config() {
    let updater = sample_updater();
    let source = backend_bootstrap_source(&[], Some(&updater), None, "1.2.3").expect("source");
    assert!(
        source.contains("__TYND_APP_INFO__: {\"version\":\"1.2.3\"} });"),
        "{source}"
    );
    assert!(source.contains("__TYND_UPDATER_CONFIG__: {"), "{source}");
    assert!(source.contains("PUBKEY"), "{source}");
    assert!(!source.to_lowercase().contains("private"), "{source}");
    let emitted = emitted_updater_config(&source);
    assert!(emitted.get("headers").is_none());
    assert!(emitted.get("proxy").is_none());
}

#[test]
fn bootstrap_merges_proxy_and_headers_with_exact_escaping() {
    let updater = sample_updater();
    let token = r#"Bearer "q" \ ${x} </script> end"#.to_owned() + "\u{2028}\t";
    let network = ResolvedUpdaterNetwork {
        proxy: Some("http://proxy.corp:3128".into()),
        headers: [("Authorization".to_owned(), token.clone())].into(),
    };
    let source =
        backend_bootstrap_source(&[], Some(&updater), Some(&network), "1.2.3").expect("source");
    let emitted = emitted_updater_config(&source);
    assert_eq!(emitted["proxy"], "http://proxy.corp:3128");
    assert_eq!(emitted["headers"]["Authorization"], token);
    // The updater fields stay next to the network settings.
    assert_eq!(emitted["publicKeys"], serde_json::json!(["PUBKEY"]));
    // One physical line: the value cannot break out of the assignment.
    assert_eq!(
        source
            .lines()
            .filter(|line| line.contains("__TYND_UPDATER_CONFIG__"))
            .count(),
        1
    );
}

#[test]
fn bootstrap_without_updater_still_reports_the_version() {
    let source =
        backend_bootstrap_source(&["@tynd/x".into()], None, None, "0.0.0").expect("source");
    assert!(source.contains("__TYND_APP_INFO__"), "{source}");
    assert!(!source.contains("__TYND_UPDATER_CONFIG__"), "{source}");
    assert!(
        source.contains("__tyndRegisterPlugin(__tyndPlugin0);"),
        "{source}"
    );
}

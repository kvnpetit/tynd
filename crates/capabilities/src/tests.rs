use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::*;

fn cap(value: Value) -> Capability {
    serde_json::from_value(value).expect("valid capability json")
}

fn caps(list: Vec<Capability>) -> BTreeMap<String, Capability> {
    list.into_iter()
        .map(|c| (c.identifier.clone(), c))
        .collect()
}

fn fs_manifest() -> BTreeMap<String, Manifest> {
    let manifest = manifest_from_plugin_commands("fs", &["readFile", "write_file"])
        .expect("valid commands")
        .expect("non-empty");
    BTreeMap::from([("fs".to_string(), manifest)])
}

fn resolve(
    manifests: &BTreeMap<String, Manifest>,
    list: Vec<Capability>,
    target: Target,
) -> Resolved {
    Resolved::resolve(manifests, caps(list), target).expect("resolves")
}

// ---------------------------------------------------------------- identifiers

#[test]
fn window_label_validation() {
    for ok in ["main", "a", "Settings_2", "dev-tools", "  main  "] {
        assert!(WindowLabel::try_new(ok).is_ok(), "{ok:?}");
    }
    for bad in [
        "",
        "   ",
        "1main",
        "-x",
        "has space",
        "a.b",
        &"a".repeat(65),
    ] {
        assert!(WindowLabel::try_new(bad).is_err(), "{bad:?}");
    }
    assert_eq!(WindowLabel::try_new("  main ").unwrap().as_ref(), "main");
    assert!(WindowLabel::try_new("a".repeat(64)).is_ok());
}

#[test]
fn plugin_name_is_lowercased_kebab() {
    assert_eq!(PluginName::try_new("FS").unwrap().as_ref(), "fs");
    assert!(PluginName::try_new("http-server").is_ok());
    for bad in ["", "1fs", "fs_x", "fs:x", "-fs"] {
        assert!(PluginName::try_new(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn method_api_capability_and_event_names() {
    assert!(MethodName::try_new("_private").is_ok());
    assert!(MethodName::try_new("readFile").is_ok());
    assert!(MethodName::try_new("read-file_2").is_ok());
    assert!(MethodName::try_new("9read").is_err());
    assert!(MethodName::try_new("a:b").is_err());

    assert_eq!(ApiName::try_new("Window").unwrap().as_ref(), "window");
    assert!(ApiName::try_new("win-dow").is_err());

    assert_eq!(
        CapabilityId::try_new("Main-Default").unwrap().as_ref(),
        "main-default"
    );
    assert!(CapabilityId::try_new("main default").is_err());

    assert!(EventName::try_new("oauth:callback").is_ok());
    assert!(EventName::try_new("").is_err());
    assert!(EventName::try_new("e".repeat(256)).is_ok());
    assert!(EventName::try_new("e".repeat(257)).is_err());
}

#[test]
fn identifiers_serialize_as_plain_strings() {
    let label = WindowLabel::try_new("main").unwrap();
    assert_eq!(serde_json::to_value(&label).unwrap(), json!("main"));
    let back: WindowLabel = serde_json::from_value(json!("main")).unwrap();
    assert_eq!(back, label);
    assert!(serde_json::from_value::<WindowLabel>(json!("1bad")).is_err());
}

#[test]
fn command_parses_plugin_and_method() {
    let cmd: Command = "fs:readFile".parse().unwrap();
    assert_eq!(cmd.plugin.as_ref(), "fs");
    assert_eq!(cmd.method.as_ref(), "readFile");

    for bad in ["nocolon", ":read", "fs:", "Fs 1:read", "fs:9bad"] {
        let err = bad.parse::<Command>().unwrap_err();
        assert!(!err.to_string().is_empty(), "{bad}");
    }
    assert!("nocolon"
        .parse::<Command>()
        .unwrap_err()
        .to_string()
        .contains("missing ':'"));
}

// ------------------------------------------------------------------- manifest

#[test]
fn command_keys() {
    assert_eq!(
        manifest_command_key(APP_ACL_KEY, "window:close"),
        "window:close"
    );
    assert_eq!(manifest_command_key("fs", "read"), "plugin:fs|read");
    assert_eq!(plugin_command_key("fs", "read").unwrap(), "plugin:fs|read");
    assert!(plugin_command_key("fs", "9bad").is_err());
    assert!(plugin_command_key(APP_ACL_KEY, "read").is_err());
}

#[test]
fn lookup_keys_include_wildcards() {
    assert_eq!(
        command_lookup_keys("plugin:fs|read"),
        vec!["plugin:fs|read".to_string(), "plugin:fs|*".to_string()]
    );
    assert_eq!(
        command_lookup_keys("plugin:fs|*"),
        vec!["plugin:fs|*".to_string()]
    );
    assert_eq!(
        command_lookup_keys("window:close"),
        vec!["window:close".to_string(), "window:*".to_string()]
    );
    assert_eq!(
        command_lookup_keys("window:*"),
        vec!["window:*".to_string()]
    );
    assert_eq!(command_lookup_keys("plain"), vec!["plain".to_string()]);
}

#[test]
fn validate_plugin_name_rules() {
    assert!(validate_plugin_name("fs").is_ok());
    assert!(validate_plugin_name("http-server").is_ok());
    assert!(validate_plugin_name(APP_ACL_KEY).is_err());
    assert!(validate_plugin_name("Fs").is_err(), "must be canonical");
    assert!(validate_plugin_name("").is_err());
    assert!(validate_plugin_name("fs:x").is_err());
}

#[test]
fn manifest_from_plugin_commands_builds_allow_and_deny_sets() {
    assert_eq!(manifest_from_plugin_commands("fs", &[]).unwrap(), None);

    let manifest = manifest_from_plugin_commands("fs", &["readFile", "write_file"])
        .unwrap()
        .unwrap();
    assert_eq!(manifest.default_permission.as_deref(), Some("allow-all"));
    for id in [
        "allow-read-file",
        "deny-read-file",
        "allow-write-file",
        "deny-write-file",
    ] {
        assert!(manifest.permissions.contains_key(id), "{id}");
    }
    assert_eq!(
        manifest.permissions["allow-read-file"].commands.allow,
        vec!["readFile".to_string()]
    );
    assert_eq!(
        manifest.permissions["deny-write-file"].commands.deny,
        vec!["write_file".to_string()]
    );
    assert_eq!(manifest.permission_sets["allow-all"].len(), 2);
    assert_eq!(manifest.permission_sets["deny-all"].len(), 2);
}

#[test]
fn manifest_from_plugin_commands_rejects_bad_input() {
    assert!(manifest_from_plugin_commands(APP_ACL_KEY, &["a"]).is_err());
    assert!(manifest_from_plugin_commands("Bad", &["a"]).is_err());
    assert!(manifest_from_plugin_commands("fs", &["9bad"]).is_err());
    // `readFile` and `read_file` collapse to the same permission suffix.
    let err = manifest_from_plugin_commands("fs", &["readFile", "read_file"]).unwrap_err();
    assert!(err.contains("duplicate"), "{err}");
}

// ------------------------------------------------------------------- resolver

#[test]
fn split_permission_ref_defaults_to_app_key() {
    assert_eq!(split_permission_ref("fs:default"), ("fs", "default"));
    assert_eq!(split_permission_ref("allow-x"), (APP_ACL_KEY, "allow-x"));
    assert_eq!(split_permission_ref("a:b:c"), ("a", "b:c"));
}

#[test]
fn platforms_match_rules() {
    assert!(platforms_match(None, Target::Linux));
    assert!(platforms_match(
        Some(&[Target::Linux, Target::Windows]),
        Target::Windows
    ));
    assert!(!platforms_match(Some(&[Target::Linux]), Target::MacOS));
    assert!(!platforms_match(Some(&[]), Target::Linux));
}

#[test]
fn target_current_is_stable() {
    assert_eq!(Target::current(), Target::current());
    if cfg!(target_os = "windows") {
        assert_eq!(Target::current(), Target::Windows);
    }
}

#[test]
fn resolved_default_permission_allows_plugin_commands() {
    let manifests = fs_manifest();
    let r = resolve(
        &manifests,
        vec![cap(json!({
            "identifier": "main",
            "windows": ["main"],
            "permissions": ["fs:default"]
        }))],
        Target::Windows,
    );
    assert!(r.is_allowed("plugin:fs|readFile", Some("main"), None, None));
    assert!(r.is_allowed("plugin:fs|write_file", Some("main"), None, None));
    // Wrong window, unknown command, other plugin.
    assert!(!r.is_allowed("plugin:fs|readFile", Some("other"), None, None));
    assert!(!r.is_allowed("plugin:fs|readFile", None, None, None));
    assert!(!r.is_allowed("plugin:fs|delete", Some("main"), None, None));
    assert!(!r.is_allowed("plugin:shell|readFile", Some("main"), None, None));
}

#[test]
fn resolved_deny_overrides_allow() {
    let manifests = fs_manifest();
    let r = resolve(
        &manifests,
        vec![cap(json!({
            "identifier": "main",
            "permissions": ["fs:allow-all", "fs:deny-write-file"]
        }))],
        Target::Linux,
    );
    assert!(r.is_allowed("plugin:fs|readFile", None, None, None));
    assert!(!r.is_allowed("plugin:fs|write_file", None, None, None));
}

#[test]
fn resolved_deny_in_other_capability_overrides_allow() {
    let manifests = fs_manifest();
    let r = resolve(
        &manifests,
        vec![
            cap(json!({ "identifier": "a", "permissions": ["fs:allow-all"] })),
            cap(json!({ "identifier": "b", "permissions": ["fs:deny-read-file"] })),
        ],
        Target::Linux,
    );
    assert!(!r.is_allowed("plugin:fs|readFile", None, None, None));
    assert!(r.is_allowed("plugin:fs|write_file", None, None, None));
}

#[test]
fn resolved_inline_permissions_and_wildcards() {
    let r = resolve(
        &BTreeMap::new(),
        vec![cap(json!({
            "identifier": "main",
            "permissions": [
                { "identifier": "win", "commands": { "allow": ["window:*"], "deny": ["window:close"] } },
                { "identifier": "menu", "commands": { "allow": ["menu:popup"] } }
            ]
        }))],
        Target::Linux,
    );
    assert!(r.is_allowed("window:setTitle", None, None, None));
    assert!(!r.is_allowed("window:close", None, None, None));
    assert!(r.is_allowed("menu:popup", None, None, None));
    assert!(!r.is_allowed("menu:other", None, None, None));
    assert!(!r.is_allowed("tray:show", None, None, None));
}

#[test]
fn resolved_window_and_webview_scopes_both_narrow() {
    let r = resolve(
        &BTreeMap::new(),
        vec![cap(json!({
            "identifier": "main",
            "windows": ["main"],
            "webviews": ["editor-*"],
            "permissions": [{ "identifier": "p", "commands": { "allow": ["x:y"] } }]
        }))],
        Target::Linux,
    );
    assert!(r.is_allowed("x:y", Some("main"), Some("editor-1"), None));
    assert!(!r.is_allowed("x:y", Some("main"), Some("preview"), None));
    assert!(!r.is_allowed("x:y", Some("other"), Some("editor-1"), None));
    assert!(!r.is_allowed("x:y", Some("main"), None, None));
    assert!(!r.is_allowed("x:y", None, Some("editor-1"), None));
}

#[test]
fn resolved_window_globs_match() {
    let r = resolve(
        &BTreeMap::new(),
        vec![cap(json!({
            "identifier": "main",
            "windows": ["tool-*"],
            "permissions": [{ "identifier": "p", "commands": { "allow": ["x:y"] } }]
        }))],
        Target::Linux,
    );
    assert!(r.is_allowed("x:y", Some("tool-a"), None, None));
    assert!(!r.is_allowed("x:y", Some("main"), None, None));
}

#[test]
fn resolved_without_scopes_matches_any_window() {
    let r = resolve(
        &BTreeMap::new(),
        vec![cap(json!({
            "identifier": "main",
            "permissions": [{ "identifier": "p", "commands": { "allow": ["x:y"] } }]
        }))],
        Target::Linux,
    );
    assert!(r.is_allowed("x:y", None, None, None));
    assert!(r.is_allowed("x:y", Some("anything"), Some("whatever"), None));
}

#[test]
fn resolved_origin_rules() {
    let local_only = resolve(
        &BTreeMap::new(),
        vec![cap(json!({
            "identifier": "a",
            "permissions": [{ "identifier": "p", "commands": { "allow": ["x:y"] } }]
        }))],
        Target::Linux,
    );
    assert!(local_only.is_allowed("x:y", None, None, None));
    // A remote origin never matches a capability without remote patterns.
    assert!(!local_only.is_allowed("x:y", None, None, Some("https://evil.test/")));

    let remote_only = resolve(
        &BTreeMap::new(),
        vec![cap(json!({
            "identifier": "b",
            "local": false,
            "remote": { "urls": ["https://app.test/*"] },
            "permissions": [{ "identifier": "p", "commands": { "allow": ["x:y"] } }]
        }))],
        Target::Linux,
    );
    assert!(!remote_only.is_allowed("x:y", None, None, None));
    assert!(remote_only.is_allowed("x:y", None, None, Some("https://app.test/page")));
    assert!(!remote_only.is_allowed("x:y", None, None, Some("https://evil.test/page")));
    assert!(!remote_only.is_allowed("x:y", None, None, Some("not a url")));
}

#[test]
fn resolved_platform_filtering() {
    let list = vec![
        cap(json!({
            "identifier": "linux-only",
            "platforms": ["linux"],
            "permissions": [{ "identifier": "p", "commands": { "allow": ["a:b"] } }]
        })),
        cap(json!({
            "identifier": "mixed",
            "permissions": [
                { "identifier": "p1", "platforms": ["windows"], "commands": { "allow": ["c:d"] } },
                { "identifier": "p2", "commands": { "allow": ["e:f"] } }
            ]
        })),
    ];
    let linux = resolve(&BTreeMap::new(), list.clone(), Target::Linux);
    assert!(linux.is_allowed("a:b", None, None, None));
    assert!(!linux.is_allowed("c:d", None, None, None));
    assert!(linux.is_allowed("e:f", None, None, None));

    let windows = resolve(&BTreeMap::new(), list, Target::Windows);
    assert!(!windows.is_allowed("a:b", None, None, None));
    assert!(windows.is_allowed("c:d", None, None, None));
    assert!(windows.is_allowed("e:f", None, None, None));
}

#[test]
fn resolved_empty_denies_everything() {
    let r = resolve(&BTreeMap::new(), vec![], Target::Linux);
    assert!(!r.is_allowed("window:close", None, None, None));
    assert!(!r.is_allowed("plugin:fs|read", Some("main"), None, None));
}

#[test]
fn resolve_reports_errors() {
    let unknown_manifest = Resolved::resolve(
        &BTreeMap::new(),
        caps(vec![cap(
            json!({ "identifier": "a", "permissions": ["nope:default"] }),
        )]),
        Target::Linux,
    );
    assert!(unknown_manifest.unwrap_err().contains("unknown manifest"));

    let manifests = fs_manifest();
    let unknown_permission = Resolved::resolve(
        &manifests,
        caps(vec![cap(
            json!({ "identifier": "a", "permissions": ["fs:allow-missing"] }),
        )]),
        Target::Linux,
    );
    assert!(unknown_permission
        .unwrap_err()
        .contains("unknown permission"));

    let bad_glob = Resolved::resolve(
        &BTreeMap::new(),
        caps(vec![cap(json!({ "identifier": "a", "windows": ["[bad"] }))]),
        Target::Linux,
    );
    assert!(bad_glob.unwrap_err().contains("invalid glob"));

    let mut no_default = fs_manifest();
    no_default.get_mut("fs").unwrap().default_permission = None;
    let missing_default = Resolved::resolve(
        &no_default,
        caps(vec![cap(
            json!({ "identifier": "a", "permissions": ["fs:default"] }),
        )]),
        Target::Linux,
    );
    assert!(missing_default.unwrap_err().contains("default permission"));
}

fn set_manifest(sets: &[(&str, &[&str])]) -> BTreeMap<String, Manifest> {
    let mut manifest = Manifest {
        default_permission: None,
        permissions: BTreeMap::new(),
        permission_sets: BTreeMap::new(),
        global_scope_schema: None,
    };
    manifest.permissions.insert(
        "leaf".into(),
        serde_json::from_value(json!({
            "identifier": "leaf",
            "commands": { "allow": ["go"] }
        }))
        .unwrap(),
    );
    for (name, members) in sets {
        manifest.permission_sets.insert(
            (*name).to_string(),
            members.iter().map(|m| (*m).to_string()).collect(),
        );
    }
    BTreeMap::from([("p".to_string(), manifest)])
}

fn resolve_ref(
    manifests: &BTreeMap<String, Manifest>,
    reference: &str,
) -> Result<Resolved, String> {
    Resolved::resolve(
        manifests,
        caps(vec![cap(
            json!({ "identifier": "a", "permissions": [reference] }),
        )]),
        Target::Linux,
    )
}

#[test]
fn permission_sets_expand_nested_and_dedupe() {
    let manifests = set_manifest(&[
        ("outer", &["inner", "leaf"]),
        ("inner", &["leaf", "p:leaf"]),
    ]);
    let r = resolve_ref(&manifests, "p:outer").expect("resolves");
    assert!(r.is_allowed("plugin:p|go", None, None, None));
}

#[test]
fn permission_set_cycles_are_rejected() {
    let manifests = set_manifest(&[("a", &["b"]), ("b", &["a"])]);
    let err = resolve_ref(&manifests, "p:a").unwrap_err();
    assert!(err.contains("cycle"), "{err}");
}

#[test]
fn permission_set_foreign_references_are_rejected() {
    let manifests = set_manifest(&[("a", &["other:leaf"])]);
    let err = resolve_ref(&manifests, "p:a").unwrap_err();
    assert!(err.contains("foreign manifest"), "{err}");
}

#[test]
fn permission_and_set_name_collision_is_rejected() {
    let manifests = set_manifest(&[("leaf", &["leaf"])]);
    let err = resolve_ref(&manifests, "p:leaf").unwrap_err();
    assert!(
        err.contains("both a permission and permission set"),
        "{err}"
    );
}

// ---------------------------------------------------------------------- model

#[test]
fn capability_defaults() {
    let c = cap(json!({ "identifier": "main" }));
    assert!(c.local);
    assert!(c.remote.is_none());
    assert!(c.windows.is_empty() && c.webviews.is_empty() && c.permissions.is_empty());
    assert!(c.platforms.is_none());
    assert_eq!(c.description, "");
}

#[test]
fn capability_requires_identifier() {
    assert!(serde_json::from_value::<Capability>(json!({ "local": true })).is_err());
}

#[test]
fn capability_round_trips_camel_case() {
    let c = cap(json!({
        "identifier": "main",
        "description": "d",
        "local": false,
        "remote": { "urls": ["https://x.test/*"] },
        "windows": ["main"],
        "webviews": ["w"],
        "permissions": ["fs:default", { "identifier": "p", "commands": { "allow": ["a:b"] } }],
        "platforms": ["linux", "macOS", "windows", "android", "iOS"]
    }));
    let value = serde_json::to_value(&c).unwrap();
    assert_eq!(value["webviews"], json!(["w"]));
    assert_eq!(value["platforms"][1], json!("macOS"));
    assert_eq!(value["platforms"][4], json!("iOS"));
    let back: Capability = serde_json::from_value(value).unwrap();
    assert_eq!(back, c);
    assert!(matches!(
        back.permissions[0],
        PermissionEntry::PermissionRef(_)
    ));
    assert!(matches!(back.permissions[1], PermissionEntry::Inline(_)));
}

#[test]
fn permission_version_must_be_non_zero() {
    let bad = serde_json::from_value::<Permission>(json!({ "identifier": "p", "version": 0 }));
    assert!(bad.is_err());
    let ok: Permission =
        serde_json::from_value(json!({ "identifier": "p", "version": 2 })).unwrap();
    assert_eq!(ok.version.map(std::num::NonZeroU64::get), Some(2));
    assert_eq!(ok.commands, Commands::default());
    assert_eq!(ok.scope, Value::Null);
}

#[test]
fn unknown_target_is_rejected() {
    assert!(serde_json::from_value::<Target>(json!("beos")).is_err());
    assert_eq!(
        serde_json::from_value::<Target>(json!("macOS")).unwrap(),
        Target::MacOS
    );
}

#[test]
fn capability_file_accepts_three_shapes() {
    let single = json!({ "identifier": "a" });
    let list = json!([{ "identifier": "a" }, { "identifier": "b" }]);
    let named = json!({ "capabilities": [{ "identifier": "a" }, { "identifier": "b" }] });

    let one = CapabilityFile::from_bytes(single.to_string().as_bytes())
        .unwrap()
        .into_capabilities();
    assert_eq!(one.len(), 1);
    for shape in [list, named] {
        let many = CapabilityFile::from_bytes(shape.to_string().as_bytes())
            .unwrap()
            .into_capabilities();
        assert_eq!(
            many.iter()
                .map(|c| c.identifier.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }
}

#[test]
fn capability_file_rejects_garbage() {
    assert!(CapabilityFile::from_bytes(b"not json").is_err());
    assert!(CapabilityFile::from_bytes(b"{\"unrelated\": 1}").is_err());
    assert!(CapabilityFile::from_bytes(b"42").is_err());
}

// --------------------------------------------------------------------- schema

#[test]
fn capability_shape_validation() {
    assert!(validate_capability_shape(&cap(json!({ "identifier": "a" }))).is_ok());
    assert!(validate_capability_shape(&cap(json!({
        "identifier": "a", "local": false, "remote": { "urls": [] }
    })))
    .is_ok());
    let err =
        validate_capability_shape(&cap(json!({ "identifier": "a", "local": false }))).unwrap_err();
    assert!(err.contains("'a'"), "{err}");
}

#[test]
fn capability_schema_shape() {
    let schema = capability_schema();
    assert_eq!(schema["title"], json!("Tynd Capability"));
    assert_eq!(schema["oneOf"].as_array().unwrap().len(), 3);
    let props = &schema["definitions"]["Capability"]["properties"];
    for key in [
        "identifier",
        "description",
        "local",
        "remote",
        "windows",
        "webviews",
        "permissions",
        "platforms",
    ] {
        assert!(props.get(key).is_some(), "{key}");
    }
    assert_eq!(
        schema["definitions"]["Capability"]["required"],
        json!(["identifier"])
    );
}

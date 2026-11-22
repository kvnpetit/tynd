use std::str::FromStr;

use serde_json::{json, Value};

use crate::{
    runtime::{
        BackendCall, BackendConfig, BackendReturnPayload, MenuItemDef, MenuRole, TrayConfig,
    },
    window::{ColorSpace, GpuPreference, HardwareAcceleration, PreferredFps, WindowConfig},
};

#[test]
fn backend_return_payload_is_ok() {
    assert!(BackendReturnPayload::Ok("x".into()).is_ok());
    assert!(!BackendReturnPayload::Err("x".into()).is_ok());
}

#[test]
fn backend_call_constructors() {
    match BackendCall::call("id-1", "greet", vec![json!("a"), json!(2)]) {
        BackendCall::Typed { id, fn_name, args } => {
            assert_eq!(id, "id-1");
            assert_eq!(fn_name, "greet");
            assert_eq!(args, vec![json!("a"), json!(2)]);
        },
        other => panic!("unexpected variant: {other:?}"),
    }
    match BackendCall::lifecycle("ready") {
        BackendCall::Typed { id, fn_name, args } => {
            assert_eq!(id, "__tynd_ready__");
            assert_eq!(fn_name, "__tynd_ready__");
            assert_eq!(args, [] as [Value; 0]);
        },
        other => panic!("unexpected variant: {other:?}"),
    }
}

#[test]
fn backend_config_defaults_from_empty_object() {
    let cfg: BackendConfig = serde_json::from_value(json!({})).unwrap();
    assert!(cfg.window.title.is_none());
    assert!(cfg.security.is_none() && cfg.capabilities.is_none());
    assert!(cfg.menu.is_empty());
    assert!(cfg.tray.is_none());
    assert!(!cfg.quit_on_last_window_closed);
    assert!(!cfg.smoke_mode);
}

#[test]
fn backend_config_reads_camel_case_fields() {
    let cfg: BackendConfig = serde_json::from_value(json!({
        "window": { "title": "App", "width": 800, "alwaysOnTop": true },
        "security": { "defaultDeny": true },
        "capabilities": [{ "identifier": "main" }],
        "devUrl": "http://localhost:5173",
        "frontendDir": "dist",
        "iconPath": "icon.png",
        "quitOnLastWindowClosed": true,
        "menu": [{ "type": "separator" }],
        "tray": { "icon": "tray.png", "tooltip": "tip" }
    }))
    .unwrap();
    assert_eq!(cfg.window.title.as_deref(), Some("App"));
    assert_eq!(cfg.window.width, Some(800));
    assert_eq!(cfg.window.always_on_top, Some(true));
    assert_eq!(cfg.security, Some(json!({ "defaultDeny": true })));
    assert!(cfg.capabilities.is_some());
    assert_eq!(cfg.dev_url.as_deref(), Some("http://localhost:5173"));
    assert_eq!(cfg.frontend_dir.as_deref(), Some("dist"));
    assert_eq!(cfg.icon_path.as_deref(), Some("icon.png"));
    assert!(cfg.quit_on_last_window_closed);
    assert_eq!(cfg.menu.len(), 1);
    let tray: &TrayConfig = cfg.tray.as_ref().unwrap();
    assert_eq!(tray.icon, "tray.png");
    assert_eq!(tray.tooltip.as_deref(), Some("tip"));
}

#[test]
fn smoke_mode_cannot_be_set_from_json() {
    let cfg: BackendConfig = serde_json::from_value(json!({ "smokeMode": true })).unwrap();
    assert!(!cfg.smoke_mode);
}

#[test]
fn menu_role_serde_and_strum_agree() {
    let cases = [
        (MenuRole::Quit, "quit"),
        (MenuRole::SelectAll, "selectAll"),
        (MenuRole::Separator, "separator"),
        (MenuRole::About, "about"),
    ];
    for (role, text) in cases {
        assert_eq!(serde_json::to_value(role).unwrap(), json!(text));
        assert_eq!(role.to_string(), text);
        assert_eq!(MenuRole::from_str(text), Ok(role));
        assert_eq!(
            serde_json::from_value::<MenuRole>(json!(text)).unwrap(),
            role
        );
    }
    assert!(MenuRole::from_str("nope").is_err());
    assert!(serde_json::from_value::<MenuRole>(json!("nope")).is_err());
}

#[test]
fn menu_item_def_tagged_round_trip() {
    let raw = json!([
        { "type": "separator" },
        { "type": "item", "id": "open", "label": "Open", "enabled": false, "accelerator": "Ctrl+O" },
        { "type": "item", "role": "quit" },
        { "type": "checkbox", "id": "c", "label": "Check", "checked": true },
        { "type": "radio", "id": "r", "label": "Radio", "checked": false },
        {
            "type": "submenu", "id": "file", "label": "File",
            "items": [{ "type": "item", "id": "nested", "label": "Nested" }]
        }
    ]);
    let items: Vec<MenuItemDef> = serde_json::from_value(raw).unwrap();
    assert_eq!(items.len(), 6);

    assert_eq!((items[0].id(), items[0].label()), (None, None));
    assert_eq!(
        (items[1].id(), items[1].label()),
        (Some("open"), Some("Open"))
    );
    assert_eq!(items[3].id(), Some("c"));
    assert_eq!(items[4].label(), Some("Radio"));
    match &items[2] {
        MenuItemDef::Item(item) => assert_eq!(item.role, Some(MenuRole::Quit)),
        other => panic!("unexpected variant: {other:?}"),
    }
    match &items[5] {
        MenuItemDef::Submenu(sub) => {
            assert_eq!(sub.items.len(), 1);
            assert_eq!(sub.items[0].id(), Some("nested"));
        },
        other => panic!("unexpected variant: {other:?}"),
    }

    let serialized = serde_json::to_value(&items).unwrap();
    assert_eq!(serialized[0], json!({ "type": "separator" }));
    assert_eq!(serialized[1]["type"], json!("item"));
    assert_eq!(serialized[1]["accelerator"], json!("Ctrl+O"));
    assert_eq!(serialized[5]["items"][0]["id"], json!("nested"));
    let reparsed: Vec<MenuItemDef> = serde_json::from_value(serialized.clone()).unwrap();
    assert_eq!(serde_json::to_value(&reparsed).unwrap(), serialized);
}

#[test]
fn menu_item_def_rejects_unknown_type() {
    assert!(serde_json::from_value::<MenuItemDef>(json!({ "type": "bogus" })).is_err());
    assert!(serde_json::from_value::<MenuItemDef>(json!({ "id": "no-type" })).is_err());
}

#[test]
fn window_config_all_fields_optional() {
    let cfg: WindowConfig = serde_json::from_value(json!({})).unwrap();
    assert!(cfg.title.is_none() && cfg.width.is_none() && cfg.gpu.is_none());
}

#[test]
fn window_config_camel_case_and_enums() {
    let cfg: WindowConfig = serde_json::from_value(json!({
        "minWidth": 100,
        "maxHeight": 900,
        "userAgent": "ua",
        "contextLostRecovery": true,
        "hardwareAcceleration": "high-performance",
        "colorSpace": "rec2020",
        "preferredFps": 144,
        "gpu": { "vendor": "nvidia" }
    }))
    .unwrap();
    assert_eq!(cfg.min_width, Some(100));
    assert_eq!(cfg.max_height, Some(900));
    assert_eq!(cfg.user_agent.as_deref(), Some("ua"));
    assert_eq!(cfg.context_lost_recovery, Some(true));
    assert_eq!(
        cfg.hardware_acceleration,
        Some(HardwareAcceleration::HighPerformance)
    );
    assert_eq!(cfg.color_space, Some(ColorSpace::Rec2020));
    assert_eq!(cfg.preferred_fps, Some(PreferredFps::Hz(144)));
    match cfg.gpu {
        Some(GpuPreference::Vendor(v)) => assert_eq!(v.vendor, "nvidia"),
        other => panic!("unexpected gpu: {other:?}"),
    }
}

#[test]
fn untagged_gpu_and_fps_variants() {
    assert_eq!(
        serde_json::from_value::<GpuPreference>(json!("discrete")).unwrap(),
        GpuPreference::Named("discrete".into())
    );
    assert_eq!(
        serde_json::from_value::<PreferredFps>(json!("monitor")).unwrap(),
        PreferredFps::Auto("monitor".into())
    );
    assert_eq!(
        serde_json::from_value::<PreferredFps>(json!(60)).unwrap(),
        PreferredFps::Hz(60)
    );
    assert!(serde_json::from_value::<PreferredFps>(json!(-1)).is_err());
    assert!(serde_json::from_value::<PreferredFps>(json!(true)).is_err());
    assert!(serde_json::from_value::<GpuPreference>(json!(3)).is_err());
}

#[test]
fn hardware_acceleration_and_color_space_names() {
    for (text, expected) in [
        ("default", HardwareAcceleration::Default),
        ("high-performance", HardwareAcceleration::HighPerformance),
        ("low-power", HardwareAcceleration::LowPower),
    ] {
        assert_eq!(
            serde_json::from_value::<HardwareAcceleration>(json!(text)).unwrap(),
            expected
        );
    }
    assert!(serde_json::from_value::<HardwareAcceleration>(json!("highPerformance")).is_err());
    for (text, expected) in [
        ("srgb", ColorSpace::Srgb),
        ("p3", ColorSpace::P3),
        ("rec2020", ColorSpace::Rec2020),
        ("linear", ColorSpace::Linear),
    ] {
        assert_eq!(
            serde_json::from_value::<ColorSpace>(json!(text)).unwrap(),
            expected
        );
    }
    assert!(serde_json::from_value::<ColorSpace>(Value::Null).is_err());
}

#[test]
fn window_config_builder_matches_deserialization() {
    let built = WindowConfig::builder()
        .title("T".to_string())
        .width(640)
        .resizable(false)
        .build();
    let parsed: WindowConfig =
        serde_json::from_value(json!({ "title": "T", "width": 640, "resizable": false })).unwrap();
    assert_eq!(built.title, parsed.title);
    assert_eq!(built.width, parsed.width);
    assert_eq!(built.resizable, parsed.resizable);
    assert!(built.height.is_none());
}

#[test]
fn window_event_call_shape() {
    use crate::runtime::{WindowEventKind, WINDOW_EVENT_FN};
    match BackendCall::window_event(WindowEventKind::Closed, "main") {
        BackendCall::Typed { id, fn_name, args } => {
            assert_eq!(id, WINDOW_EVENT_FN);
            assert_eq!(fn_name, WINDOW_EVENT_FN);
            assert_eq!(args, vec![json!("closed"), json!("main")]);
        },
        other => panic!("unexpected variant: {other:?}"),
    }
    assert_eq!(WindowEventKind::Created.as_str(), "created");
}

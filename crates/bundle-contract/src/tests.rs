use std::path::PathBuf;

use serde::{de::DeserializeOwned, Serialize};
use serde_json::json;

use super::*;

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) {
    let text = serde_json::to_string(value).expect("serialize");
    let back: T = serde_json::from_str(&text).expect("deserialize");
    assert_eq!(&back, value);
}

fn bundle_config() -> BundleConfig {
    BundleConfig {
        name: "Demo".into(),
        version: "1.2.3".into(),
        identifier: "dev.tynd.demo".into(),
        description: Some("short".into()),
        long_description: None,
        homepage: Some("https://example.com".into()),
        authors: Some(vec!["Ada".into()]),
        license: Some("Apache-2.0".into()),
        copyright: None,
        category: Some("Utility".into()),
        license_file: Some(PathBuf::from("/project/LICENSE")),
        targets: vec!["nsis".into(), "msi".into()],
        project_root: PathBuf::from("/project"),
        out_dir: PathBuf::from("/project/release"),
        binary_path: PathBuf::from("/project/release/demo.exe"),
        resources: vec![PathBuf::from("/project/assets")],
        file_associations: vec![FileAssociation {
            ext: vec!["demo".into()],
            content_types: Some(vec!["public.data".into()]),
            name: Some("Demo document".into()),
            description: None,
            role: FileAssociationRole::QlGenerator,
            mime_type: Some("application/x-demo".into()),
            rank: FileAssociationRank::Owner,
            exported_type: Some(ExportedFileType {
                identifier: "dev.tynd.demo.document".into(),
                conforms_to: Some(vec!["public.data".into()]),
            }),
        }],
        icons: vec![PathBuf::from("icon.svg")],
        linux: LinuxConfig {
            deb: DebConfig {
                depends: vec!["libgtk-3-0".into()],
                section: Some("utils".into()),
                ..DebConfig::default()
            },
            rpm: RpmConfig {
                depends: vec!["gtk3".into()],
                release: Some("2".into()),
                epoch: Some(1),
                desktop_template: Some(PathBuf::from("app.desktop.hbs")),
            },
            appimage: AppImageConfig {
                bundle_xdg_open: true,
            },
            pkgbuild: PkgbuildConfig {
                depends: vec!["libayatana-appindicator".into()],
            },
        },
        macos: MacOsConfig {
            url_schemes: vec!["demo".into()],
            minimum_system_version: Some("14.2".into()),
            entitlements: Some(PathBuf::from("app.entitlements")),
            info_plist: Some(PathBuf::from("Info.extra.plist")),
        },
        windows: WindowsConfig {
            webview_install_mode: Some(WebviewInstallMode::FixedRuntime {
                path: PathBuf::from("webview2"),
            }),
            minimum_webview2_version: Some("120.0.0.0".into()),
            allow_downgrades: Some(false),
            install_mode: Some(NsisInstallMode::PerMachine),
            msix: Some(MsixConfig {
                publisher: "CN=Demo".into(),
                publisher_display_name: None,
                min_version: Some("10.0.19041.0".into()),
                version: Some("1.2.3.0".into()),
            }),
            requested_execution_level: Some(ExecutionLevel::RequireAdministrator),
        },
        custom_nsis_template: None,
        custom_wix_template: Some("<Wix/>".into()),
    }
}

fn pack_config() -> PackConfig {
    PackConfig {
        host_binary: PathBuf::from("host.exe"),
        output: PathBuf::from("out/app.exe"),
        icon: Some(PathBuf::from("icon.png")),
        app_name: "Demo".into(),
        version: "1.2.3".into(),
        backend_bundle: PathBuf::from("backend.js"),
        frontend_dir: PathBuf::from("frontend"),
        sidecars: vec![SidecarSpec {
            name: "helper".into(),
            path: PathBuf::from("bin/helper"),
        }],
        bun_runtime: Some(PathBuf::from("bun")),
        bun_version: Some("1.3.14".into()),
        webview2_runtime_path: Some("webview2".into()),
        observability_json: Some("{}".into()),
        app_identifier: Some("com.example.demo".into()),
        debug: true,
        windows: WindowsPackOptions {
            embed_icon: false,
            patch_subsystem_gui: true,
            requested_execution_level: Some(ExecutionLevel::HighestAvailable),
        },
    }
}

#[test]
fn bundle_config_roundtrips() {
    roundtrip(&bundle_config());
}

#[test]
fn pack_config_roundtrips() {
    roundtrip(&pack_config());
    roundtrip(&PackConfig {
        debug: false,
        ..pack_config()
    });
}

#[test]
fn codesign_config_roundtrips() {
    for action in [CodesignAction::Sign, CodesignAction::Notarize] {
        roundtrip(&CodesignConfig {
            action,
            targets: vec![PathBuf::from("Demo.app")],
            identity: CodesignIdentity::P12 {
                path: PathBuf::from("cert.p12"),
                password_env: "P12_PASSWORD".into(),
            },
            notarization: Some(Notarization {
                api_key_path: PathBuf::from("key.json"),
            }),
            entitlements: None,
        });
    }
}

#[test]
fn webview_install_modes_roundtrip() {
    for mode in [
        WebviewInstallMode::Skip,
        WebviewInstallMode::DownloadBootstrapper { silent: false },
        WebviewInstallMode::EmbedBootstrapper { silent: true },
        WebviewInstallMode::OfflineInstaller { silent: true },
    ] {
        roundtrip(&mode);
    }
}

/// The field and variant names are the wire format: renaming one is a
/// deliberate contract change, not a refactor.
#[test]
fn wire_names_are_stable() {
    let pack = serde_json::to_value(PackConfig {
        debug: false,
        ..pack_config()
    })
    .expect("serialize");
    assert_eq!(pack["host_binary"], "host.exe");
    assert_eq!(pack["windows"]["patch_subsystem_gui"], true);
    assert!(pack.get("debug").is_none(), "debug is only sent when on");

    let bundle = serde_json::to_value(bundle_config()).expect("serialize");
    assert_eq!(bundle["file_associations"][0]["role"], "QLGenerator");
    assert_eq!(
        bundle["file_associations"][0]["mimeType"],
        "application/x-demo"
    );
    assert_eq!(
        bundle["windows"]["webview_install_mode"],
        json!({ "type": "fixedRuntime", "path": "webview2" })
    );

    let codesign = serde_json::to_value(CodesignConfig {
        action: CodesignAction::Notarize,
        targets: Vec::new(),
        identity: CodesignIdentity::P12 {
            path: PathBuf::from("cert.p12"),
            password_env: "P12_PASSWORD".into(),
        },
        notarization: None,
        entitlements: None,
    })
    .expect("serialize");
    assert_eq!(codesign["action"], CodesignAction::Notarize.as_str());
    assert_eq!(codesign["identity"]["kind"], "p12");
}

#[test]
fn optional_sections_take_their_defaults() {
    let pack: PackConfig = serde_json::from_value(json!({
        "host_binary": "host",
        "output": "out",
        "app_name": "Demo",
        "version": "1.0.0",
        "backend_bundle": "backend.js",
        "frontend_dir": "frontend",
        "windows": {}
    }))
    .expect("minimal pack config");
    assert!(!pack.debug);
    assert_eq!(pack.sidecars, Vec::new());
    assert!(pack.windows.embed_icon && pack.windows.patch_subsystem_gui);

    let mode: WebviewInstallMode =
        serde_json::from_value(json!({ "type": "embedBootstrapper" })).expect("install mode");
    assert_eq!(mode, WebviewInstallMode::EmbedBootstrapper { silent: true });
}

#[test]
fn unknown_fields_are_rejected() {
    let mut pack = serde_json::to_value(pack_config()).expect("serialize");
    pack["surprise"] = json!(1);
    assert!(serde_json::from_value::<PackConfig>(pack).is_err());

    let mut bundle = serde_json::to_value(bundle_config()).expect("serialize");
    bundle["windows"]["surprise"] = json!(1);
    assert!(serde_json::from_value::<BundleConfig>(bundle).is_err());
}

#[test]
fn windows_enums_use_camel_case_wire_names() {
    assert_eq!(
        serde_json::to_value(NsisInstallMode::CurrentUser).expect("serialize"),
        json!("currentUser")
    );
    assert_eq!(
        serde_json::to_value(ExecutionLevel::RequireAdministrator).expect("serialize"),
        json!("requireAdministrator")
    );
    assert_eq!(
        ExecutionLevel::RequireAdministrator.as_str(),
        "requireAdministrator"
    );
}

#[test]
fn safe_file_stem_replaces_path_and_shell_characters() {
    assert_eq!(safe_file_stem("My App"), "My-App");
    assert_eq!(safe_file_stem("a/b\\c:d"), "a-b-c-d");
    assert_eq!(safe_file_stem("v1.2_beta-x"), "v1.2_beta-x");
}

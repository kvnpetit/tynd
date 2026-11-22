use std::{fs, path::Path};

use serde_json::json;

use super::{scratch_package_json, typegen_workspace_manifest};

#[test]
fn typegen_manifest_strips_the_verbatim_prefix() {
    let manifest = typegen_workspace_manifest(Path::new(
        r"\\?\C:\work\app\node_modules\@tynd\plugin-sdk\native",
    ))
    .expect("manifest");
    assert!(
        manifest.contains(
            r#"tynd-plugin-sdk = { path = "C:/work/app/node_modules/@tynd/plugin-sdk/native" }"#
        ),
        "{manifest}"
    );
    assert!(!manifest.contains("//?/"), "{manifest}");
    toml::from_str::<toml::Table>(&manifest).expect("valid TOML");
}

#[test]
fn scratch_app_reuses_the_plugins_local_and_pinned_tynd_packages() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plugin = dir.path().join("plugin");
    let core = dir.path().join("tynd/packages/core");
    fs::create_dir_all(&plugin).expect("plugin dir");
    fs::create_dir_all(&core).expect("core dir");
    let plugin = dunce::canonicalize(&plugin).expect("canonical plugin");
    let core = dunce::canonicalize(&core).expect("canonical core");
    let manifest = json!({
        "peerDependencies": { "@tynd/core": "^1.0.0", "@tynd/host-src": "1.2.3" },
        "devDependencies": { "@tynd/core": "file:../tynd/packages/core", "@tynd/cli": "^1.0.0" }
    });

    let package = scratch_package_json("@acme/plugin-x", &plugin, &manifest);

    let portable = |path: &Path| crate::fsx::portable_path(&path.to_string_lossy());
    assert_eq!(
        package["dependencies"]["@acme/plugin-x"],
        format!("link:{}", portable(&plugin))
    );
    assert_eq!(
        package["dependencies"]["@tynd/core"],
        format!("file:{}", portable(&core))
    );
    assert_eq!(package["devDependencies"]["@tynd/host-src"], "1.2.3");
    // A range is not the plugin's own build: the CLI's range applies.
    assert_eq!(
        package["devDependencies"]["@tynd/cli"],
        format!("^{}", crate::VERSION)
    );
}

#[test]
fn scratch_app_resolves_workspace_packages_to_their_install() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plugin = dir.path().join("plugin");
    let installed = plugin.join("node_modules/@tynd/core");
    fs::create_dir_all(&installed).expect("installed dir");
    fs::write(installed.join("package.json"), "{}").expect("package.json");
    let plugin = dunce::canonicalize(&plugin).expect("canonical plugin");
    let manifest = json!({ "devDependencies": { "@tynd/core": "workspace:*" } });

    let package = scratch_package_json("plugin-x", &plugin, &manifest);

    let installed = dunce::canonicalize(plugin.join("node_modules/@tynd/core")).expect("core");
    assert_eq!(
        package["dependencies"]["@tynd/core"],
        format!(
            "file:{}",
            crate::fsx::portable_path(&installed.to_string_lossy())
        )
    );
}

#[test]
fn scratch_app_copies_overrides_and_resolutions_with_local_paths_resolved() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plugin = dir.path().join("plugin");
    let tooling = dir.path().join("tynd/npm/tooling");
    fs::create_dir_all(&plugin).expect("plugin dir");
    fs::create_dir_all(&tooling).expect("tooling dir");
    let plugin = dunce::canonicalize(&plugin).expect("canonical plugin");
    let tooling = dunce::canonicalize(&tooling).expect("canonical tooling");
    let manifest = json!({
        "overrides": {
            "@tynd/tooling": "file:../tynd/npm/tooling",
            "foo": { "bar": "link:../tynd/npm/tooling" },
            "baz": "1.2.3"
        },
        "resolutions": { "@tynd/tooling": "file:../tynd/npm/tooling" }
    });

    let package = scratch_package_json("plugin-x", &plugin, &manifest);

    let local = format!(
        "file:{}",
        crate::fsx::portable_path(&tooling.to_string_lossy())
    );
    assert_eq!(package["overrides"]["@tynd/tooling"], local);
    assert_eq!(
        package["overrides"]["foo"]["bar"],
        format!(
            "link:{}",
            crate::fsx::portable_path(&tooling.to_string_lossy())
        )
    );
    assert_eq!(package["overrides"]["baz"], "1.2.3");
    assert_eq!(package["resolutions"]["@tynd/tooling"], local);
}

#[test]
fn scratch_app_without_overrides_has_none() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package = scratch_package_json("plugin-x", dir.path(), &json!({}));
    assert!(package.get("overrides").is_none());
    assert!(package.get("resolutions").is_none());
}

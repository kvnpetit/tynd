use serde_json::json;

use super::*;

#[test]
fn native_path_accepts_current_directory_prefixes_only_inside_the_package() {
    for valid in ["native", "./native", "crates/native", "./crates/./native"] {
        assert!(valid_native_relative_path(valid), "{valid}");
    }
    for invalid in [
        "",
        ".",
        "./",
        "./.",
        "../native",
        "native/../..",
        "/native",
        "C:native",
    ] {
        assert!(!valid_native_relative_path(invalid), "{invalid}");
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let package = dir.path().join("plugin");
    fs::create_dir_all(package.join("native")).expect("create native crate");
    fs::write(package.join("native/Cargo.toml"), "").expect("write manifest");
    let native = json!({ "path": "./native" });
    let resolved = resolve_native_dir(
        "acme-plugin",
        &package,
        native.as_object().expect("native object"),
    )
    .expect("./native resolves");
    assert_eq!(
        resolved,
        fs::canonicalize(package.join("native")).expect("canonical native")
    );
}

#[test]
fn plugin_crate_names_are_cargo_packages_mapped_to_rust_idents() {
    assert_eq!(
        default_plugin_crate_name("@acme/plugin.foo"),
        "acme-plugin-foo"
    );
    assert!(is_cargo_package_name("tynd-plugin-screen-rec"));
    assert!(is_cargo_package_name("legacy_name"));
    for invalid in ["", "-lead", "9lives", "Upper", "crate", "a b"] {
        assert!(!is_cargo_package_name(invalid), "{invalid}");
    }
    assert_eq!(
        crate_ident("tynd-plugin-screen-rec"),
        "tynd_plugin_screen_rec"
    );
}

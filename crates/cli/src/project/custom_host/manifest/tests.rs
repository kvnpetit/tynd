use std::path::PathBuf;

use serde_json::{json, Value};

use super::*;
use crate::project::custom_host::{
    plugin_copy::cache_native_plugins,
    test_support::{native_plugin, parse, plugin_crate_at, sources},
};

fn manifest_for(config: &TyndConfig, apis: &[&str]) -> String {
    let apis = apis.iter().map(|api| (*api).to_owned()).collect();
    render_custom_cargo_manifest(
        "demo",
        "demo_0123abcd",
        &sources(),
        &apis,
        &[],
        &observability_host_features(config),
        true,
    )
}

fn host_features_line(manifest: &str) -> &str {
    manifest
        .lines()
        .find(|line| line.starts_with("tynd-host "))
        .expect("tynd-host dependency line")
}

#[test]
fn no_observability_adds_no_telemetry_features() {
    let config = parse(json!({}));
    assert_eq!(observability_host_features(&config), [] as [String; 0]);
    let manifest = manifest_for(&config, &["acl", "fs"]);
    let line = host_features_line(&manifest);
    assert!(!line.contains("obs-"), "{line}");
    assert!(line.contains("\"api-fs\""));
}

#[test]
fn core_apis_map_to_no_host_feature() {
    let manifest = manifest_for(&parse(json!({})), &["acl", "fs", "log"]);
    let line = host_features_line(&manifest);
    assert!(line.contains("\"api-fs\""), "{line}");
    for core in CORE_HOST_APIS {
        assert!(!line.contains(&format!("\"api-{core}\"")), "{line}");
    }
    assert!(!line.contains("api-core"), "{line}");
}

#[test]
fn sentry_and_otel_enable_their_own_features() {
    let sentry =
        parse(json!({ "observability": { "sentry": { "dsn": "https://k@o.example.com/1" } } }));
    assert_eq!(observability_host_features(&sentry), ["obs-sentry"]);
    let otel =
        parse(json!({ "observability": { "otel": { "endpoint": "http://localhost:4317" } } }));
    assert_eq!(observability_host_features(&otel), ["obs-otel"]);
    let both = parse(json!({ "observability": {
        "sentry": { "dsn": "https://k@o.example.com/1" },
        "otel": { "endpoint": "http://localhost:4317" }
    }}));
    let manifest = manifest_for(&both, &["acl"]);
    let line = host_features_line(&manifest);
    assert!(
        line.contains("\"obs-sentry\"") && line.contains("\"obs-otel\""),
        "{line}"
    );
}

#[test]
fn blank_pipelines_do_not_pull_telemetry_crates() {
    let config = parse(json!({ "observability": {
        "sentry": { "dsn": "" }, "otel": { "endpoint": " " }
    }}));
    assert_eq!(observability_host_features(&config), [] as [String; 0]);
}

#[test]
fn manifest_rendering_is_deterministic() {
    let config =
        parse(json!({ "observability": { "sentry": { "dsn": "https://k@o.example.com/1" } } }));
    assert_eq!(
        manifest_for(&config, &["acl", "menu", "tray", "multiwindow"]),
        manifest_for(&config, &["multiwindow", "tray", "menu", "acl"]),
    );
}

#[test]
fn ui_apis_map_to_their_cargo_features() {
    let manifest = manifest_for(&parse(json!({})), &["menu", "tray", "multiwindow"]);
    let line = host_features_line(&manifest);
    for feature in ["api-menu", "api-tray", "api-multiwindow"] {
        assert!(line.contains(&format!("\"{feature}\"")), "{line}");
    }
}

const ROOT_MANIFEST: &str = include_str!("../../../../../../Cargo.toml");

fn root_workspace_table(key: &str) -> toml::Table {
    let root: toml::Table = toml::from_str(ROOT_MANIFEST).expect("root Cargo.toml parses");
    root["workspace"][key]
        .as_table()
        .unwrap_or_else(|| panic!("root Cargo.toml has [workspace.{key}]"))
        .clone()
}

fn parse_entry(key: &str, value: &str) -> toml::Value {
    let table: toml::Table =
        toml::from_str(&format!("{key} = {value}")).expect("template entry parses");
    table[key].clone()
}

#[test]
fn workspace_dependencies_match_root_manifest() {
    let root = root_workspace_table("dependencies");
    for (name, value) in WORKSPACE_DEPENDENCIES {
        let expected = root
            .get(*name)
            .unwrap_or_else(|| panic!("{name} is missing from root [workspace.dependencies]"));
        assert_eq!(
            &parse_entry(name, value),
            expected,
            "{name} drifted from root"
        );
    }
}

#[test]
fn workspace_package_matches_root_manifest() {
    let root = root_workspace_table("package");
    let template: toml::Table = WORKSPACE_PACKAGE
        .iter()
        .map(|(key, value)| ((*key).to_owned(), parse_entry(key, value)))
        .collect();
    assert_eq!(template, root);
}

#[test]
fn workspace_covers_everything_official_plugins_inherit() {
    let plugins_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/standalone");
    let dependencies: BTreeSet<&str> = WORKSPACE_DEPENDENCIES
        .iter()
        .map(|(name, _)| *name)
        .collect();
    let package: BTreeSet<&str> = WORKSPACE_PACKAGE.iter().map(|(key, _)| *key).collect();
    let mut checked = 0;
    for entry in fs::read_dir(&plugins_dir).expect("plugins/standalone is readable") {
        let manifest_path = entry
            .expect("plugin dir entry")
            .path()
            .join("native/Cargo.toml");
        let Ok(source) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let manifest: toml::Table = toml::from_str(&source).expect("plugin Cargo.toml parses");
        checked += 1;
        let inherits = |value: &toml::Value| {
            value.get("workspace").and_then(toml::Value::as_bool) == Some(true)
        };
        for (key, value) in manifest["package"].as_table().expect("[package]") {
            if inherits(value) {
                assert!(
                    package.contains(key.as_str()),
                    "{}: inherits package.{key}",
                    manifest_path.display()
                );
            }
        }
        for section in ["dependencies", "build-dependencies"] {
            let Some(table) = manifest.get(section).and_then(toml::Value::as_table) else {
                continue;
            };
            for (name, value) in table {
                if inherits(value) && name != "tynd-plugin-sdk" {
                    assert!(
                        dependencies.contains(name.as_str()),
                        "{}: inherits {name}",
                        manifest_path.display()
                    );
                }
            }
        }
    }
    assert!(checked > 0, "no official plugin manifests found");
}

#[test]
fn generated_manifest_is_a_workspace_of_the_app_alone() {
    let manifest = manifest_for(&parse(json!({})), &["acl"]);
    let parsed: toml::Table = toml::from_str(&manifest).expect("generated manifest parses");
    let workspace = parsed["workspace"].as_table().expect("[workspace]");
    // The root package is the only member. `members = ["."]` would cancel
    // `exclude`, and the editable plugin copies would join this workspace.
    assert!(workspace.get("members").is_none());
    assert_eq!(
        workspace["exclude"].as_array().expect("exclude"),
        &[toml::Value::from(LOCAL_PLUGINS_DIR)]
    );
    // What plugin crates inherit lives in their own workspace; cargo warns
    // about workspace tables no member uses.
    for table in ["package", "lints", "dependencies"] {
        assert!(workspace.get(table).is_none(), "[workspace.{table}]");
    }
    assert!(parsed.get("lints").is_none());
    assert_eq!(parsed["package"]["name"].as_str(), Some("demo"));
    assert_eq!(parsed["bin"][0]["name"].as_str(), Some("demo_0123abcd"));
}

#[test]
fn plugin_workspace_is_valid_toml_without_framework_lint_policy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plugin = native_plugin(dir.path(), "");
    let manifest = render_plugin_workspace_manifest(&plugin, &sources().plugin_sdk)
        .expect("workspace manifest renders");
    let parsed: toml::Table = toml::from_str(&manifest).expect("workspace manifest parses");
    let workspace = parsed["workspace"].as_table().expect("[workspace]");
    assert_eq!(
        workspace["members"].as_array().expect("members"),
        &[toml::Value::from(PLUGIN_COPY_CRATE_DIR)]
    );
    assert!(workspace["lints"]
        .as_table()
        .expect("[workspace.lints]")
        .is_empty());
    assert!(workspace["package"].get("readme").is_none());
    assert_eq!(workspace["package"]["edition"].as_str(), Some("2021"));
    // The plugin inherits no dependency, and cargo warns about unused ones.
    assert!(workspace["dependencies"]
        .as_table()
        .expect("[workspace.dependencies]")
        .is_empty());
    // A virtual workspace: the plugin crate is the package.
    assert!(parsed.get("package").is_none());
}

#[test]
fn plugin_workspace_declares_only_the_dependencies_the_plugin_inherits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plugin = native_plugin(dir.path(), "");
    fs::write(
        plugin.native_dir.join("Cargo.toml"),
        r#"[package]
name = "acme-plugin"
version.workspace = true

[dependencies]
tynd-plugin-sdk.workspace = true
serde_json = { workspace = true }
tracing = "0.1"

[target.'cfg(windows)'.dependencies]
parking_lot.workspace = true

[build-dependencies]
base64.workspace = true
"#,
    )
    .expect("write plugin manifest");
    let manifest = render_plugin_workspace_manifest(&plugin, &sources().plugin_sdk)
        .expect("workspace manifest renders");
    let parsed: toml::Table = toml::from_str(&manifest).expect("workspace manifest parses");
    let dependencies = parsed["workspace"]["dependencies"]
        .as_table()
        .expect("[workspace.dependencies]");
    let names: Vec<&str> = dependencies.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        ["base64", "parking_lot", "serde_json", "tynd-plugin-sdk"]
    );
    assert_eq!(
        dependencies["serde_json"],
        root_workspace_table("dependencies")["serde_json"]
    );
    assert_eq!(
        dependencies["tynd-plugin-sdk"]["path"].as_str(),
        Some("/src/crates/plugin-sdk")
    );
}

#[test]
fn nested_plugin_crates_contribute_inherited_workspace_dependencies() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plugin = native_plugin(dir.path(), "");
    fs::write(
        plugin.native_dir.join("Cargo.toml"),
        "[package]
name = \"acme-plugin\"

[dependencies]
tynd-plugin-sdk.workspace = true
",
    )
    .expect("write plugin manifest");
    let nested = plugin.native_dir.join("crates/core");
    fs::create_dir_all(&nested).expect("create nested crate");
    fs::write(
        nested.join("Cargo.toml"),
        "[package]
name = \"acme_core\"

[dependencies]
serde.workspace = true
",
    )
    .expect("write nested manifest");
    let ignored = plugin.native_dir.join("target/package/stale");
    fs::create_dir_all(&ignored).expect("create stale crate");
    fs::write(
        ignored.join("Cargo.toml"),
        "[package]
name = \"stale\"

[dependencies]
dashmap.workspace = true
",
    )
    .expect("write stale manifest");
    let inherited = inherited_workspace_dependencies(&[plugin]).expect("scan plugin");
    assert!(inherited.contains("serde"), "{inherited:?}");
    assert!(inherited.contains("tynd-plugin-sdk"), "{inherited:?}");
    assert!(!inherited.contains("dashmap"), "{inherited:?}");
}

#[test]
fn unparsable_plugin_manifests_are_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plugin = native_plugin(dir.path(), "");
    fs::write(
        plugin.native_dir.join("Cargo.toml"),
        "[package]
name = \"acme-plugin\"

[dependencies]
serde.workspace = true
",
    )
    .expect("write plugin manifest");
    let template = plugin.native_dir.join("templates/crate");
    fs::create_dir_all(&template).expect("create template crate");
    fs::write(template.join("Cargo.toml"), "[package\nname = {{name}}\n")
        .expect("write template manifest");
    let inherited = inherited_workspace_dependencies(&[plugin]).expect("scan plugin");
    assert!(inherited.contains("serde"), "{inherited:?}");
}

/// A dependency-free crate named `name` at `dir`, a workspace of its own,
/// declaring `features`.
fn stub_crate(dir: &Path, name: &str, features: &[toml::Value]) {
    let features = features
        .iter()
        .map(|feature| feature.as_str().expect("feature name").to_owned() + " = []\n")
        .collect::<String>();
    fs::create_dir_all(dir.join("src")).expect("create stub crate");
    fs::write(dir.join("src/lib.rs"), "").expect("write stub source");
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[workspace]

[package]
name = \"{name}\"
version = \"0.0.0\"
edition = \"2021\"

[features]
{features}"
        ),
    )
    .expect("write stub manifest");
}

/// Cargo must load the generated workspace when an editable plugin copy,
/// which is inside `.tynd/build`, inherits from its own generated workspace:
/// cargo used to resolve `edition.workspace = true` against the app workspace
/// (`workspace.package.edition` was not defined).
#[test]
fn cargo_loads_the_workspace_with_an_editable_plugin_inheriting_from_its_own() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let build = root.join(".tynd/build");
    fs::create_dir_all(build.join("src")).expect("create build directory");
    fs::write(build.join("src/main.rs"), "fn main() {}").expect("write main");
    let mut plugins = [plugin_crate_at(&root.join("acme-plugin/native"), "")];
    fs::write(
        plugins[0].native_dir.join("Cargo.toml"),
        "[package]
name = \"acme-plugin\"
version.workspace = true
edition.workspace = true
license.workspace = true

[lints]
workspace = true
",
    )
    .expect("write plugin manifest");
    let sources = RustSourcePaths {
        runtime: root.join("src/runtime"),
        host: root.join("src/host"),
        plugin_sdk: root.join("src/plugin-sdk"),
    };
    cache_native_plugins(
        &root.join("cache"),
        &build,
        &mut plugins,
        &sources.plugin_sdk,
    )
    .expect("copy plugin");
    assert!(plugins[0].crate_dir.starts_with(&build));

    let manifest = render_custom_cargo_manifest(
        "demo",
        "demo_0123abcd",
        &sources,
        &BTreeSet::new(),
        &plugins,
        &[],
        false,
    );
    fs::write(build.join("Cargo.toml"), &manifest).expect("write app manifest");
    let parsed: toml::Table = toml::from_str(&manifest).expect("generated manifest parses");
    for (name, path) in [
        ("tynd-runtime", &sources.runtime),
        ("tynd-host", &sources.host),
    ] {
        // The runtime dependency carries no features.
        let features = parsed["dependencies"][name]
            .get("features")
            .and_then(toml::Value::as_array)
            .cloned()
            .unwrap_or_default();
        stub_crate(path, name, &features);
    }

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = std::process::Command::new(cargo)
        .args(["metadata", "--offline", "--format-version", "1"])
        .arg("--manifest-path")
        .arg(build.join("Cargo.toml"))
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("cargo metadata JSON");
    let plugin = metadata["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .find(|package| package["name"] == "acme-plugin")
        .expect("plugin package");
    // Inherited from the plugin's own workspace.
    assert_eq!(plugin["edition"], "2021");
    assert_eq!(plugin["license"], "Apache-2.0");
    assert_eq!(
        metadata["workspace_members"]
            .as_array()
            .expect("workspace members")
            .len(),
        1
    );
}

#[test]
fn custom_main_passes_owned_permission_strings() {
    let plugin = NativeBuildPlugin {
        package: "acme-plugin".to_owned(),
        crate_name: "acme-plugin".to_owned(),
        crate_dir: PathBuf::from("/plugins/acme"),
        native_dir: PathBuf::from("/plugins/acme"),
        features: Vec::new(),
        permissions: vec!["fs:*".to_owned(), "http:get".to_owned()],
        required_apis: Vec::new(),
    };
    let main = render_custom_main(&[plugin]).expect("render main.rs");
    // `register_with_permissions` takes `Option<Vec<String>>`: a `&str` vec
    // would not compile.
    assert!(
        main.contains("            Some(vec![\"fs:*\".to_owned(), \"http:get\".to_owned()]),\n"),
        "{main}"
    );
    assert!(main.contains("acme_plugin::__tynd_plugin(),"), "{main}");
    assert!(main.ends_with("    tynd_runtime::run();\n}\n"), "{main}");
}

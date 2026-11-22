use std::{fs, path::Path};

use serde_json::Value;

use super::{add, project, remove};
use crate::args::{AddArgs, RemoveArgs};

fn add_offline(cwd: &Path, inputs: &[&str]) {
    add(AddArgs {
        inputs: inputs.iter().map(|input| (*input).to_owned()).collect(),
        cwd: cwd.to_path_buf(),
        no_install: true,
        apis: None,
    })
    .expect("tynd add");
}

fn read_config(cwd: &Path) -> Value {
    let source = fs::read_to_string(cwd.join("tynd.config.json")).expect("read config");
    serde_json::from_str(&source).expect("parse config")
}

fn plugin_packages(config: &Value) -> Vec<String> {
    config["plugins"]
        .as_array()
        .map(|plugins| {
            plugins
                .iter()
                .filter_map(|entry| entry.as_str().or_else(|| entry["pkg"].as_str()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn plugin_first_capabilities_map_to_their_package() {
    assert_eq!(
        project::capability_plugin_package("filesystem"),
        Some("@tynd/plugin-fs")
    );
    assert_eq!(
        project::capability_plugin_package("HTTP"),
        Some("@tynd/plugin-http")
    );
    assert_eq!(
        project::capability_plugin_package("clipboard"),
        Some("@tynd/plugin-clipboard")
    );
    assert_eq!(project::capability_plugin_package("log"), None);
    assert_eq!(project::capability_plugin_package("@acme/plugin-x"), None);
}

#[test]
fn catalog_aliases_resolve_to_their_plugin() {
    for (alias, package) in [
        ("kv", "@tynd/plugin-store"),
        ("copy", "@tynd/plugin-clipboard"),
        ("open", "@tynd/plugin-shell"),
        ("Hotkeys", "@tynd/plugin-shortcuts"),
        ("db", "@tynd/plugin-sql"),
        ("sql", "@tynd/plugin-sql"),
        ("server", "@tynd/plugin-http-server"),
    ] {
        assert!(
            project::is_capability_alias(alias),
            "{alias} is a catalog name"
        );
        assert_eq!(
            project::capability_plugin_package(alias),
            Some(package),
            "{alias}"
        );
    }
    // Every advertised catalog name is accepted, never left to `bun add`.
    for (name, package, _) in crate::config::PLUGIN_ADD_NAMES {
        assert!(project::is_capability_alias(name), "{name}");
        assert_eq!(project::capability_plugin_package(name), Some(*package));
    }
    assert!(!project::is_capability_alias("left-pad"));
    assert!(!project::is_capability_alias("@acme/plugin-x"));
}

/// The repository root, where plugins/catalog.json and the docs live.
fn repository_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn catalog_entries() -> Vec<Value> {
    let source =
        fs::read_to_string(repository_root().join("plugins/catalog.json")).expect("read catalog");
    serde_json::from_str(&source).expect("parse catalog")
}

#[test]
fn catalog_docs_urls_map_to_existing_pages() {
    const SITE: &str = "https://tynd-apps.github.io/tynd/docs/";
    let docs = repository_root().join("apps/docs/src/content/docs/docs");
    for entry in catalog_entries() {
        let url = entry["docs"].as_str().expect("catalog entry has docs");
        let page = url
            .strip_prefix(SITE)
            .and_then(|page| page.strip_suffix('/'))
            .unwrap_or_else(|| panic!("{url} is not a docs page URL"));
        assert!(
            docs.join(format!("{page}.mdx")).is_file()
                || docs.join(page).join("index.mdx").is_file(),
            "{url} has no page under {}",
            docs.display()
        );
    }
}

/// `distTag` of each catalog entry equals the `publishConfig.tag` its package
/// is published with, so `tynd add` installs a version that exists.
#[test]
fn catalog_dist_tags_match_the_published_tags() {
    let mut published = std::collections::BTreeMap::new();
    for family in ["plugins/host", "plugins/standalone"] {
        for directory in fs::read_dir(repository_root().join(family)).expect("read plugins") {
            let manifest = directory.expect("plugin entry").path().join("package.json");
            let Ok(source) = fs::read_to_string(&manifest) else {
                continue;
            };
            let manifest: Value = serde_json::from_str(&source).expect("parse package.json");
            if let Some(name) = manifest["name"].as_str() {
                let tag = manifest
                    .pointer("/publishConfig/tag")
                    .and_then(Value::as_str)
                    .filter(|tag| *tag != "latest")
                    .map(str::to_owned);
                published.insert(name.to_owned(), tag);
            }
        }
    }
    let mut tagged = Vec::new();
    for entry in catalog_entries() {
        let package = entry["pkg"].as_str().expect("catalog entry has pkg");
        let tag = published
            .get(package)
            .unwrap_or_else(|| panic!("{package} has no package under plugins/"));
        assert_eq!(
            entry.get("distTag").and_then(Value::as_str),
            tag.as_deref(),
            "{package}"
        );
        if let Some(tag) = tag {
            tagged.push((package.to_owned(), tag.clone()));
        }
    }
    tagged.sort();
    let generated: Vec<(String, String)> = crate::config::PLUGIN_DIST_TAGS
        .iter()
        .map(|(package, tag)| ((*package).to_owned(), (*tag).to_owned()))
        .collect();
    assert_eq!(generated, tagged, "run `bun scripts/gen-api-catalog.ts`");
}

#[test]
fn adding_a_plugin_first_api_also_adds_its_plugin() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("tynd.config.json"), "{}\n").expect("write config");

    add_offline(dir.path(), &["fs"]);

    let config = read_config(dir.path());
    assert_eq!(config["apis"], serde_json::json!(["fs"]));
    assert_eq!(plugin_packages(&config), vec!["@tynd/plugin-fs"]);

    // Re-running is idempotent.
    add_offline(dir.path(), &["filesystem"]);
    let config = read_config(dir.path());
    assert_eq!(config["apis"], serde_json::json!(["fs"]));
    assert_eq!(plugin_packages(&config), vec!["@tynd/plugin-fs"]);
}

#[test]
fn adding_catalog_aliases_declares_the_catalog_plugin() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("tynd.config.json"),
        "{}
",
    )
    .expect("write config");

    add_offline(dir.path(), &["kv", "db"]);

    let config = read_config(dir.path());
    assert_eq!(config["apis"], serde_json::json!(["store"]));
    assert_eq!(
        plugin_packages(&config),
        vec!["@tynd/plugin-store", "@tynd/plugin-sql"]
    );
}

#[test]
fn adding_a_core_capability_adds_no_plugin() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("tynd.config.json"), "{}\n").expect("write config");

    add_offline(dir.path(), &["log"]);

    let config = read_config(dir.path());
    assert_eq!(config["apis"], serde_json::json!(["log"]));
    assert_eq!(plugin_packages(&config), Vec::<String>::new());
}

#[test]
fn adding_a_former_preset_api_adds_its_system_plugin() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("tynd.config.json"), "{}\n").expect("write config");

    add_offline(dir.path(), &["clipboard"]);

    let config = read_config(dir.path());
    assert_eq!(config["apis"], serde_json::json!(["clipboard"]));
    assert_eq!(plugin_packages(&config), vec!["@tynd/plugin-clipboard"]);
}

fn add_offline_fails(cwd: &Path, inputs: &[&str]) {
    add(AddArgs {
        inputs: inputs.iter().map(|input| (*input).to_owned()).collect(),
        cwd: cwd.to_path_buf(),
        no_install: true,
        apis: None,
    })
    .expect_err("tynd add must fail");
}

#[test]
fn a_failing_input_rolls_back_the_whole_add() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("tynd.config.json"), "{}\n").expect("write config");

    add_offline_fails(dir.path(), &["fs", "../not-a-package"]);

    assert_eq!(
        fs::read_to_string(dir.path().join("tynd.config.json")).expect("read config"),
        "{}\n"
    );
}

const VITE_PACKAGE: &str = "{ \"name\": \"app\", \"devDependencies\": { \"vite\": \"^6.0.0\" } }\n";

fn vite_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("package.json"), VITE_PACKAGE).expect("write package.json");
    fs::write(dir.path().join("vite.config.ts"), "export default {};\n").expect("write vite");
    fs::write(dir.path().join("index.html"), "<html></html>\n").expect("write html");
    dir
}

#[test]
fn a_failed_add_removes_what_init_scaffolded() {
    // `fs` runs `tynd init` (no config yet) before the bad input fails.
    let dir = vite_project();
    let package = VITE_PACKAGE;

    add_offline_fails(dir.path(), &["fs", "../not-a-package"]);

    let mut names: Vec<String> = fs::read_dir(dir.path())
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(names, ["index.html", "package.json", "vite.config.ts"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).expect("read package.json"),
        package
    );
}

fn remove_offline(cwd: &Path, inputs: &[&str]) -> anyhow::Result<()> {
    remove(&RemoveArgs {
        inputs: inputs.iter().map(|input| (*input).to_owned()).collect(),
        cwd: cwd.to_path_buf(),
        no_uninstall: true,
    })
}

#[test]
fn remove_undoes_add_for_host_and_plugin_capabilities() {
    for format in ["json", "ts", "jsonc", "toml", "yaml"] {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(format!("tynd.config.{format}"));
        let original = match format {
            "json" | "jsonc" => "{}\n",
            "ts" => "export default {};\n",
            "toml" => "frontendDir = \"dist\"\n",
            _ => "frontendDir: dist\n",
        };
        fs::write(&path, original).expect("write config");

        add_offline(dir.path(), &["log", "fs", "db"]);
        let added = fs::read_to_string(&path).expect("read config");
        assert!(
            added.contains("log") && added.contains("@tynd/plugin-fs"),
            "{format}: {added}"
        );

        remove_offline(dir.path(), &["log", "filesystem", "db"]).expect("tynd remove");
        let removed = fs::read_to_string(&path).expect("read config");
        for gone in ["\"log\"", "\"fs\"", "@tynd/plugin-fs", "@tynd/plugin-sql"] {
            assert!(
                !removed.contains(gone),
                "{format}: {gone} left in {removed}"
            );
        }
        // The emptied `apis` and `plugins` lists that add created are dropped.
        assert_eq!(removed, original, "{format}");
    }
}

#[test]
fn removing_a_core_capability_keeps_other_capabilities() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("tynd.config.json"), "{}\n").expect("write config");
    add_offline(dir.path(), &["log", "dialog"]);

    remove_offline(dir.path(), &["log"]).expect("tynd remove");

    let config = read_config(dir.path());
    assert_eq!(config["apis"], serde_json::json!(["dialog"]));
    assert_eq!(plugin_packages(&config), vec!["@tynd/plugin-dialog"]);
}

#[test]
fn a_failing_input_rolls_back_the_whole_remove() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("tynd.config.json"), "{}\n").expect("write config");
    add_offline(dir.path(), &["log", "fs"]);
    let before = fs::read_to_string(dir.path().join("tynd.config.json")).expect("read config");

    remove_offline(dir.path(), &["log", "fs", "../not-a-package"])
        .expect_err("tynd remove must fail");

    assert_eq!(
        fs::read_to_string(dir.path().join("tynd.config.json")).expect("read config"),
        before
    );
}

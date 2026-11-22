use serde_json::json;

use super::*;

fn config(value: serde_json::Value) -> TyndConfig {
    serde_json::from_value(value).expect("config shape should deserialize")
}

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, body) in files {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, body).expect("write");
    }
    dir
}

fn warnings(config: &TyndConfig, root: &Path) -> Vec<String> {
    plugin_import_warnings(config, root)
}

#[test]
fn warns_on_plugin_package_import_without_declaration() {
    let dir = project(&[(
        "ui/main.tsx",
        "import { vault } from \"@tynd/plugin-vault/client\";\n",
    )]);
    let warnings = warnings(&config(json!({})), dir.path());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("ui/main.tsx"));
}

#[test]
fn former_preset_apis_warn_through_their_system_plugin() {
    let dir = project(&[(
        "src/app.ts",
        "import { dialog } from \"@tynd/plugin-dialog/client\";\n\
             import { shell } from \"@tynd/plugin-shell/client\";\n\
             import { singleInstance } from \"@tynd/plugin-single-instance/client\";\n",
    )]);
    let undeclared = warnings(&config(json!({})), dir.path());
    assert_eq!(undeclared.len(), 3);
    for package in [
        "@tynd/plugin-dialog",
        "@tynd/plugin-shell",
        "@tynd/plugin-single-instance",
    ] {
        assert!(undeclared.iter().any(|warning| warning.contains(package)));
    }
    let declared = config(json!({ "plugins": [
            "@tynd/plugin-dialog", "@tynd/plugin-shell", "@tynd/plugin-single-instance"
        ] }));
    assert_eq!(warnings(&declared, dir.path()), [] as [String; 0]);
}

#[test]
fn type_only_imports_do_not_count() {
    let dir = project(&[(
        "src/types.ts",
        "import type { VaultApi } from \"@tynd/plugin-vault/client\";\n\
             import type {\n  Dialog,\n  Options,\n} from '@tynd/plugin-dialog';\n\
             export type { Shell } from \"@tynd/plugin-shell/client\";\n\
             export type * from \"@tynd/plugin-tray\";\n",
    )]);
    assert_eq!(warnings(&config(json!({})), dir.path()), [] as [String; 0]);

    let mixed = project(&[(
        "src/app.ts",
        "import type { VaultApi } from \"@tynd/plugin-vault/client\";\n\
             import { vault } from \"@tynd/plugin-vault/client\";\n\
             export { shell } from \"@tynd/plugin-shell/client\";\n",
    )]);
    assert_eq!(warnings(&config(json!({})), mixed.path()).len(), 2);
}

#[test]
fn non_import_mentions_do_not_count() {
    let dir = project(&[
        (
            "src/app.ts",
            "// import { fs } from \"@tynd/plugin-fs/client\";\n\
                 /* import { vault } from '@tynd/plugin-vault'; */\n\
                 const help = `import { shell } from \"@tynd/plugin-shell\"`;\n\
                 const label = \"@tynd/plugin-tray\";\n",
        ),
        (
            "src/app.test.ts",
            "vi.mock(\"@tynd/plugin-fs/client\", () => ({}));\n",
        ),
        (
            "vite.config.ts",
            "export default { optimizeDeps: { exclude: [\"@tynd/plugin-fs\"] } };\n",
        ),
    ]);
    assert_eq!(warnings(&config(json!({})), dir.path()), [] as [String; 0]);
}

#[test]
fn inline_type_members_count_only_when_all_are_types() {
    let types = project(&[(
        "src/types.ts",
        "import { type VaultApi, type Entry as E } from \"@tynd/plugin-vault/client\";\n\
             export { type Shell } from \"@tynd/plugin-shell\";\n",
    )]);
    assert_eq!(
        warnings(&config(json!({})), types.path()),
        [] as [String; 0]
    );

    let mixed = project(&[(
        "src/app.ts",
        "import { type VaultApi, vault } from \"@tynd/plugin-vault/client\";\n",
    )]);
    let found = warnings(&config(json!({})), mixed.path());
    assert_eq!(found.len(), 1);
    assert!(found[0].contains("@tynd/plugin-vault"));
}

#[test]
fn type_only_detection_stays_within_its_statement() {
    let dir = project(&[(
        "src/index.ts",
        "export type A = B\nexport { fs } from \"@tynd/plugin-fs\"\n",
    )]);
    let found = warnings(&config(json!({})), dir.path());
    assert_eq!(found.len(), 1);
    assert!(found[0].contains("@tynd/plugin-fs"));
}

#[test]
fn dynamic_bare_and_require_imports_count() {
    let dir = project(&[
        (
            "a.ts",
            "const { fs } = await import(\"@tynd/plugin-fs/client\");\n",
        ),
        (
            "b.cjs",
            "const { vault } = require('@tynd/plugin-vault');\n",
        ),
        ("c.ts", "import \"@tynd/plugin-shell/client\"\n"),
    ]);
    let found = warnings(&config(json!({})), dir.path());
    assert_eq!(found.len(), 3, "{found:?}");
}

#[test]
fn build_fails_on_undeclared_plugin_import() {
    let dir = project(&[(
        "src/app.ts",
        "import { vault } from \"@tynd/plugin-vault/client\";\n",
    )]);
    let loaded = |value| config::LoadedConfig {
        config: config(value),
        path: None,
        root: dir.path().to_path_buf(),
        sources: Vec::new(),
        environment: BTreeMap::new(),
    };
    let error = ensure_plugin_imports_declared(&loaded(json!({})))
        .expect_err("undeclared plugin import must fail the build")
        .to_string();
    assert!(error.contains("@tynd/plugin-vault"), "{error}");
    assert!(error.contains("tynd add vault"), "{error}");
    ensure_plugin_imports_declared(&loaded(json!({ "plugins": ["@tynd/plugin-vault"] })))
        .expect("declared plugin builds");
}

#[test]
fn declared_plugin_or_api_is_silent() {
    let dir = project(&[(
        "app.ts",
        "import { fs } from \"@tynd/plugin-fs/client\";\n\
             import { http } from \"@tynd/plugin-http\";\n",
    )]);
    let by_plugin = config(json!({
        "plugins": ["@tynd/plugin-fs", { "pkg": "@tynd/plugin-http" }]
    }));
    assert_eq!(warnings(&by_plugin, dir.path()), [] as [String; 0]);
    let by_api = config(json!({ "apis": ["fs", "http"] }));
    assert_eq!(warnings(&by_api, dir.path()), [] as [String; 0]);
}

#[test]
fn ignores_host_apis_node_modules_and_hidden_dirs() {
    let dir = project(&[
        ("a.ts", "import { app, os } from \"@tynd/core/client\";\n"),
        (
            "b.ts",
            "import { osCall } from \"@tynd/core/client/bridge\";\n",
        ),
        (
            "node_modules/x/index.ts",
            "import { fs } from \"@tynd/plugin-fs/client\";\n",
        ),
        (
            ".tynd/gen.ts",
            "import { fs } from \"@tynd/plugin-fs/client\";\n",
        ),
    ]);
    assert_eq!(warnings(&config(json!({})), dir.path()), [] as [String; 0]);
}

#[test]
fn groups_files_per_api_and_truncates_the_list() {
    let files: Vec<(String, String)> = (0..5)
        .map(|i| {
            (
                format!("f{i}.ts"),
                "import { fs } from \"@tynd/plugin-fs/client\";\n".to_owned(),
            )
        })
        .collect();
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(name, body)| (name.as_str(), body.as_str()))
        .collect();
    let dir = project(&refs);
    let warnings = warnings(&config(json!({})), dir.path());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("2 more"));
}

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

#[test]
fn warns_when_scoped_api_is_imported_without_policy() {
    let dir = project(&[(
        "backend/main.ts",
        "import { fs } from \"@tynd/plugin-fs/client\";\n\
             import { shell } from '@tynd/plugin-shell/client';\n\
             app.start({});\n",
    )]);
    let message = open_scope_warning(&config(json!({})), dir.path()).expect("warning");
    assert!(message.contains("fs, shell"), "{message}");
    assert!(message.contains("security policy"));
}

fn warned_apis(source: &str) -> Option<String> {
    let dir = project(&[("backend/main.ts", source)]);
    let message = open_scope_warning(&config(json!({})), dir.path())?;
    // Only the "app uses ..." list; the advice text names `fs, http`.
    let uses = message.strip_prefix("app uses ")?.split(" but ").next()?;
    Some(uses.to_owned())
}

#[test]
fn warns_on_every_plugin_import_form() {
    for (source, api) in [
        (
            "import * as sh from \"@tynd/plugin-shell/client\";\n",
            "shell",
        ),
        ("export * from '@tynd/plugin-process';\n", "process"),
        (
            "const { http } = await import(\"@tynd/plugin-http/client\");\n",
            "http",
        ),
        ("const fs = require('@tynd/plugin-fs/client');\n", "fs"),
    ] {
        assert_eq!(warned_apis(source).as_deref(), Some(api), "{source}");
    }
}

#[test]
fn ignores_core_and_unscoped_plugin_imports() {
    for source in [
        "import { app, os } from \"@tynd/core/client\";\n",
        "import { app } from \"@tynd/core\";\napp.start({});\n",
        "import { dialog } from \"@tynd/plugin-dialog/client\";\n",
        "import { fs } from \"other-package\";\n",
    ] {
        assert!(warned_apis(source).is_none(), "{source}");
    }
}

#[test]
fn warns_on_plugin_imports_and_declared_apis_and_plugins() {
    let dir = project(&[(
        "a.ts",
        "import { http } from \"@tynd/plugin-http/client\";\n",
    )]);
    let message = open_scope_warning(&config(json!({})), dir.path()).expect("warning");
    assert!(message.contains("http"));

    let empty = project(&[("a.ts", "export {};\n")]);
    let official = config(json!({ "plugins": ["@tynd/plugin-process"] }));
    let message = open_scope_warning(&official, empty.path()).expect("warning");
    assert!(message.contains("process"));
    let custom = config(json!({ "plugins": ["acme-plugin"] }));
    let message = open_scope_warning(&custom, empty.path()).expect("warning");
    assert!(message.contains("plugin acme-plugin"));
    let apis = config(json!({ "apis": ["fs", "dialog"] }));
    let message = open_scope_warning(&apis, empty.path()).expect("warning");
    assert!(message.contains("fs") && !message.contains("dialog"));
}

#[test]
fn silent_when_a_policy_is_configured() {
    for start in [
        "app.start({ security: { defaultDeny: true } });",
        "const security = {}; app.start({ security });",
        "app.start({ window, security, tray });",
    ] {
        let dir = project(&[
            ("a.ts", "import { fs } from \"@tynd/plugin-fs/client\";\n"),
            ("backend/main.ts", start),
        ]);
        assert!(
            open_scope_warning(&config(json!({})), dir.path()).is_none(),
            "{start}"
        );
    }
}

#[test]
fn silent_for_quoted_bound_and_unresolvable_policy_configs() {
    for start in [
        "app.start({ \"security\": { defaultDeny: true } });",
        "const cfg = { window, security: {} };\napp.start(cfg);",
        "const cfg: AppConfig = { security };\nawait app.start(cfg);",
        "app.start({ ...base });",
        "app.start(loadConfig());",
        "app.start(imported);",
    ] {
        let dir = project(&[
            ("a.ts", "import { fs } from \"@tynd/plugin-fs/client\";\n"),
            ("backend/main.ts", start),
        ]);
        assert!(
            open_scope_warning(&config(json!({})), dir.path()).is_none(),
            "{start}"
        );
    }
}

#[test]
fn still_warns_when_security_text_is_not_the_start_config_key() {
    for start in [
        "// security: { defaultDeny: true }\napp.start({ window });",
        "/* app.start({ security }) */\napp.start({});",
        "const note = \"security: on\"; app.start({ window });",
        "const security = {};\napp.start({ window, title: security });",
        "const cfg = { window };\napp.start(cfg);",
        "app.start({ window: { security: {} } });",
        "app.start();",
        "const x = { security };\nlog(x);\napp.start({ window });",
    ] {
        let dir = project(&[
            ("a.ts", "import { fs } from \"@tynd/plugin-fs/client\";\n"),
            ("backend/main.ts", start),
        ]);
        assert!(
            open_scope_warning(&config(json!({})), dir.path()).is_some(),
            "{start}"
        );
    }
}

fn assert_warns(source: &str, expect_warning: bool) {
    let dir = project(&[
        ("a.ts", "import { fs } from \"@tynd/plugin-fs/client\";\n"),
        ("backend/main.ts", source),
    ]);
    assert_eq!(
        open_scope_warning(&config(json!({})), dir.path()).is_some(),
        expect_warning,
        "{source}"
    );
}

#[test]
fn warns_when_only_unrelated_start_calls_mention_a_config() {
    for start in [
        "osc.start(0);\napp.start({ window });",
        "recorder.start(1000);\napp.start({});",
        "spinner.start(\"Building\");\napp.start({ window });",
        "worker.start({ security: true });\napp.start({ window });",
        "spinner.start(label);\napp.start({ window });",
    ] {
        assert_warns(start, true);
    }
}

#[test]
fn silent_for_an_aliased_app_import() {
    for source in [
        "import { app as tynd } from \"@tynd/core\";\ntynd.start({ security });",
        "const { app: a } = await import(\"@tynd/core\");\na.start({ security: {} });",
    ] {
        assert_warns(source, false);
    }
}

#[test]
fn regex_literals_do_not_swap_string_and_code() {
    for source in [
            "const t = s.replace(/[\"']/g, \"\");\napp.start({ security: { defaultDeny: true } });",
            "const t = s.replace(/'/g, \"\");\napp.start({ security });",
            "if (/\\/\"/.test(s)) {}\napp.start({ security });",
            "const ok = x.match(/a\\/[/\"]b/) ? 1 : 2;\nconst r = /`/.test(s);\napp.start({ security });",
            "const half = total / 2; const t = a / b;\napp.start({ security });",
        ] {
            assert_warns(source, false);
        }
    // A division is not a regex: the quote after it still opens a string.
    assert_warns(
        "const t = a / b; const s = \"app.start({ security })\";",
        true,
    );
}

#[test]
fn silent_without_scoped_apis_or_plugins() {
    let dir = project(&[
        (
            "a.ts",
            "import { dialog } from \"@tynd/plugin-dialog/client\";\n",
        ),
        (
            "node_modules/x/index.ts",
            "import { fs } from \"@tynd/plugin-fs/client\";\n",
        ),
    ]);
    let official = config(json!({ "plugins": ["@tynd/plugin-notification"] }));
    assert!(open_scope_warning(&official, dir.path()).is_none());
}

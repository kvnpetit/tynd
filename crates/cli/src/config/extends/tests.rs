use std::fs;

use super::super::load::load_typed;

const SCHEMA_URL: &str = "https://kvnpetit.github.io/tynd/schemas/tynd.config.json";

#[test]
fn top_level_schema_is_accepted_in_every_layer() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("base.json"),
        format!(r#"{{ "$schema": "{SCHEMA_URL}", "apis": ["clipboard"] }}"#),
    )
    .expect("write base");
    fs::write(
        dir.path().join("tynd.config.json"),
        format!(r#"{{ "$schema": "{SCHEMA_URL}", "extends": "./base.json" }}"#),
    )
    .expect("write config");

    let loaded = load_typed(dir.path(), None).expect("config with $schema loads");
    assert_eq!(loaded.config.apis, vec!["clipboard".to_owned()]);
}

#[test]
fn nested_schema_is_still_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("tynd.config.json"),
        format!(r#"{{ "bundle": {{ "$schema": "{SCHEMA_URL}" }} }}"#),
    )
    .expect("write config");

    let error = load_typed(dir.path(), None).expect_err("nested $schema is unknown");
    assert!(format!("{error:#}").contains("$schema"), "{error:#}");
}

#[test]
fn non_string_schema_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("tynd.config.json"),
        r#"{ "$schema": true }"#,
    )
    .expect("write config");

    let error = load_typed(dir.path(), None).expect_err("non-string $schema");
    assert!(
        format!("{error:#}").contains("$schema") && format!("{error:#}").contains("string"),
        "{error:#}"
    );
}

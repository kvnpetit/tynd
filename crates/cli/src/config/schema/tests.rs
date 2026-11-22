use std::{env, fs, path::Path};

use serde_json::Value;

use super::{
    generate::{config_schema, render_config_schema},
    KNOWN_APIS,
};

const PUBLISHED: &str = "../../packages/cli/schemas/tynd.config.json";

/// `bun scripts/gen-api-catalog.ts` runs this test with `TYND_CONFIG_SCHEMA_OUT` set to
/// obtain the schema (it formats the published copy with Biome); otherwise the test checks
/// that the published copy describes the same schema as the model.
#[test]
fn published_schema_matches_the_model() {
    if let Some(out) = env::var_os("TYND_CONFIG_SCHEMA_OUT") {
        fs::write(out, render_config_schema()).unwrap();
        return;
    }
    let published =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(PUBLISHED)).unwrap();
    let published: Value = serde_json::from_str(&published).unwrap();
    assert!(
        published == config_schema(),
        "{PUBLISHED} is stale; run `bun scripts/gen-api-catalog.ts`"
    );
}

fn objects<'a>(value: &'a Value, found: &mut Vec<&'a Value>) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| objects(item, found)),
        Value::Object(object) => {
            if object.contains_key("properties") {
                found.push(value);
            }
            object.values().for_each(|item| objects(item, found));
        },
        _ => {},
    }
}

#[test]
fn every_object_rejects_unknown_keys_like_the_model() {
    let schema = config_schema();
    let mut found = Vec::new();
    objects(&schema, &mut found);
    assert!(
        found.len() > 10,
        "the schema should describe the nested config objects"
    );
    for object in found {
        assert_eq!(
            object["additionalProperties"],
            Value::Bool(false),
            "{object} accepts unknown keys"
        );
    }
}

#[test]
fn api_names_follow_the_catalog_and_never_admit_null() {
    let schema = config_schema();
    let names: Vec<&str> = schema["$defs"]["ApiName"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(names, KNOWN_APIS);
    assert!(!render_config_schema().contains("\"null\""));
}

#[test]
fn file_level_keys_and_required_fields_are_described() {
    let schema = config_schema();
    let properties = &schema["properties"];
    assert!(properties.get("$schema").is_some());
    assert!(properties.get("extends").is_some());
    assert_eq!(
        schema["$defs"]["UpdaterConfig"]["required"],
        serde_json::json!(["endpoints", "publicKeys"])
    );
    assert_eq!(
        schema["$defs"]["BundleConfig"]["required"],
        serde_json::json!(["identifier"])
    );
}

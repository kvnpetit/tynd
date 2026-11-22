//! Builds the published schema from the model; test-only, because the schema is emitted
//! by `cargo test` (see `tests::published_schema_matches_the_model`).

use schemars::generate::SchemaSettings;
use serde_json::{json, Map, Value};

use crate::config::TyndConfig;

const SCHEMA_ID: &str = "https://tynd-apps.github.io/tynd/schemas/tynd.config.json";

/// The published schema of `tynd.config.*`.
pub(super) fn config_schema() -> Value {
    let mut schema = SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<TyndConfig>();
    schema.insert("$id".to_owned(), SCHEMA_ID.into());
    let mut value = schema.to_value();
    normalize(&mut value);
    value
}

/// Pretty-printed schema with a trailing newline, as published.
pub(super) fn render_config_schema() -> String {
    let mut text = serde_json::to_string_pretty(&config_schema())
        .unwrap_or_else(|error| unreachable!("a JSON value always serializes: {error}"));
    text.push('\n');
    text
}

/// Marks a field that serde defaults (so the CLI can report it missing in its own words)
/// but that a config must spell: `#[schemars(extend("x-tynd-required" = true))]`.
const REQUIRED_MARKER: &str = "x-tynd-required";

fn normalize(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(normalize),
        Value::Object(object) => {
            drop_null_alternatives(object);
            apply_required_markers(object);
            unwrap_doc_lines(object);
            object.values_mut().for_each(normalize);
        },
        _ => {},
    }
}

fn apply_required_markers(object: &mut Map<String, Value>) {
    let Some(Value::Object(properties)) = object.get_mut("properties") else {
        return;
    };
    let mut required = Vec::new();
    for (name, property) in properties.iter_mut() {
        let Some(property) = property.as_object_mut() else {
            continue;
        };
        if property.remove(REQUIRED_MARKER) == Some(Value::Bool(true)) {
            // A default never applies to a key the config must spell.
            property.remove("default");
            required.push(Value::String(name.clone()));
        }
    }
    if required.is_empty() {
        return;
    }
    if let Value::Array(existing) = object
        .entry("required")
        .or_insert_with(|| Value::Array(Vec::new()))
    {
        existing.extend(required);
    }
}

/// Doc comments keep their source line breaks; editors show a description as one
/// paragraph per blank-line-separated block.
fn unwrap_doc_lines(object: &mut Map<String, Value>) {
    if let Some(Value::String(description)) = object.get_mut("description") {
        *description = description
            .split("\n\n")
            .map(|paragraph| paragraph.split('\n').collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n\n");
    }
}

/// Serde accepts `null` for an absent `Option`, but a config never needs to spell it:
/// drop the `null` alternatives schemars adds so editors and the TS types only offer
/// the real value.
fn drop_null_alternatives(object: &mut Map<String, Value>) {
    if object.get("default").is_some_and(Value::is_null) {
        object.remove("default");
    }
    if let Some(Value::Array(types)) = object.get_mut("type") {
        types.retain(|kind| kind != "null");
        if let [single] = types.as_slice() {
            let single = single.clone();
            object.insert("type".to_owned(), single);
        }
    }
    if let Some(Value::Array(values)) = object.get_mut("enum") {
        values.retain(|value| !value.is_null());
    }
    let Some(Value::Array(alternatives)) = object.get_mut("anyOf") else {
        return;
    };
    alternatives.retain(|alternative| *alternative != json!({ "type": "null" }));
    if let [Value::Object(single)] = alternatives.as_slice() {
        let single = single.clone();
        object.remove("anyOf");
        for (key, value) in single {
            object.entry(key).or_insert(value);
        }
    }
}

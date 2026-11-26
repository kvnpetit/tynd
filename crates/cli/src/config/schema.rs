//! JSON schema of a config file, derived from the typed model in `model.rs`.
//!
//! `bun scripts/gen-api-catalog.ts` publishes it as `packages/cli/schemas/tynd.config.json`
//! (plus the byte-identical docs copy) and generates the TypeScript config types from it;
//! the tests fail when the published copy no longer matches the model.

use std::borrow::Cow;

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde_json::{json, Value};

use super::KNOWN_APIS;

/// Schema-only stand-in for an API name: one of `KNOWN_APIS`.
pub(crate) struct ApiName;

impl JsonSchema for ApiName {
    fn schema_name() -> Cow<'static, str> {
        "ApiName".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Native API that can be enabled in an app-specific Tynd host.",
            "type": "string",
            "enum": KNOWN_APIS,
        })
    }
}

/// Adds the file-level keys that `extends.rs` strips before the typed model
/// deserializes a config file.
pub(super) fn config_file_keys(schema: &mut Schema) {
    let local = json!({
        "type": "string",
        "minLength": 1,
        "not": { "pattern": "^[A-Za-z][A-Za-z0-9+.-]*://" },
    });
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    properties.insert(
        "$schema".to_owned(),
        json!({
            "description": "Editor-only JSON Schema reference. Ignored by the CLI.",
            "type": "string",
        }),
    );
    properties.insert(
        "extends".to_owned(),
        json!({
            "description": "Base config files (local paths or npm packages, no URLs) this file extends; its own values override theirs.",
            "anyOf": [local, { "type": "array", "items": local, "minItems": 1 }],
        }),
    );
}

#[cfg(test)]
mod generate;
#[cfg(test)]
mod tests;

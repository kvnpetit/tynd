use serde_json::{json, Value};

use crate::Capability;

pub fn validate_capability_shape(cap: &Capability) -> Result<(), String> {
    if !cap.local && cap.remote.is_none() {
        return Err(format!(
            "acl.validateCapability: capability '{}' has `local: false` and no `remote` pattern - \
             it would never match. Set `local: true` (default) or supply a `remote` URL pattern.",
            cap.identifier
        ));
    }
    Ok(())
}

pub fn capability_schema() -> Value {
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "Tynd Capability",
        "description": "Permission grant for one or more windows / webviews. Tynd capability format.",
        "oneOf": [
            { "$ref": "#/definitions/Capability" },
            { "type": "array", "items": { "$ref": "#/definitions/Capability" } },
            {
                "type": "object",
                "required": ["capabilities"],
                "properties": {
                    "capabilities": {
                        "type": "array",
                        "items": { "$ref": "#/definitions/Capability" }
                    }
                }
            }
        ],
        "definitions": {
            "Capability": capability_definition_schema()
        }
    })
}

fn capability_definition_schema() -> Value {
    json!({
        "type": "object",
        "required": ["identifier"],
        "properties": {
            "identifier": {
                "type": "string",
                "description": "Unique capability id (e.g. main-window-default)."
            },
            "description": { "type": "string" },
            "local": {
                "type": "boolean",
                "default": true,
                "description": "Whether this capability applies to local app URLs."
            },
            "remote": {
                "type": "object",
                "properties": {
                    "urls": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "URL patterns that may use this capability."
                    }
                }
            },
            "windows": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Window labels (glob patterns supported)."
            },
            "webviews": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Webview labels (glob patterns supported)."
            },
            "permissions": {
                "type": "array",
                "uniqueItems": true,
                "items": {
                    "oneOf": [
                        { "type": "string" },
                        inline_permission_schema()
                    ]
                }
            },
            "platforms": target_list_schema()
        }
    })
}

fn inline_permission_schema() -> Value {
    json!({
        "type": "object",
        "required": ["identifier"],
        "properties": {
            "identifier": { "type": "string" },
            "description": { "type": "string" },
            "version": {
                "type": "integer",
                "minimum": 1
            },
            "commands": {
                "type": "object",
                "properties": {
                    "allow": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "deny": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                }
            },
            "scope": {},
            "platforms": target_list_schema()
        }
    })
}

fn target_list_schema() -> Value {
    json!({
        "type": "array",
        "items": {
            "enum": ["macOS", "windows", "linux", "android", "iOS"]
        }
    })
}

//! `tynd explain <topic>`.

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::{
    args::ExplainArgs,
    config,
    mcp::catalog::{find_known_error, known_error_json, known_errors},
    output::{write_json, write_line},
};

use super::shared::normalize_capability;

pub(in crate::commands) fn explain(args: &ExplainArgs) -> Result<()> {
    let details = details(args.topic.as_deref().unwrap_or("runtime"))?;
    if args.json {
        return write_json(&details);
    }
    if details["kind"] == "error" {
        print_error(&details)?;
        return Ok(());
    }
    if let Some(summary) = details["summary"].as_str() {
        write_line(format_args!("{summary}"))?;
    }
    if let Some(values) = details.get("values").and_then(Value::as_array) {
        write_line(format_args!(
            "{}",
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
    }
    Ok(())
}

fn details(topic: &str) -> Result<Value> {
    if let Some(api) = normalize_capability(topic) {
        return Ok(
            json!({ "topic": api, "kind": "capability", "summary": format!("Native {api} API compiled into the host when requested.") }),
        );
    }
    Ok(match topic {
        "runtime" => {
            json!({ "topic": "runtime", "summary": "Rust owns the CLI, MCP, host and bundling control plane. Bun executes user TypeScript only." })
        },
        "capabilities" => json!({
            "topic": "capabilities",
            "summary": "Available native capabilities:",
            "values": config::KNOWN_APIS,
        }),
        "errors" => json!({
            "topic": "errors",
            "summary": "Known error codes (`tynd explain <CODE>` for one):",
            "values": known_errors().iter().map(|error| error.code).collect::<Vec<_>>(),
        }),
        other => {
            if let Some(error) = find_known_error(&other.to_ascii_uppercase()) {
                let mut details = known_error_json(error);
                details["topic"] = json!(error.code);
                details["kind"] = json!("error");
                return Ok(details);
            }
            if looks_like_error_code(other) {
                bail!("unknown error code {other:?}; run `tynd explain errors` to list every code");
            }
            bail!(
                "unknown explanation topic {other:?}; expected runtime, capabilities, errors, \
                 a capability name or an error code"
            )
        },
    })
}

/// `SOME_CODE`: upper-case words joined by underscores.
fn looks_like_error_code(topic: &str) -> bool {
    topic.contains('_')
        && topic
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn print_error(details: &Value) -> Result<()> {
    let field = |key: &str| details[key].as_str().unwrap_or_default();
    write_line(format_args!("{}: {}", field("code"), field("title")))?;
    write_line(format_args!("  Symptom: {}", field("symptom")))?;
    write_line(format_args!("  Cause:   {}", field("cause")))?;
    write_line(format_args!("  Fix:     {}", field("fix")))?;
    if let Some(doc) = details["doc"].as_str() {
        write_line(format_args!("  Docs:    {doc} (tynd-mcp resource)"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::details;

    #[test]
    fn error_codes_print_their_catalog_entry() {
        let entry = details("BACKEND_DEAD").expect("known code");
        assert_eq!(entry["kind"], "error");
        assert_eq!(entry["topic"], "BACKEND_DEAD");
        assert_eq!(entry["title"], "Host process died");
        // Codes are matched without regard to case.
        assert_eq!(
            details("protocol_error").expect("known code")["code"],
            "PROTOCOL_ERROR"
        );
    }

    #[test]
    fn errors_topic_lists_every_catalog_code() {
        let list = details("errors").expect("errors topic");
        let values = list["values"].as_array().expect("values");
        assert_eq!(values.len(), crate::mcp::catalog::known_errors().len());
        assert!(values.iter().any(|code| code == "PERMISSION_DENIED"));
        // The dev error overlay points to `tynd explain BACKEND_ERROR`.
        assert!(values.iter().any(|code| code == "BACKEND_ERROR"));
    }

    #[test]
    fn unknown_codes_point_to_the_code_list() {
        let error = details("TYND_NOPE_001").expect_err("unknown code");
        assert!(error.to_string().contains("tynd explain errors"), "{error}");
        let error = details("nope").expect_err("unknown topic");
        assert!(
            error.to_string().contains("unknown explanation topic"),
            "{error}"
        );
    }

    #[test]
    fn capabilities_still_resolve_before_codes() {
        assert_eq!(details("FS").expect("capability")["kind"], "capability");
    }
}

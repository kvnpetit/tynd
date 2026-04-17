use serde_json::Value;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "openExternal" => {
            let url = args
                .as_str()
                .or_else(|| args.get("url").and_then(|u| u.as_str()))
                .ok_or_else(|| "shell.openExternal: missing url argument".to_string())?;
            // Only allow safe URL schemes to prevent executing arbitrary handlers
            if !url.starts_with("http://")
                && !url.starts_with("https://")
                && !url.starts_with("mailto:")
            {
                return Err(format!(
                    "shell.openExternal: unsupported scheme in '{url}' — only http, https, mailto allowed"
                ));
            }
            open::that(url).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        },
        "openPath" => {
            let path = args
                .as_str()
                .or_else(|| args.get("path").and_then(|p| p.as_str()))
                .ok_or_else(|| "shell.openPath: missing path argument".to_string())?;
            open::that(path).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        },
        _ => Err(format!("shell.{method}: unknown method")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_unsupported_scheme() {
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://example.com",
            "vscode://open?file=x",
            "data:text/html,<script>",
            "no-scheme-at-all",
        ] {
            let err = dispatch("openExternal", &Value::String(bad.to_string())).expect_err(bad);
            assert!(err.contains("unsupported scheme"), "{bad}: {err}");
        }
    }

    #[test]
    fn missing_url_is_reported() {
        let err = dispatch("openExternal", &json!({})).unwrap_err();
        assert!(err.contains("missing url"));
    }

    #[test]
    fn missing_path_is_reported() {
        let err = dispatch("openPath", &json!({})).unwrap_err();
        assert!(err.contains("missing path"));
    }

    #[test]
    fn unknown_method_errors() {
        assert!(dispatch("nope", &json!({})).is_err());
    }

    #[test]
    fn accepts_url_from_object_form() {
        let err = dispatch("openExternal", &json!({ "url": "ftp://x" })).unwrap_err();
        assert!(err.contains("unsupported scheme"));
    }
}

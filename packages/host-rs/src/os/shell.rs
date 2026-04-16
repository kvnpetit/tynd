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

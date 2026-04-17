use serde_json::{json, Value};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "info" => Ok(info()),
        "homeDir" => Ok(path_or_null(dirs::home_dir())),
        "tmpDir" => Ok(path_to_value(&std::env::temp_dir())),
        "configDir" => Ok(path_or_null(dirs::config_dir())),
        "dataDir" => Ok(path_or_null(dirs::data_dir())),
        "cacheDir" => Ok(path_or_null(dirs::cache_dir())),
        "exePath" => Ok(std::env::current_exe()
            .ok()
            .map_or(Value::Null, |p| path_to_value(&p))),
        "cwd" => Ok(std::env::current_dir()
            .ok()
            .map_or(Value::Null, |p| path_to_value(&p))),
        "env" => {
            let key = args
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "os.env: missing 'key'".to_string())?;
            Ok(std::env::var(key).map_or(Value::Null, Value::String))
        },
        _ => Err(format!("os.{method}: unknown method")),
    }
}

fn path_or_null(p: Option<std::path::PathBuf>) -> Value {
    p.map_or(Value::Null, |p| path_to_value(&p))
}

fn path_to_value(p: &std::path::Path) -> Value {
    Value::String(p.to_string_lossy().into_owned())
}

fn info() -> Value {
    json!({
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "family": std::env::consts::FAMILY,
    })
}

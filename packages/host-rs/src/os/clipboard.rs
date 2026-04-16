use arboard::Clipboard;
use serde_json::Value;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "readText"  => read_text(),
        "writeText" => write_text(args),
        _ => Err(format!("clipboard.{method}: unknown method")),
    }
}

fn read_text() -> Result<Value, String> {
    let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
    let text = cb.get_text().unwrap_or_default();
    Ok(Value::String(text))
}

fn write_text(args: &Value) -> Result<Value, String> {
    let text = args
        .as_str()
        .or_else(|| args.get("text").and_then(|t| t.as_str()))
        .unwrap_or("");
    let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_text(text).map_err(|e| e.to_string())?;
    Ok(Value::Null)
}

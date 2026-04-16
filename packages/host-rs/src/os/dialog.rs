use rfd::{AsyncFileDialog, AsyncMessageDialog, MessageButtons, MessageDialogResult, MessageLevel};
use serde_json::Value;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "openFile"  => open_file(args, false),
        "openFiles" => open_file(args, true),
        "saveFile"  => save_file(args),
        "message"   => message_box(args),
        "confirm"   => confirm(args),
        _ => Err(format!("dialog.{method}: unknown method")),
    }
}

fn build_open_dialog(args: &Value) -> AsyncFileDialog {
    let mut d = AsyncFileDialog::new();
    if let Some(title) = args.get("title").and_then(|t| t.as_str()) {
        d = d.set_title(title);
    }
    if let Some(dir) = args.get("defaultDir").and_then(|p| p.as_str()) {
        d = d.set_directory(dir);
    }
    apply_filters(d, args)
}

fn apply_filters(mut d: AsyncFileDialog, args: &Value) -> AsyncFileDialog {
    if let Some(filters) = args.get("filters").and_then(|f| f.as_array()) {
        for filter in filters {
            let name = filter.get("name").and_then(|n| n.as_str()).unwrap_or("*");
            let exts: Vec<&str> = filter
                .get("extensions")
                .and_then(|e| e.as_array())
                .map(|arr| arr.iter().filter_map(|e| e.as_str()).collect())
                .unwrap_or_default();
            d = d.add_filter(name, &exts);
        }
    }
    d
}

fn open_file(args: &Value, multiple: bool) -> Result<Value, String> {
    let d = build_open_dialog(args);
    if multiple {
        let files = pollster::block_on(d.pick_files());
        Ok(match files {
            None => Value::Null,
            Some(handles) => Value::Array(
                handles
                    .iter()
                    .map(|h| Value::String(h.path().to_string_lossy().into_owned()))
                    .collect(),
            ),
        })
    } else {
        let file = pollster::block_on(d.pick_file());
        Ok(file
            .map(|h| Value::String(h.path().to_string_lossy().into_owned()))
            .unwrap_or(Value::Null))
    }
}

fn save_file(args: &Value) -> Result<Value, String> {
    let mut d = AsyncFileDialog::new();
    if let Some(title) = args.get("title").and_then(|t| t.as_str()) {
        d = d.set_title(title);
    }
    if let Some(name) = args.get("defaultName").and_then(|p| p.as_str()) {
        d = d.set_file_name(name);
    }
    if let Some(dir) = args.get("defaultDir").and_then(|p| p.as_str()) {
        d = d.set_directory(dir);
    }
    d = apply_filters(d, args);

    let file = pollster::block_on(d.save_file());
    Ok(file
        .map(|h| Value::String(h.path().to_string_lossy().into_owned()))
        .unwrap_or(Value::Null))
}

fn message_box(args: &Value) -> Result<Value, String> {
    let text = args
        .get("message")
        .and_then(|m| m.as_str())
        .or_else(|| args.as_str())
        .unwrap_or("");
    let title = args.get("title").and_then(|t| t.as_str()).unwrap_or("Message");
    let level = match args.get("kind").and_then(|k| k.as_str()).unwrap_or("info") {
        "warning" => MessageLevel::Warning,
        "error"   => MessageLevel::Error,
        _         => MessageLevel::Info,
    };

    pollster::block_on(
        AsyncMessageDialog::new()
            .set_title(title)
            .set_description(text)
            .set_level(level)
            .set_buttons(MessageButtons::Ok)
            .show(),
    );
    Ok(Value::Null)
}

fn confirm(args: &Value) -> Result<Value, String> {
    let text = args
        .get("message")
        .and_then(|m| m.as_str())
        .or_else(|| args.as_str())
        .unwrap_or("");
    let title = args.get("title").and_then(|t| t.as_str()).unwrap_or("Confirm");

    let result = pollster::block_on(
        AsyncMessageDialog::new()
            .set_title(title)
            .set_description(text)
            .set_level(MessageLevel::Info)
            .set_buttons(MessageButtons::OkCancel)
            .show(),
    );
    Ok(Value::Bool(result == MessageDialogResult::Ok))
}

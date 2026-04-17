use serde_json::{json, Value};
use std::fs;
use std::time::UNIX_EPOCH;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "readText" => read_text(args),
        "writeText" => write_text(args),
        "exists" => exists(args),
        "stat" => stat(args),
        "readDir" => read_dir(args),
        "mkdir" => mkdir(args),
        "remove" => remove(args),
        "rename" => rename(args),
        "copy" => copy(args),
        _ => Err(format!("fs.{method}: unknown method")),
    }
}

fn path_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("fs: missing '{name}' argument"))
}

fn read_text(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let s = fs::read_to_string(path).map_err(|e| format!("fs.readText({path}): {e}"))?;
    Ok(Value::String(s))
}

fn write_text(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "fs.writeText: missing 'content'".to_string())?;
    if args
        .get("createDirs")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        if let Some(parent) = std::path::Path::new(path).parent() {
            fs::create_dir_all(parent).map_err(|e| format!("fs.writeText: {e}"))?;
        }
    }
    fs::write(path, content).map_err(|e| format!("fs.writeText({path}): {e}"))?;
    Ok(Value::Null)
}

fn exists(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    Ok(Value::Bool(std::path::Path::new(path).exists()))
}

fn stat(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let meta = fs::metadata(path).map_err(|e| format!("fs.stat({path}): {e}"))?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    Ok(json!({
        "size": meta.len(),
        "isFile": meta.is_file(),
        "isDir": meta.is_dir(),
        "isSymlink": meta.file_type().is_symlink(),
        "mtime": mtime,
    }))
}

fn read_dir(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let entries = fs::read_dir(path).map_err(|e| format!("fs.readDir({path}): {e}"))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("fs.readDir: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let ft = entry.file_type().map_err(|e| format!("fs.readDir: {e}"))?;
        out.push(json!({
            "name": name,
            "isFile": ft.is_file(),
            "isDir": ft.is_dir(),
            "isSymlink": ft.is_symlink(),
        }));
    }
    Ok(Value::Array(out))
}

fn mkdir(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if recursive {
        fs::create_dir_all(path).map_err(|e| format!("fs.mkdir({path}): {e}"))?;
    } else {
        fs::create_dir(path).map_err(|e| format!("fs.mkdir({path}): {e}"))?;
    }
    Ok(Value::Null)
}

fn remove(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let p = std::path::Path::new(path);
    let meta = fs::symlink_metadata(p).map_err(|e| format!("fs.remove({path}): {e}"))?;
    if meta.is_dir() {
        if recursive {
            fs::remove_dir_all(p).map_err(|e| format!("fs.remove({path}): {e}"))?;
        } else {
            fs::remove_dir(p).map_err(|e| format!("fs.remove({path}): {e}"))?;
        }
    } else {
        fs::remove_file(p).map_err(|e| format!("fs.remove({path}): {e}"))?;
    }
    Ok(Value::Null)
}

fn rename(args: &Value) -> Result<Value, String> {
    let from = path_arg(args, "from")?;
    let to = path_arg(args, "to")?;
    fs::rename(from, to).map_err(|e| format!("fs.rename({from} → {to}): {e}"))?;
    Ok(Value::Null)
}

fn copy(args: &Value) -> Result<Value, String> {
    let from = path_arg(args, "from")?;
    let to = path_arg(args, "to")?;
    fs::copy(from, to).map_err(|e| format!("fs.copy({from} → {to}): {e}"))?;
    Ok(Value::Null)
}

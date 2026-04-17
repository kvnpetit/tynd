use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "exec" => exec(args, false),
        "execShell" => exec(args, true),
        _ => Err(format!("process.{method}: unknown method")),
    }
}

fn exec(args: &Value, shell: bool) -> Result<Value, String> {
    let cmd_str = args
        .get("cmd")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "process.exec: missing 'cmd'".to_string())?;

    let argv: Vec<String> = args
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut command = if shell {
        let joined = if argv.is_empty() {
            cmd_str.to_string()
        } else {
            format!("{cmd_str} {}", argv.join(" "))
        };
        #[cfg(target_os = "windows")]
        {
            let mut c = Command::new("cmd");
            c.args(["/C", &joined]);
            c
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut c = Command::new("sh");
            c.args(["-c", &joined]);
            c
        }
    } else {
        let mut c = Command::new(cmd_str);
        c.args(&argv);
        c
    };

    if let Some(cwd) = args.get("cwd").and_then(|v| v.as_str()) {
        command.current_dir(cwd);
    }

    if let Some(env_obj) = args.get("env").and_then(|v| v.as_object()) {
        let merged: HashMap<String, String> = env_obj
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect();
        command.envs(merged);
    }

    let input = args.get("input").and_then(|v| v.as_str()).map(String::from);

    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|e| format!("spawn failed: {e}"))?;

    if let Some(data) = input {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(data.as_bytes())
                .map_err(|e| format!("stdin write failed: {e}"))?;
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("wait failed: {e}"))?;

    Ok(json!({
        "code": output.status.code(),
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_captures_stdout() {
        #[cfg(target_os = "windows")]
        let v = exec(
            &json!({ "cmd": "cmd", "args": ["/C", "echo hello"] }),
            false,
        )
        .unwrap();
        #[cfg(not(target_os = "windows"))]
        let v = exec(&json!({ "cmd": "echo", "args": ["hello"] }), false).unwrap();
        assert!(v["stdout"].as_str().unwrap().contains("hello"));
        assert_eq!(v["code"].as_i64().unwrap(), 0);
    }

    #[test]
    fn exec_shell_runs_pipe() {
        #[cfg(target_os = "windows")]
        let v = exec(&json!({ "cmd": "echo piped" }), true).unwrap();
        #[cfg(not(target_os = "windows"))]
        let v = exec(&json!({ "cmd": "echo piped | tr a-z A-Z" }), true).unwrap();
        let stdout = v["stdout"].as_str().unwrap();
        assert!(!stdout.is_empty());
    }

    #[test]
    fn missing_cmd_errors() {
        assert!(exec(&json!({}), false).is_err());
    }

    #[test]
    fn nonzero_exit_preserved() {
        #[cfg(target_os = "windows")]
        let v = exec(&json!({ "cmd": "cmd", "args": ["/C", "exit 7"] }), false).unwrap();
        #[cfg(not(target_os = "windows"))]
        let v = exec(&json!({ "cmd": "sh", "args": ["-c", "exit 7"] }), false).unwrap();
        assert_eq!(v["code"].as_i64().unwrap(), 7);
    }
}

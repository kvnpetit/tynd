use std::{
    io::Write,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use base64::Engine as _;
use serde_json::{json, Map, Value};

use super::{
    encoding::{join_output, join_output_until, read_limited_output, OutputEncoding, OutputReader},
    prepare_command, proc_err,
};
use crate::security::policy;

const DEFAULT_EXEC_TIMEOUT_MS: u64 = 60_000;
const MAX_EXEC_TIMEOUT_MS: u64 = 10 * 60_000;
const DEFAULT_EXEC_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_EXEC_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
/// How long output readers get to reach EOF once the child has exited or been
/// killed. Buffered data drains in microseconds; anything longer means a
/// descendant still holds the pipe, and the call returns the output read so
/// far (so `execShell("srv &")` reports the shell's exit status).
const PIPE_DRAIN_GRACE: Duration = Duration::from_secs(2);

type OutputReaderSlot = Option<OutputReader>;
type InputWriter = Option<thread::JoinHandle<std::io::Result<()>>>;

pub(super) struct ExecRequest {
    pub(crate) command: String,
    pub(crate) argv: Vec<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) env: Option<Map<String, Value>>,
    pub(crate) input: Option<String>,
    pub(crate) input_encoding: Option<String>,
    pub(crate) output_encoding: Option<String>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) max_output_bytes: Option<u64>,
    pub(crate) shell: bool,
}

struct OutputReaders {
    stdout: OutputReaderSlot,
    stderr: OutputReaderSlot,
}

struct WaitOutcome {
    status: ExitStatus,
    timed_out: bool,
}

pub(super) fn exec_impl(request: ExecRequest) -> Result<Value, String> {
    let method = if request.shell { "execShell" } else { "exec" };
    validate_shell_request(&request)?;
    let timeout = resolve_timeout(request.timeout_ms);
    let max_output_bytes = resolve_output_limit(request.max_output_bytes);
    let output_encoding = parse_output_encoding(request.output_encoding.as_deref(), method)?;
    let input = decode_input(request.input, request.input_encoding.as_deref(), method)?;

    let mut command = build_command(&request.command, &request.argv, request.shell);
    prepare_command(
        &mut command,
        "exec",
        request.cwd.as_deref(),
        request.env.as_ref(),
    )?;
    configure_stdio(&mut command, input.is_some());

    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn failed: {error}"))?;
    let output_exceeded = Arc::new(AtomicBool::new(false));
    // The timeout covers the whole call, including the stdin write below.
    let deadline = Instant::now() + timeout;
    let readers = start_output_readers(&mut child, max_output_bytes, &output_exceeded)?;
    let writer = start_input_writer(&mut child, input)?;
    let outcome = wait_for_exit(&mut child, deadline, &output_exceeded)?;
    let drain_deadline = Instant::now() + PIPE_DRAIN_GRACE;
    let stdout = join_output_until(readers.stdout, "stdout", drain_deadline);
    let stderr = join_output_until(readers.stderr, "stderr", drain_deadline);
    let input_result = finish_input_writer(writer);

    validate_completion(
        method,
        timeout,
        max_output_bytes,
        outcome.timed_out,
        &output_exceeded,
    )?;
    let stdout = stdout?;
    let stderr = stderr?;
    input_result?;
    Ok(render_output(
        outcome.status,
        stdout,
        stderr,
        output_encoding,
    ))
}

fn validate_shell_request(request: &ExecRequest) -> Result<(), String> {
    if !request.shell {
        return Ok(());
    }
    if !request.argv.is_empty() {
        return Err(
            "process.execShell: extra 'args' are not allowed in shell mode - embed them in the \
             'cmd' string yourself (and quote them)"
                .into(),
        );
    }
    // Shell mode never accepts caller env: `cmd /C` expands `%VAR%` (and `sh`
    // expands `$VAR`) before the first-token allowlist can matter.
    let env_keys = request
        .env
        .iter()
        .flat_map(|env| env.keys().map(String::as_str));
    policy::check_shell_exec_with_env(&request.command, env_keys)
}

fn resolve_timeout(timeout_ms: Option<u64>) -> Duration {
    Duration::from_millis(
        timeout_ms
            .unwrap_or(DEFAULT_EXEC_TIMEOUT_MS)
            .clamp(1, MAX_EXEC_TIMEOUT_MS),
    )
}

fn resolve_output_limit(max_output_bytes: Option<u64>) -> usize {
    max_output_bytes
        .and_then(|bytes| usize::try_from(bytes).ok())
        .unwrap_or(DEFAULT_EXEC_OUTPUT_BYTES)
        .clamp(1, MAX_EXEC_OUTPUT_BYTES)
}

fn parse_output_encoding(value: Option<&str>, method: &str) -> Result<OutputEncoding, String> {
    match value {
        None | Some("utf8") => Ok(OutputEncoding::Utf8),
        Some("base64") => Ok(OutputEncoding::Base64),
        Some(other) => Err(proc_err(
            method,
            format!("outputEncoding: expected utf8|base64, got '{other}'"),
        )),
    }
}

fn decode_input(
    input: Option<String>,
    encoding: Option<&str>,
    method: &str,
) -> Result<Option<Vec<u8>>, String> {
    let Some(data) = input else {
        return Ok(None);
    };
    match encoding.unwrap_or("utf8") {
        "utf8" => Ok(Some(data.into_bytes())),
        "base64" => base64::engine::general_purpose::STANDARD
            .decode(data.as_bytes())
            .map(Some)
            .map_err(|error| proc_err(method, format!("'input' is not valid base64: {error}"))),
        other => Err(proc_err(
            method,
            format!("inputEncoding: expected 'utf8' or 'base64', got '{other}'"),
        )),
    }
}

fn build_command(command: &str, argv: &[String], shell: bool) -> Command {
    if !shell {
        let mut process = Command::new(command);
        process.args(argv);
        return process;
    }

    #[cfg(target_os = "windows")]
    {
        let mut process = Command::new("cmd");
        process.args(["/C", command]);
        // `cmd` searches the current directory before PATH; the caller picks
        // the cwd, so a planted `git.bat` there would run in place of the
        // allowlisted program.
        process.env("NoDefaultCurrentDirectoryInExePath", "1");
        process
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut process = Command::new("sh");
        process.args(["-c", command]);
        process
    }
}

fn configure_stdio(command: &mut Command, has_input: bool) {
    command.stdin(if has_input {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
}

fn start_output_readers(
    child: &mut Child,
    max_output_bytes: usize,
    output_exceeded: &Arc<AtomicBool>,
) -> Result<OutputReaders, String> {
    let stdout = child
        .stdout
        .take()
        .map(|stdout| {
            read_limited_output(
                stdout,
                "stdout",
                max_output_bytes,
                Arc::clone(output_exceeded),
            )
        })
        .transpose()
        .map_err(|error| {
            let _ = terminate(child);
            format!("failed to spawn stdout reader: {error}")
        })?;

    let stderr = match child
        .stderr
        .take()
        .map(|stderr| {
            read_limited_output(
                stderr,
                "stderr",
                max_output_bytes,
                Arc::clone(output_exceeded),
            )
        })
        .transpose()
    {
        Ok(stderr) => stderr,
        Err(error) => {
            let _ = terminate(child);
            let _ = join_output(stdout, "stdout");
            return Err(format!("failed to spawn stderr reader: {error}"));
        },
    };

    Ok(OutputReaders { stdout, stderr })
}

/// Feed `input` to the child on its own thread so a child that never reads
/// stdin cannot stall the call past its timeout.
fn start_input_writer(child: &mut Child, input: Option<Vec<u8>>) -> Result<InputWriter, String> {
    let Some(bytes) = input else {
        return Ok(None);
    };
    let Some(mut stdin) = child.stdin.take() else {
        return Ok(None);
    };
    thread::Builder::new()
        .name("tynd-process-stdin".into())
        .spawn(move || {
            let result = stdin.write_all(&bytes);
            // Dropping `stdin` closes the pipe so the child sees EOF.
            drop(stdin);
            result
        })
        .map(Some)
        .map_err(|error| {
            let _ = terminate(child);
            format!("failed to spawn stdin writer: {error}")
        })
}

/// Collect the stdin writer's result when it already finished. A writer still
/// blocked (child alive-but-deaf descendant holding the pipe) is detached; a
/// broken pipe only means the child stopped reading, which is not an error.
fn finish_input_writer(writer: InputWriter) -> Result<(), String> {
    let Some(writer) = writer else {
        return Ok(());
    };
    if !writer.is_finished() {
        return Ok(());
    }
    match writer.join() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Ok(Err(error)) => Err(format!("stdin write failed: {error}")),
        Err(_) => Err("stdin writer panicked".into()),
    }
}

fn wait_for_exit(
    child: &mut Child,
    deadline: Instant,
    output_exceeded: &AtomicBool,
) -> Result<WaitOutcome, String> {
    loop {
        if output_exceeded.load(Ordering::Acquire) {
            let status = terminate(child)
                .map_err(|error| format!("wait after output cap failed: {error}"))?;
            return Ok(WaitOutcome {
                status,
                timed_out: false,
            });
        }
        if Instant::now() >= deadline {
            let status =
                terminate(child).map_err(|error| format!("wait after timeout failed: {error}"))?;
            return Ok(WaitOutcome {
                status,
                timed_out: true,
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(WaitOutcome {
                    status,
                    timed_out: false,
                });
            },
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => return Err(format!("wait failed: {error}")),
        }
    }
}

fn terminate(child: &mut Child) -> std::io::Result<ExitStatus> {
    let _ = child.kill();
    child.wait()
}

fn validate_completion(
    method: &str,
    timeout: Duration,
    max_output_bytes: usize,
    timed_out: bool,
    output_exceeded: &AtomicBool,
) -> Result<(), String> {
    if output_exceeded.load(Ordering::Acquire) {
        return Err(format!(
            "process.{method}: output exceeded {max_output_bytes}-byte cap"
        ));
    }
    if timed_out {
        return Err(format!(
            "process.{method}: timed out after {}ms",
            timeout.as_millis()
        ));
    }
    Ok(())
}

fn render_output(
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    encoding: OutputEncoding,
) -> Value {
    let (stdout, stderr) = match encoding {
        OutputEncoding::Utf8 => (
            Value::String(String::from_utf8_lossy(&stdout).into_owned()),
            Value::String(String::from_utf8_lossy(&stderr).into_owned()),
        ),
        OutputEncoding::Base64 => (
            Value::String(base64::engine::general_purpose::STANDARD.encode(stdout)),
            Value::String(base64::engine::general_purpose::STANDARD.encode(stderr)),
        ),
    };

    json!({
        "code": status.code(),
        "stdout": stdout,
        "stderr": stderr,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A program that stays alive ~30 s without reading stdin.
    fn sleeper() -> (String, Vec<String>) {
        if cfg!(windows) {
            (
                "ping".into(),
                vec!["-n".into(), "30".into(), "127.0.0.1".into()],
            )
        } else {
            ("sleep".into(), vec!["30".into()])
        }
    }

    fn request(command: String, argv: Vec<String>) -> ExecRequest {
        ExecRequest {
            command,
            argv,
            cwd: None,
            env: None,
            input: None,
            input_encoding: None,
            output_encoding: None,
            timeout_ms: None,
            max_output_bytes: None,
            shell: false,
        }
    }

    #[test]
    fn timeout_applies_even_when_child_never_reads_a_large_stdin() {
        let (command, argv) = sleeper();
        let mut req = request(command, argv);
        req.input = Some("x".repeat(4 * 1024 * 1024));
        req.timeout_ms = Some(500);
        let started = Instant::now();
        let error = exec_impl(req).expect_err("must time out");
        assert!(error.contains("timed out"), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn stdin_is_delivered_and_output_captured() {
        let (command, argv) = if cfg!(windows) {
            ("sort".to_string(), Vec::new())
        } else {
            ("cat".to_string(), Vec::new())
        };
        let mut req = request(command, argv);
        req.input = Some("hello".into());
        let value = exec_impl(req).unwrap();
        assert_eq!(value["code"], json!(0));
        assert!(value["stdout"].as_str().unwrap().contains("hello"));
    }

    #[test]
    fn output_cap_is_enforced() {
        let (command, argv) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".into(), r"dir /s C:\Windows".into()],
            )
        } else {
            ("yes".to_string(), Vec::new())
        };
        let mut req = request(command, argv);
        req.max_output_bytes = Some(1024);
        let error = exec_impl(req).expect_err("cap must trip");
        assert!(error.contains("output exceeded"), "{error}");
    }

    #[test]
    fn shell_mode_refuses_caller_env() {
        let mut req = request("git status".into(), Vec::new());
        req.shell = true;
        let mut env = Map::new();
        env.insert("X".into(), json!("status & calc"));
        req.env = Some(env);
        let error = validate_shell_request(&req).expect_err("env must be refused");
        assert!(error.contains("env"), "{error}");
    }

    #[test]
    fn shell_mode_refuses_extra_args() {
        let mut req = request("git".into(), vec!["status".into()]);
        req.shell = true;
        assert!(validate_shell_request(&req).is_err());
    }

    #[test]
    fn non_shell_mode_skips_the_shell_check() {
        let mut req = request("git".into(), vec!["status".into()]);
        let mut env = Map::new();
        env.insert("X".into(), json!("1"));
        req.env = Some(env);
        assert!(validate_shell_request(&req).is_ok());
    }

    #[test]
    fn join_output_until_returns_partial_output_for_a_held_pipe() {
        // A pipe whose write end never closes: one chunk, then block forever.
        struct Held(Option<Vec<u8>>, std::sync::mpsc::Receiver<()>);
        impl std::io::Read for Held {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                if let Some(chunk) = self.0.take() {
                    out[..chunk.len()].copy_from_slice(&chunk);
                    return Ok(chunk.len());
                }
                let _ = self.1.recv();
                Ok(0)
            }
        }
        let (_never_closed_tx, rx) = std::sync::mpsc::channel::<()>();
        let reader = read_limited_output(
            Held(Some(b"hello".to_vec()), rx),
            "stdout",
            1024,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let output = join_output_until(
            Some(reader),
            "stdout",
            Instant::now() + Duration::from_millis(200),
        )
        .expect("a held pipe must not fail the call");
        assert_eq!(output, b"hello");
    }

    #[cfg(windows)]
    #[test]
    fn shell_mode_does_not_run_a_program_planted_in_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tyndplanted.bat"), "@echo PLANTED\r\n").unwrap();
        let mut command = build_command("tyndplanted", &[], true);
        command.current_dir(dir.path());
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let output = command.output().unwrap();
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("PLANTED"),
            "planted batch file ran"
        );
    }

    #[cfg(windows)]
    #[test]
    fn shell_mode_disables_cwd_program_lookup_explicitly() {
        // The parent environment may already carry the variable; the child
        // must set it itself so the guarantee does not depend on the host env.
        let command = build_command("git status", &[], true);
        let value = command
            .get_envs()
            .find(|(key, _)| *key == "NoDefaultCurrentDirectoryInExePath")
            .and_then(|(_, value)| value);
        assert_eq!(value, Some(std::ffi::OsStr::new("1")));
    }

    #[cfg(unix)]
    #[test]
    fn shell_background_child_does_not_fail_the_call() {
        // Same pipeline as `exec_impl`, minus the allowlist policy.
        let mut command = build_command("sleep 5 &", &[], true);
        configure_stdio(&mut command, false);
        let mut child = command.spawn().unwrap();
        let exceeded = Arc::new(AtomicBool::new(false));
        let readers = start_output_readers(&mut child, 1024, &exceeded).unwrap();
        let outcome = wait_for_exit(
            &mut child,
            Instant::now() + Duration::from_secs(10),
            &exceeded,
        )
        .unwrap();
        assert!(outcome.status.success());
        let drain = Instant::now() + Duration::from_millis(300);
        assert!(join_output_until(readers.stdout, "stdout", drain).is_ok());
        assert!(join_output_until(readers.stderr, "stderr", drain).is_ok());
    }
}

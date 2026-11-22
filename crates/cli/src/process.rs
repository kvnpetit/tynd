use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::{OsStr, OsString},
    fmt,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};

pub(crate) const DEFAULT_MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// Process execution policy stays flat so call sites override one concern at a time.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
pub(crate) struct CommandSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<OsString, Option<OsString>>,
    pub passthrough_sensitive_env: bool,
    pub timeout: Option<Duration>,
    pub max_output_bytes: usize,
    pub allow_failure: bool,
    pub secret_args: BTreeSet<usize>,
    pub pipe_stdin: bool,
}

impl CommandSpec {
    pub(crate) fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            passthrough_sensitive_env: false,
            timeout: None,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            allow_failure: false,
            secret_args: BTreeSet::new(),
            pipe_stdin: false,
        }
    }

    pub(crate) fn arg(mut self, argument: impl Into<OsString>) -> Self {
        self.args.push(argument.into());
        self
    }

    pub(crate) fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(arguments.into_iter().map(Into::into));
        self
    }

    pub(crate) fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub(crate) fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.insert(key.into(), Some(value.into()));
        self
    }

    pub(crate) fn env_remove(mut self, key: impl Into<OsString>) -> Self {
        self.env.insert(key.into(), None);
        self
    }

    pub(crate) fn passthrough_sensitive_env(mut self, passthrough: bool) -> Self {
        self.passthrough_sensitive_env = passthrough;
        self
    }

    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub(crate) fn without_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }

    pub(crate) fn max_output_bytes(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }

    pub(crate) fn allow_failure(mut self, allow: bool) -> Self {
        self.allow_failure = allow;
        self
    }

    pub(crate) fn secret_arg(mut self, index: usize) -> Self {
        self.secret_args.insert(index);
        self
    }

    pub(crate) fn piped_stdin(mut self) -> Self {
        self.pipe_stdin = true;
        self
    }

    pub(crate) fn redacted(&self) -> String {
        redact_command(&self.program, &self.args, &self.secret_args)
    }
}

#[derive(Debug)]
pub(crate) struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl ProcessOutput {
    pub(crate) fn success(&self) -> bool {
        self.status.success()
    }

    pub(crate) fn exit_code(&self) -> i32 {
        self.status.code().unwrap_or(-1)
    }

    pub(crate) fn stdout_trimmed(&self) -> &str {
        self.stdout.trim()
    }
}

pub(crate) fn run<P, I, S>(program: P, args: I, cwd: Option<&Path>) -> Result<ProcessOutput>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_spec(plain_spec(program, args, cwd))
}

/// Default policy spec for `program args...`, optionally run from `cwd`.
fn plain_spec<P, I, S>(program: P, args: I, cwd: Option<&Path>) -> CommandSpec
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut spec = CommandSpec::new(program.as_ref().to_os_string());
    spec.args = args
        .into_iter()
        .map(|argument| argument.as_ref().to_os_string())
        .collect();
    spec.cwd = cwd.map(Path::to_path_buf);
    spec
}

pub(crate) fn run_spec(spec: CommandSpec) -> Result<ProcessOutput> {
    if spec.max_output_bytes == 0 {
        bail!("max_output_bytes must be greater than zero");
    }

    let label = spec.redacted();
    crate::output::debug(format!("run {label}"));
    let started = Instant::now();
    let mut child = spawn_captured(&spec, &label)?;

    let stdout = child.stdout.take().context("subprocess stdout missing")?;
    let stderr = child.stderr.take().context("subprocess stderr missing")?;
    let (limit_tx, limit_rx) = mpsc::channel::<&'static str>();
    let stdout_handle = capture(stdout, spec.max_output_bytes, "stdout", limit_tx.clone());
    let stderr_handle = capture(stderr, spec.max_output_bytes, "stderr", limit_tx);
    let deadline = spec.timeout.map(|timeout| started + timeout);

    let status = loop {
        if let Ok(stream) = limit_rx.try_recv() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            bail!("{label} {stream} exceeded {} bytes", spec.max_output_bytes);
        }
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("wait for {label}"))?
        {
            break status;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            let _ = child.kill();
            let status = child
                .wait()
                .with_context(|| format!("terminate timed-out process {label}"))?;
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            bail!(
                "{label} timed out after {} ms (exit {})",
                spec.timeout.unwrap_or_default().as_millis(),
                status.code().unwrap_or(-1)
            );
        }
        thread::sleep(Duration::from_millis(10));
    };

    let stdout_bytes = join_capture(stdout_handle, "stdout", &label)?;
    let stderr_bytes = join_capture(stderr_handle, "stderr", &label)?;
    let output = ProcessOutput {
        status,
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
    };
    if !output.success() && !spec.allow_failure {
        let diagnostic = output.stderr.trim();
        bail!(
            "{label} exited with code {}{}",
            output.exit_code(),
            if diagnostic.is_empty() {
                String::new()
            } else {
                format!("\n{diagnostic}")
            }
        );
    }
    Ok(output)
}

/// Spawn `spec` with piped stdout/stderr and a `null` stdin.
fn spawn_captured(spec: &CommandSpec, label: &str) -> Result<Child> {
    let mut command = build_command(spec);
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    command
        .spawn()
        .with_context(|| format!("failed to start {label}"))
}

pub(crate) fn run_streaming(spec: CommandSpec) -> Result<ExitStatus> {
    let label = spec.redacted();
    let started = Instant::now();
    let timeout = spec.timeout;
    let allow_failure = spec.allow_failure;
    let mut child = spawn_spec(spec)?;
    let deadline = timeout.map(|timeout| started + timeout);
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("wait for {label}"))?
        {
            break status;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{label} timed out after {} ms",
                timeout.unwrap_or_default().as_millis()
            );
        }
        thread::sleep(Duration::from_millis(25));
    };
    if !status.success() && !allow_failure {
        bail!("{label} exited with code {}", status.code().unwrap_or(-1));
    }
    Ok(status)
}

pub(crate) fn spawn_spec(spec: CommandSpec) -> Result<Child> {
    let label = spec.redacted();
    crate::output::debug(format!("spawn {label}"));
    let mut command = build_command(&spec);
    command
        .stdin(if spec.pipe_stdin {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .stdout(if crate::output::quiet() {
            Stdio::null()
        } else {
            Stdio::inherit()
        })
        .stderr(Stdio::inherit());
    command
        .spawn()
        .with_context(|| format!("failed to start {label}"))
}

pub(crate) fn find_executable(program: impl AsRef<OsStr>) -> Option<PathBuf> {
    let program = Path::new(program.as_ref());
    if program.components().count() > 1 || program.is_absolute() {
        return executable_file(program).then(|| program.to_path_buf());
    }
    let path = env::var_os("PATH")?;
    let extensions = executable_extensions();
    for directory in env::split_paths(&path) {
        let candidate = directory.join(program);
        if executable_file(&candidate) {
            return Some(candidate);
        }
        if candidate.extension().is_none() {
            for extension in &extensions {
                let candidate = candidate.with_extension(extension);
                if executable_file(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

pub(crate) fn sanitized_environment() -> BTreeMap<OsString, OsString> {
    env::vars_os()
        .filter(|(key, _)| !is_sensitive_env_key(key))
        .collect()
}

pub(crate) fn is_sensitive_env_key(key: impl AsRef<OsStr>) -> bool {
    let name = key.as_ref().to_string_lossy().to_ascii_uppercase();
    name.starts_with("R2_")
        || name.starts_with("AWS_")
        || name == "TYND_MACOS_P12_PASSWORD"
        || (name.starts_with("TYND_") && name.ends_with("_TOKEN"))
        || name == "GITHUB_TOKEN"
        || name == "NPM_TOKEN"
        || name == "NODE_OPTIONS"
        || name == "NODE_PATH"
}

pub(crate) fn redact_command(
    program: &OsStr,
    arguments: &[OsString],
    secret_indices: &BTreeSet<usize>,
) -> String {
    let mut rendered = shell_quote(program);
    for (index, argument) in arguments.iter().enumerate() {
        rendered.push(' ');
        if secret_indices.contains(&index) || looks_like_inline_secret(argument) {
            rendered.push_str("***");
        } else {
            rendered.push_str(&shell_quote(argument));
        }
    }
    rendered
}

fn build_command(spec: &CommandSpec) -> Command {
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    if !spec.passthrough_sensitive_env {
        for (key, _) in env::vars_os() {
            if is_sensitive_env_key(&key) {
                command.env_remove(key);
            }
        }
    }
    for (key, value) in &spec.env {
        match value {
            Some(value) => {
                command.env(key, value);
            },
            None => {
                command.env_remove(key);
            },
        }
    }
    command
}

fn capture<R>(
    mut reader: R,
    limit: usize,
    label: &'static str,
    limit_tx: mpsc::Sender<&'static str>,
) -> thread::JoinHandle<std::io::Result<Vec<u8>>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = Vec::with_capacity(limit.min(64 * 1024));
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                return Ok(output);
            }
            if output.len().saturating_add(count) > limit {
                let _ = limit_tx.send(label);
                return Ok(output);
            }
            output.extend_from_slice(&buffer[..count]);
        }
    })
}

fn join_capture(
    handle: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    stream: &str,
    command: &str,
) -> Result<Vec<u8>> {
    handle
        .join()
        .map_err(|_| anyhow::anyhow!("{command} {stream} reader panicked"))?
        .with_context(|| format!("read {stream} from {command}"))
}

fn looks_like_inline_secret(value: &OsStr) -> bool {
    let value = value.to_string_lossy().to_ascii_lowercase();
    [
        "--password=",
        "--token=",
        "--secret=",
        "--secret-key=",
        "--access-key=",
        "/p:",
    ]
    .iter()
    .any(|prefix| value.starts_with(prefix))
}

fn shell_quote(value: &OsStr) -> String {
    let value = value.to_string_lossy();
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_-./:\\@".contains(character))
    {
        return value.into_owned();
    }
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn executable_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|metadata| metadata.is_file())
}

fn executable_extensions() -> Vec<String> {
    #[cfg(windows)]
    {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned())
            .split(';')
            .filter_map(|extension| {
                let extension = extension.trim().trim_start_matches('.');
                (!extension.is_empty()).then(|| extension.to_ascii_lowercase())
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

impl fmt::Display for ProcessOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.stdout)
    }
}

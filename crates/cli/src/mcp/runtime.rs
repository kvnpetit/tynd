use std::{
    collections::HashMap,
    ffi::OsString,
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use super::{
    bridge::DebugClient,
    catalog::Catalog,
    discovery::{pick_newest, scan_live, DiscoveryEntry, ScanOptions},
};

const LAUNCH_COOLDOWN: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent MCP security and discovery switches map directly to CLI flags"
)]
pub(super) struct RuntimeConfig {
    pub(super) allow_eval: bool,
    pub(super) allow_launch: bool,
    pub(super) auto_switch: bool,
    pub(super) allow_non_loopback: bool,
    pub(super) app_id: Option<String>,
    pub(super) rescan_interval: Duration,
    pub(super) connect_timeout: Duration,
}

impl RuntimeConfig {
    /// Discovery scan of the default directory, honouring the loopback gate.
    fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            allow_non_loopback: self.allow_non_loopback,
            ..ScanOptions::default()
        }
    }
}

#[derive(Debug)]
pub(super) struct Runtime {
    pub(super) config: RuntimeConfig,
    pub(super) catalog: Catalog,
    client: Option<DebugClient>,
    entry: Option<DiscoveryEntry>,
    last_scan: Option<Instant>,
    launch_until: Option<Instant>,
}

impl Runtime {
    pub(super) fn new(config: RuntimeConfig) -> Self {
        Self {
            config,
            catalog: Catalog::embedded(),
            client: None,
            entry: None,
            last_scan: None,
            launch_until: None,
        }
    }

    pub(super) fn attached_entry(&self) -> Option<&DiscoveryEntry> {
        self.entry.as_ref()
    }

    pub(super) fn refresh(&mut self, force: bool) -> Result<bool> {
        if !force
            && self
                .last_scan
                .is_some_and(|last| last.elapsed() < self.config.rescan_interval)
        {
            return Ok(self.client.is_some());
        }
        self.last_scan = Some(Instant::now());
        let entries = scan_live(&self.config.scan_options())?;
        let candidate = pick_newest(&entries, self.config.app_id.as_deref()).cloned();
        let should_replace = match (&self.entry, &candidate) {
            (None, Some(_)) => true,
            (Some(current), Some(next)) if self.config.auto_switch => {
                next.started_at_ms > current.started_at_ms
            },
            _ => false,
        };
        if should_replace {
            let target = candidate.expect("candidate exists when replacing");
            match DebugClient::connect(&target, self.config.connect_timeout) {
                Ok(client) => {
                    tracing::info!(app_id = %target.app_id, pid = target.pid, "MCP attached");
                    self.client = Some(client);
                    self.entry = Some(target);
                },
                Err(error) => {
                    tracing::warn!(app_id = %target.app_id, "MCP attach failed: {error:#}");
                    if self.client.is_none() {
                        self.entry = None;
                    }
                },
            }
        }
        Ok(self.client.is_some())
    }

    pub(super) fn request_live(&mut self, method: &str, params: Value) -> Result<Value> {
        self.refresh(false)?;
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| anyhow!("{method} requires an attached app — start `tynd dev`"))?;
        match client.request(method, params) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.client = None;
                self.entry = None;
                self.last_scan = None;
                Err(error)
            },
        }
    }

    pub(super) fn request_if_attached(
        &mut self,
        method: &str,
        params: Value,
    ) -> Option<Result<Value>> {
        if self.refresh(false).ok()? {
            Some(self.request_live(method, params))
        } else {
            None
        }
    }

    pub(super) fn launch(&mut self, project: &str) -> Result<Value> {
        if !self.config.allow_launch {
            bail!(
                "app.launch is disabled. Start the server with `--allow-launch` to enable spawning tynd."
            );
        }
        if self
            .launch_until
            .is_some_and(|until| Instant::now() < until)
        {
            bail!("app.launch already in flight; wait for the previous launch to settle");
        }
        let project = validate_project_path(project)?;
        let binary = crate::project::resolve_binary(crate::project::TyndBinary::Tynd)?;
        let mut command = Command::new(&binary);
        command
            .arg("dev")
            .current_dir(&project)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env_clear()
            .envs(filtered_environment());
        let mut child = command
            .spawn()
            .with_context(|| format!("launch {} dev", binary.display()))?;
        let pid = child.id();
        std::thread::Builder::new()
            .name(format!("tynd-mcp-launch-{pid}"))
            .spawn(move || match child.wait() {
                Ok(status) => tracing::debug!(pid, %status, "launched Tynd app exited"),
                Err(error) => tracing::warn!(pid, "failed to reap launched Tynd app: {error}"),
            })
            .context("start launched-app reaper")?;
        self.launch_until = Some(Instant::now() + LAUNCH_COOLDOWN);
        Ok(json!({ "ok": true, "pid": pid, "bin": binary }))
    }

    pub(super) fn discovery_status(&mut self) -> Value {
        match scan_live(&self.config.scan_options()) {
            Ok(entries) => json!({
                "ok": true,
                "found": entries.len(),
                "attached": self.entry.as_ref().map(entry_json),
            }),
            Err(error) => json!({
                "ok": false,
                "error": error.to_string(),
                "attached": self.entry.as_ref().map(entry_json),
            }),
        }
    }
}

pub(super) fn connect_newest(config: &RuntimeConfig) -> Result<(DiscoveryEntry, DebugClient)> {
    let entries = scan_live(&config.scan_options())?;
    let entry = pick_newest(&entries, config.app_id.as_deref())
        .cloned()
        .ok_or_else(|| anyhow!("no live tynd dev app found — start `tynd dev` first"))?;
    let client = DebugClient::connect(&entry, config.connect_timeout)?;
    Ok((entry, client))
}

pub(super) fn sanitize_json(value: Value) -> Value {
    match value {
        Value::String(text) => {
            Value::String(super::bridge::sanitize_payload_text(&strip_ansi(&text)))
        },
        Value::Array(values) => Value::Array(values.into_iter().map(sanitize_json).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    (
                        super::bridge::sanitize_payload_text(&strip_ansi(&key)),
                        sanitize_json(value),
                    )
                })
                .collect(),
        ),
        other => other,
    }
}

fn entry_json(entry: &DiscoveryEntry) -> Value {
    json!({
        "appId": entry.app_id,
        "pid": entry.pid,
        "transport": match entry.transport {
            super::discovery::TransportKind::Uds => "uds",
            super::discovery::TransportKind::Tcp => "tcp",
        },
        "startedAt": entry.started_at_ms,
        "tyndVersion": entry.tynd_version,
    })
}

fn validate_project_path(raw: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        bail!("app.launch requires an absolute path: {raw}");
    }
    #[cfg(windows)]
    {
        let raw = raw.replace('/', "\\");
        if raw.starts_with(r"\\?\") || raw.starts_with(r"\\.\") {
            bail!("app.launch refuses Windows device path: {raw}");
        }
        if raw.starts_with(r"\\") {
            bail!("app.launch refuses UNC path: {raw}");
        }
    }
    let metadata = fs::metadata(&path)
        .with_context(|| format!("app.launch path does not exist: {}", path.display()))?;
    if !metadata.is_dir() {
        bail!("app.launch path is not a directory: {}", path.display());
    }
    Ok(path)
}

fn filtered_environment() -> HashMap<OsString, OsString> {
    std::env::vars_os()
        .filter(|(key, _)| {
            let key = key.to_string_lossy().to_ascii_uppercase();
            key != "NODE_OPTIONS"
                && key != "NODE_PATH"
                && key != "DEBUG"
                && !key.starts_with("R2_")
                && !key.starts_with("AWS_")
                && ![
                    "TOKEN",
                    "SECRET",
                    "PASSWORD",
                    "PRIVATE_KEY",
                    "ACCESS_KEY",
                    "API_KEY",
                    "CREDENTIAL",
                ]
                .iter()
                .any(|sensitive| key.contains(sensitive))
        })
        .collect()
}

fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0x1b {
            let character = input[index..].chars().next().expect("valid UTF-8");
            output.push(character);
            index += character.len_utf8();
            continue;
        }
        index += 1;
        if index >= bytes.len() {
            break;
        }
        if bytes[index] == b'[' {
            index += 1;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
        } else if bytes[index] == b']' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == 0x07 {
                    index += 1;
                    break;
                }
                if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                    index += 2;
                    break;
                }
                index += 1;
            }
        } else if bytes[index].is_ascii() {
            // Two-byte escape (`ESC c`). A non-ASCII byte starts a multi-byte
            // character: skipping one byte would land inside it and make the
            // next slice panic, so the character is kept instead.
            index += 1;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn strips_csi_and_osc_sequences() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m ok"), "red ok");
        assert_eq!(strip_ansi("a\x1b]0;title\x07b"), "ab");
        assert_eq!(strip_ansi("a\x1b]0;title\x1b\\b"), "ab");
        assert_eq!(strip_ansi("dangling\x1b"), "dangling");
    }

    #[test]
    fn keeps_multibyte_characters_after_an_escape() {
        assert_eq!(strip_ansi("x\x1b\u{e9}y"), "x\u{e9}y");
        assert_eq!(strip_ansi("\x1b\u{1f600}!"), "\u{1f600}!");
        assert_eq!(strip_ansi("\x1bcrest"), "rest");
    }
}

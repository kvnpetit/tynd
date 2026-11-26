use std::{
    fs::{self, File, Metadata},
    io::Read,
    net::IpAddr,
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

const MAX_DISCOVERY_FILE_BYTES: u64 = 64 * 1024;
const MAX_DISCOVERY_FILES: usize = 1024;
const MAX_FIELD_LEN: usize = 256;
const DEFAULT_MAX_AGE: Duration = Duration::from_hours(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiscoveryTrust {
    UidScoped,
    #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
    SharedTmp,
}

#[derive(Clone, Debug)]
pub(super) struct DiscoveryEntry {
    pub(super) app_id: String,
    pub(super) pid: u32,
    pub(super) socket: String,
    pub(super) transport: TransportKind,
    pub(super) cookie: Option<String>,
    pub(super) started_at_ms: i64,
    pub(super) tynd_version: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TransportKind {
    Uds,
    Tcp,
}

impl TransportKind {
    /// TCP and Windows named-pipe endpoints must complete the discovery cookie
    /// handshake; Unix sockets do not.
    pub(super) fn requires_cookie(self) -> bool {
        self == Self::Tcp || (cfg!(windows) && self == Self::Uds)
    }
}

#[derive(Clone, Debug)]
pub(super) struct ScanOptions {
    pub(super) dir: Option<PathBuf>,
    pub(super) trust: Option<DiscoveryTrust>,
    pub(super) max_age: Duration,
    pub(super) allow_non_loopback: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            dir: None,
            trust: None,
            max_age: DEFAULT_MAX_AGE,
            allow_non_loopback: false,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostPayload {
    pid: u32,
    app_id: String,
    #[serde(default)]
    app_version: Option<String>,
    endpoint: HostEndpoint,
    started_at: String,
    #[serde(default)]
    cookie: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HostEndpoint {
    #[serde(default)]
    unix: Option<String>,
    #[serde(default)]
    tcp: Option<u16>,
    #[serde(default)]
    host: Option<String>,
}

pub(super) fn scan_live(options: &ScanOptions) -> Result<Vec<DiscoveryEntry>> {
    let (dir, trust) = match &options.dir {
        Some(dir) => (
            dir.clone(),
            options.trust.unwrap_or(DiscoveryTrust::UidScoped),
        ),
        None => resolve_discovery_dir()?,
    };
    if !dir.is_absolute() {
        return Err(anyhow!(
            "discovery dir resolved to non-absolute path: {}",
            dir.display()
        ));
    }
    if !validate_directory(&dir, trust)? {
        return Ok(Vec::new());
    }

    let Some(mut names) = list_json_files(&dir)? else {
        return Ok(Vec::new());
    };
    if names.len() > MAX_DISCOVERY_FILES {
        tracing::warn!(
            "discovery: refusing {} JSON files in {}; cap is {}",
            names.len(),
            dir.display(),
            MAX_DISCOVERY_FILES
        );
        return Ok(Vec::new());
    }

    names.sort();
    let now_ms = unix_millis(SystemTime::now());
    let max_age_ms = i64::try_from(options.max_age.as_millis()).unwrap_or(i64::MAX);
    let mut out = Vec::new();
    for path in names {
        match read_discovery_file(&path, trust, options.allow_non_loopback) {
            Ok(Some(entry))
                if now_ms.saturating_sub(entry.started_at_ms) <= max_age_ms
                    && process_is_alive(entry.pid, trust) =>
            {
                out.push(entry);
            },
            Ok(_) => {},
            Err(error) => tracing::debug!("discovery: skip {}: {error:#}", path.display()),
        }
    }
    out.sort_by_key(|entry| entry.started_at_ms);
    Ok(out)
}

/// Paths of the `*.json` entries of `dir`, or `None` when it does not exist.
/// Unreadable entries are skipped.
fn list_json_files(dir: &Path) -> Result<Option<Vec<PathBuf>>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("read discovery dir {}", dir.display()));
        },
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!("discovery entry read failed: {error}");
                continue;
            },
        };
        if entry.file_name().to_string_lossy().ends_with(".json") {
            names.push(entry.path());
        }
    }
    Ok(Some(names))
}

pub(super) fn pick_newest<'a>(
    entries: &'a [DiscoveryEntry],
    app_id: Option<&str>,
) -> Option<&'a DiscoveryEntry> {
    entries
        .iter()
        .filter(|entry| app_id.is_none_or(|wanted| entry.app_id == wanted))
        .max_by_key(|entry| entry.started_at_ms)
}

pub(super) fn resolve_discovery_dir() -> Result<(PathBuf, DiscoveryTrust)> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("home directory unavailable"))?;
        return Ok((
            home.join("Library/Application Support/tynd/debug"),
            DiscoveryTrust::UidScoped,
        ));
    }
    #[cfg(windows)]
    {
        let base = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| anyhow!("local application data directory unavailable"))?;
        return Ok((base.join("tynd/debug"), DiscoveryTrust::UidScoped));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR") {
            let xdg = PathBuf::from(xdg);
            if xdg.is_absolute() {
                return Ok((xdg.join("tynd"), DiscoveryTrust::UidScoped));
            }
        }
        return Ok((
            std::env::temp_dir().join(format!("tynd-{}", current_uid())),
            DiscoveryTrust::SharedTmp,
        ));
    }
    #[allow(unreachable_code)]
    Err(anyhow!("unsupported platform"))
}

fn validate_directory(dir: &Path, trust: DiscoveryTrust) -> Result<bool> {
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).with_context(|| format!("stat {}", dir.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        tracing::warn!("discovery: refusing non-directory {}", dir.display());
        return Ok(false);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o777 != 0o700 {
            tracing::warn!(
                "discovery: {} mode is 0{:o}; refusing (need 0700)",
                dir.display(),
                metadata.mode() & 0o777
            );
            return Ok(false);
        }
        let uid = current_uid();
        if metadata.uid() != uid {
            tracing::warn!(
                "discovery: {} owned by uid {}, not current uid {uid}",
                dir.display(),
                metadata.uid()
            );
            return Ok(false);
        }
    }
    #[cfg(windows)]
    if trust == DiscoveryTrust::SharedTmp {
        tracing::warn!(
            "discovery: {} is shared; cookie handshake remains mandatory",
            dir.display()
        );
    }
    let _ = trust;
    Ok(true)
}

fn read_discovery_file(
    path: &Path,
    trust: DiscoveryTrust,
    allow_non_loopback: bool,
) -> Result<Option<DiscoveryEntry>> {
    let before = fs::symlink_metadata(path)?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Ok(None);
    }
    if before.len() > MAX_DISCOVERY_FILE_BYTES {
        tracing::warn!(
            "discovery: skip {}: {} bytes exceeds cap {}",
            path.display(),
            before.len(),
            MAX_DISCOVERY_FILE_BYTES
        );
        return Ok(None);
    }
    if !owned_by_current_user(&before, trust) {
        tracing::warn!("discovery: skip {}: foreign owner", path.display());
        return Ok(None);
    }

    let file = File::open(path)?;
    let opened = file.metadata()?;
    if !same_file(&before, &opened) || opened.len() > MAX_DISCOVERY_FILE_BYTES {
        return Ok(None);
    }
    let mut raw = String::new();
    file.take(MAX_DISCOVERY_FILE_BYTES + 1)
        .read_to_string(&mut raw)?;
    if raw.len() as u64 > MAX_DISCOVERY_FILE_BYTES {
        return Ok(None);
    }
    let payload: HostPayload = serde_json::from_str(&raw)?;
    let app_id = sanitize_field(&payload.app_id);
    if payload.pid == 0 || app_id.is_empty() {
        return Ok(None);
    }

    let (socket, transport) = resolve_endpoint(&payload.endpoint, allow_non_loopback)?;
    let requires_cookie = transport.requires_cookie();
    let cookie = payload.cookie.filter(|value| !value.is_empty());
    let cookie = match cookie {
        Some(cookie) if valid_cookie(&cookie) => Some(cookie),
        _ if requires_cookie => return Ok(None),
        _ => None,
    };

    let started_at_ms = parse_rfc3339_millis(&payload.started_at)?;
    Ok(Some(DiscoveryEntry {
        app_id,
        pid: payload.pid,
        socket,
        transport,
        cookie,
        started_at_ms,
        tynd_version: sanitize_field(payload.app_version.as_deref().unwrap_or("unknown")),
    }))
}

fn resolve_endpoint(
    endpoint: &HostEndpoint,
    allow_non_loopback: bool,
) -> Result<(String, TransportKind)> {
    match (&endpoint.unix, endpoint.tcp) {
        (Some(path), None) if !path.is_empty() => Ok((path.clone(), TransportKind::Uds)),
        (None, Some(port)) if port > 0 => {
            let host = endpoint.host.as_deref().unwrap_or("127.0.0.1");
            if !allow_non_loopback && !is_loopback(host) {
                return Err(anyhow!(
                    "non-loopback discovery host {host} requires explicit opt-in"
                ));
            }
            let host = host.trim_matches(['[', ']']);
            let socket = if host.contains(':') {
                format!("[{host}]:{port}")
            } else {
                format!("{host}:{port}")
            };
            Ok((socket, TransportKind::Tcp))
        },
        _ => Err(anyhow!(
            "discovery endpoint must contain exactly one of unix or tcp"
        )),
    }
}

fn is_loopback(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => address.is_loopback(),
        Ok(IpAddr::V6(address)) => {
            address.is_loopback()
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| mapped.is_loopback())
        },
        Err(_) => false,
    }
}

fn valid_cookie(cookie: &str) -> bool {
    (32..=256).contains(&cookie.len())
        && cookie.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'_' | b'-')
        })
}

fn sanitize_field(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_FIELD_LEN)
        .collect()
}

fn unix_millis(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(error) => -i64::try_from(error.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

fn parse_rfc3339_millis(value: &str) -> Result<i64> {
    let (date, rest) = value
        .split_once('T')
        .or_else(|| value.split_once('t'))
        .ok_or_else(|| anyhow!("startedAt is not RFC3339: {value}"))?;
    let mut date_parts = date.split('-');
    let year = parse_part::<i64>(date_parts.next(), "year")?;
    let month = parse_part::<u32>(date_parts.next(), "month")?;
    let day = parse_part::<u32>(date_parts.next(), "day")?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) {
        return Err(anyhow!("startedAt date invalid: {value}"));
    }
    let max_day = days_in_month(year, month);
    if day == 0 || day > max_day {
        return Err(anyhow!("startedAt day invalid: {value}"));
    }

    let (clock, offset_seconds) = split_clock_offset(rest)?;
    let mut clock_parts = clock.split(':');
    let hour = parse_part::<u32>(clock_parts.next(), "hour")?;
    let minute = parse_part::<u32>(clock_parts.next(), "minute")?;
    let seconds_raw = clock_parts
        .next()
        .ok_or_else(|| anyhow!("startedAt second missing"))?;
    if clock_parts.next().is_some() || hour > 23 || minute > 59 {
        return Err(anyhow!("startedAt clock invalid: {value}"));
    }
    let (second_text, fraction) = seconds_raw.split_once('.').unwrap_or((seconds_raw, ""));
    let second: u32 = second_text.parse().context("startedAt second invalid")?;
    if second > 60 {
        return Err(anyhow!("startedAt second invalid: {value}"));
    }
    let millis = fraction_millis(fraction)?;
    let days = days_from_civil(year, month, day);
    let seconds = days
        .saturating_mul(86_400)
        .saturating_add(i64::from(hour) * 3_600)
        .saturating_add(i64::from(minute) * 60)
        .saturating_add(i64::from(second))
        .saturating_sub(offset_seconds);
    Ok(seconds
        .saturating_mul(1_000)
        .saturating_add(i64::from(millis)))
}

fn split_clock_offset(value: &str) -> Result<(&str, i64)> {
    if let Some(clock) = value.strip_suffix('Z').or_else(|| value.strip_suffix('z')) {
        return Ok((clock, 0));
    }
    let split = value
        .char_indices()
        .rev()
        .find(|(index, character)| *index > 0 && matches!(character, '+' | '-'))
        .ok_or_else(|| anyhow!("startedAt timezone missing"))?;
    let (clock, offset) = value.split_at(split.0);
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let offset = &offset[1..];
    let (hours, minutes) = offset
        .split_once(':')
        .ok_or_else(|| anyhow!("startedAt timezone invalid"))?;
    let hours: i64 = hours.parse().context("startedAt timezone hour invalid")?;
    let minutes: i64 = minutes
        .parse()
        .context("startedAt timezone minute invalid")?;
    if hours > 23 || minutes > 59 {
        return Err(anyhow!("startedAt timezone invalid"));
    }
    Ok((clock, sign * (hours * 3_600 + minutes * 60)))
}

fn fraction_millis(fraction: &str) -> Result<u32> {
    if fraction.is_empty() {
        return Ok(0);
    }
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(anyhow!("startedAt fractional second invalid"));
    }
    let mut digits = fraction.bytes().take(3).collect::<Vec<_>>();
    while digits.len() < 3 {
        digits.push(b'0');
    }
    let text = String::from_utf8(digits).context("startedAt fractional second invalid")?;
    text.parse().context("startedAt fractional second invalid")
}

fn parse_part<T>(value: Option<&str>, name: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    value
        .ok_or_else(|| anyhow!("startedAt {name} missing"))?
        .parse()
        .with_context(|| format!("startedAt {name} invalid"))
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

// Howard Hinnant's civil-date conversion, returning days since 1970-01-01.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = i64::from(month);
    let day = i64::from(day);
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * shifted_month + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(unix)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.uid() == right.uid()
        && left.len() == right.len()
}

#[cfg(not(unix))]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    left.len() == right.len()
        && left.created().ok() == right.created().ok()
        && left.modified().ok() == right.modified().ok()
}

#[cfg(unix)]
fn owned_by_current_user(metadata: &Metadata, _trust: DiscoveryTrust) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.uid() == current_uid()
}

#[cfg(not(unix))]
fn owned_by_current_user(_metadata: &Metadata, _trust: DiscoveryTrust) -> bool {
    true
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    // SAFETY: getuid has no preconditions and does not dereference pointers.
    unsafe { getuid() }
}

#[cfg(unix)]
fn process_is_alive(pid: u32, trust: DiscoveryTrust) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 performs existence/permission probing and sends no signal.
    if unsafe { kill(pid, 0) } == 0 {
        return true;
    }
    let permission_denied = std::io::Error::last_os_error().raw_os_error() == Some(1);
    permission_denied && trust == DiscoveryTrust::UidScoped
}

#[cfg(windows)]
fn process_is_alive(pid: u32, trust: DiscoveryTrust) -> bool {
    use std::ffi::c_void;

    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    // SAFETY: OpenProcess receives a scalar pid and returns an owned kernel handle.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        let permission_denied = std::io::Error::last_os_error().raw_os_error() == Some(5);
        return permission_denied && trust == DiscoveryTrust::UidScoped;
    }
    // SAFETY: handle came from successful OpenProcess and is closed exactly once.
    let _ = unsafe { CloseHandle(handle) };
    true
}

#[cfg(not(any(unix, windows)))]
fn process_is_alive(_pid: u32, _trust: DiscoveryTrust) -> bool {
    true
}

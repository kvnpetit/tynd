use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use tynd_debug_protocol::protocol::{write_frame, MAX_FRAME_BYTES};

use super::discovery::{DiscoveryEntry, TransportKind};

const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(15);
const NOTIFICATION_POLL_TIMEOUT: Duration = Duration::from_millis(250);

trait ReadWrite: Read + Write + Send {
    fn set_notification_polling(&self, enabled: bool) -> io::Result<()>;
}

impl ReadWrite for TcpStream {
    fn set_notification_polling(&self, enabled: bool) -> io::Result<()> {
        let timeout = if enabled {
            NOTIFICATION_POLL_TIMEOUT
        } else {
            DEFAULT_RPC_TIMEOUT
        };
        self.set_read_timeout(Some(timeout))
    }
}

#[cfg(feature = "local-socket")]
impl ReadWrite for interprocess::local_socket::Stream {
    fn set_notification_polling(&self, enabled: bool) -> io::Result<()> {
        use interprocess::local_socket::traits::Stream as _;

        self.set_nonblocking(enabled)
    }
}

#[derive(Debug, Default)]
struct FrameReader {
    header: [u8; 4],
    header_read: usize,
    body: Vec<u8>,
    body_read: usize,
}

impl FrameReader {
    fn read(&mut self, stream: &mut dyn ReadWrite) -> io::Result<Option<Vec<u8>>> {
        while self.header_read < self.header.len() {
            match stream.read(&mut self.header[self.header_read..]) {
                Ok(0) if self.header_read == 0 => return Ok(None),
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "debug frame header was truncated",
                    ));
                },
                Ok(read) => self.header_read += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
                Err(error) => return Err(error),
            }
        }

        if self.body.is_empty() {
            let length = u32::from_le_bytes(self.header);
            if length > MAX_FRAME_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("debug frame exceeds {MAX_FRAME_BYTES} bytes"),
                ));
            }
            self.body.resize(length as usize, 0);
        }

        while self.body_read < self.body.len() {
            match stream.read(&mut self.body[self.body_read..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "debug frame body was truncated",
                    ));
                },
                Ok(read) => self.body_read += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
                Err(error) => return Err(error),
            }
        }

        self.header_read = 0;
        self.body_read = 0;
        Ok(Some(std::mem::take(&mut self.body)))
    }
}

#[derive(Debug)]
pub(super) struct HostNotification {
    pub(super) method: String,
    pub(super) params: Value,
}

pub(super) struct DebugClient {
    stream: Box<dyn ReadWrite>,
    reader: FrameReader,
    next_id: u64,
    notifications: VecDeque<HostNotification>,
}

impl std::fmt::Debug for DebugClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DebugClient")
            .field("next_id", &self.next_id)
            .field("queued_notifications", &self.notifications.len())
            .finish_non_exhaustive()
    }
}

impl DebugClient {
    pub(super) fn connect(entry: &DiscoveryEntry, connect_timeout: Duration) -> Result<Self> {
        let stream = match entry.transport {
            TransportKind::Tcp => connect_tcp(&entry.socket, connect_timeout)?,
            TransportKind::Uds => connect_local(&entry.socket)?,
        };
        let mut client = Self {
            stream,
            reader: FrameReader::default(),
            next_id: 1,
            notifications: VecDeque::new(),
        };
        if entry.transport.requires_cookie() {
            let cookie = entry
                .cookie
                .as_deref()
                .ok_or_else(|| anyhow!("debug transport requires discovery cookie"))?;
            client.handshake(cookie)?;
        }
        Ok(client)
    }

    pub(super) fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.to_string();
        self.next_id = self.next_id.saturating_add(1);
        self.request_with_id(Value::String(id), method, params)
    }

    pub(super) fn read_notification(&mut self) -> Result<Option<HostNotification>> {
        if let Some(notification) = self.notifications.pop_front() {
            return Ok(Some(notification));
        }
        loop {
            let Some(message) = self.read_message()? else {
                return Ok(None);
            };
            if let Some(notification) = parse_notification(&message) {
                return Ok(Some(notification));
            }
        }
    }

    pub(super) fn set_notification_polling(&self, enabled: bool) -> Result<()> {
        self.stream
            .set_notification_polling(enabled)
            .context("configure debug notification polling")
    }

    fn handshake(&mut self, cookie: &str) -> Result<()> {
        let response = self.request_with_id(
            Value::String("hello".into()),
            "bridge.hello",
            json!({ "cookie": cookie }),
        )?;
        let echoed = response
            .get("cookie")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("bridge handshake did not echo cookie"))?;
        if !constant_time_equal(echoed.as_bytes(), cookie.as_bytes()) {
            bail!("bridge handshake cookie mismatch");
        }
        Ok(())
    }

    fn request_with_id(&mut self, id: Value, method: &str, params: Value) -> Result<Value> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let body = serde_json::to_vec(&request).context("encode debug request")?;
        write_frame(&mut self.stream, &body).context("write debug request")?;

        loop {
            let Some(message) = self.read_message()? else {
                bail!("debug bridge closed while waiting for {method}");
            };
            if let Some(notification) = parse_notification(&message) {
                self.notifications.push_back(notification);
                continue;
            }
            if message.get("id") != Some(&id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32_603);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .map_or_else(|| "unknown debug bridge error".into(), sanitize_text);
                bail!("bridge error {code}: {message}");
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn read_message(&mut self) -> Result<Option<Value>> {
        let Some(body) = self
            .reader
            .read(self.stream.as_mut())
            .context("read debug frame")?
        else {
            return Ok(None);
        };
        let message = serde_json::from_slice(&body).context("decode debug JSON-RPC frame")?;
        Ok(Some(message))
    }
}

fn parse_notification(message: &Value) -> Option<HostNotification> {
    if message.get("id").is_some() {
        return None;
    }
    let method = message.get("method")?.as_str()?.to_owned();
    Some(HostNotification {
        method,
        params: message.get("params").cloned().unwrap_or(Value::Null),
    })
}

fn connect_tcp(spec: &str, timeout: Duration) -> Result<Box<dyn ReadWrite>> {
    let spec = spec.strip_prefix("tcp://").unwrap_or(spec);
    let addresses = spec
        .to_socket_addrs()
        .with_context(|| format!("resolve TCP debug endpoint {spec}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        bail!("TCP debug endpoint resolved to no addresses: {spec}");
    }
    let mut last_error = None;
    for address in addresses {
        match connect_tcp_address(address, timeout) {
            Ok(stream) => return Ok(Box::new(stream)),
            Err(error) => last_error = Some(error),
        }
    }
    match last_error {
        Some(error) => Err(error),
        None => Err(anyhow!("connect TCP debug endpoint {spec}")),
    }
}

fn connect_tcp_address(address: SocketAddr, timeout: Duration) -> Result<TcpStream> {
    let stream = if timeout.is_zero() {
        TcpStream::connect(address)
    } else {
        TcpStream::connect_timeout(&address, timeout)
    }
    .with_context(|| format!("connect TCP debug endpoint {address}"))?;
    stream
        .set_read_timeout(Some(DEFAULT_RPC_TIMEOUT))
        .context("set debug read timeout")?;
    stream
        .set_write_timeout(Some(DEFAULT_RPC_TIMEOUT))
        .context("set debug write timeout")?;
    stream.set_nodelay(true).context("set TCP_NODELAY")?;
    Ok(stream)
}

#[cfg(all(feature = "local-socket", unix))]
fn connect_local(path: &str) -> Result<Box<dyn ReadWrite>> {
    use std::path::Path;

    use interprocess::local_socket::{prelude::*, GenericFilePath, Stream};

    let name = Path::new(path)
        .as_os_str()
        .to_fs_name::<GenericFilePath>()
        .map_err(|error| anyhow!("invalid local debug socket name: {error}"))?;
    let stream = Stream::connect(name)
        .with_context(|| format!("connect local debug socket {}", Path::new(path).display()))?;
    stream
        .set_recv_timeout(Some(DEFAULT_RPC_TIMEOUT))
        .context("set local debug read timeout")?;
    stream
        .set_send_timeout(Some(DEFAULT_RPC_TIMEOUT))
        .context("set local debug write timeout")?;
    Ok(Box::new(stream))
}

#[cfg(all(feature = "local-socket", windows))]
fn connect_local(path: &str) -> Result<Box<dyn ReadWrite>> {
    use interprocess::local_socket::{prelude::*, GenericNamespaced, Stream};

    let pipe = path
        .trim_start_matches(r"\\.\pipe\")
        .trim_start_matches(r"\\?\pipe\");
    let name = pipe
        .to_ns_name::<GenericNamespaced>()
        .map_err(|error| anyhow!("invalid named-pipe debug socket: {error}"))?;
    let stream =
        Stream::connect(name).with_context(|| format!("connect named-pipe debug socket {path}"))?;
    Ok(Box::new(stream))
}

#[cfg(not(feature = "local-socket"))]
fn connect_local(_path: &str) -> Result<Box<dyn ReadWrite>> {
    bail!("local debug sockets require the local-socket feature")
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let difference = left
        .iter()
        .zip(right)
        .fold(0_u8, |accumulator, (left, right)| {
            accumulator | (left ^ right)
        });
    difference == 0
}

pub(super) fn sanitize_text(value: &str) -> String {
    sanitize_payload_text(value).chars().take(4096).collect()
}

pub(super) fn sanitize_payload_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            let code = u32::from(*character);
            *character == '\t'
                || *character == '\n'
                || *character == '\r'
                || (code >= 0x20 && code != 0x7f && !(0x80..=0x9f).contains(&code))
        })
        .collect()
}

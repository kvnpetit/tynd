use std::io::{self, Cursor};

use serde_json::{json, Value};

use crate::{
    ipc_tap::IpcFrame,
    protocol::{encode, read_frame, write_frame, Request, Response, RpcError, MAX_FRAME_BYTES},
};

#[test]
fn frame_round_trip() {
    let mut buf = Vec::new();
    write_frame(&mut buf, b"hello").unwrap();
    write_frame(&mut buf, b"").unwrap();
    write_frame(&mut buf, &[0u8, 255, 7]).unwrap();
    assert_eq!(&buf[..4], &5u32.to_le_bytes());

    let mut cursor = Cursor::new(buf);
    assert_eq!(read_frame(&mut cursor).unwrap(), Some(b"hello".to_vec()));
    assert_eq!(read_frame(&mut cursor).unwrap(), Some(Vec::new()));
    assert_eq!(read_frame(&mut cursor).unwrap(), Some(vec![0, 255, 7]));
    assert_eq!(read_frame(&mut cursor).unwrap(), None);
}

#[test]
fn read_frame_rejects_oversized_length() {
    let mut data = (MAX_FRAME_BYTES + 1).to_le_bytes().to_vec();
    data.extend_from_slice(b"ignored");
    let err = read_frame(&mut Cursor::new(data)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn read_frame_accepts_max_declared_length_only_with_body() {
    // A frame at the cap with no body must fail with EOF, not allocate-and-succeed.
    let data = MAX_FRAME_BYTES.to_le_bytes().to_vec();
    let err = read_frame(&mut Cursor::new(data)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn read_frame_rejects_truncated_input() {
    // Partial length prefix.
    let err = read_frame(&mut Cursor::new(vec![1u8, 0])).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    // Body shorter than declared.
    let mut data = 10u32.to_le_bytes().to_vec();
    data.extend_from_slice(b"abc");
    let err = read_frame(&mut Cursor::new(data)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn read_frame_empty_input_is_clean_eof() {
    assert_eq!(read_frame(&mut Cursor::new(Vec::new())).unwrap(), None);
}

#[test]
fn request_defaults() {
    let req: Request = serde_json::from_str(r#"{"jsonrpc":"2.0","method":"ping"}"#).unwrap();
    assert_eq!(req.jsonrpc, "2.0");
    assert_eq!(req.method, "ping");
    assert_eq!(req.params, Value::Null);
    assert!(req.id.is_none());

    let req: Request =
        serde_json::from_str(r#"{"jsonrpc":"2.0","method":"m","params":[1],"id":7}"#).unwrap();
    assert_eq!(req.params, json!([1]));
    assert_eq!(req.id, Some(json!(7)));

    assert!(serde_json::from_str::<Request>(r#"{"jsonrpc":"2.0"}"#).is_err());
}

#[test]
fn response_ok_omits_error() {
    let value = serde_json::to_value(Response::ok(Some(json!(1)), json!({ "a": 1 }))).unwrap();
    assert_eq!(
        value,
        json!({ "jsonrpc": "2.0", "id": 1, "result": { "a": 1 } })
    );
}

#[test]
fn response_err_omits_result_and_empty_data() {
    let value = serde_json::to_value(Response::err(None, RpcError::method_not_found("x"))).unwrap();
    assert_eq!(
        value,
        json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": -32601, "message": "method not found: x" }
        })
    );
}

#[test]
fn rpc_error_codes() {
    assert_eq!(RpcError::parse_error().code, -32700);
    assert_eq!(RpcError::invalid_request().code, -32600);
    assert_eq!(RpcError::method_not_found("m").code, -32601);
    assert_eq!(RpcError::invalid_params("p").code, -32602);
    assert_eq!(RpcError::internal_error("i").code, -32603);
    assert_eq!(RpcError::already_attached().code, -32099);
    assert_eq!(RpcError::already_attached().message, "already_attached");
}

#[test]
fn encoded_response_survives_framing() {
    let body = encode(&Response::ok(Some(json!("abc")), json!(true))).unwrap();
    let mut buf = Vec::new();
    write_frame(&mut buf, &body).unwrap();
    let frame = read_frame(&mut Cursor::new(buf)).unwrap().unwrap();
    let parsed: Value = serde_json::from_slice(&frame).unwrap();
    assert_eq!(parsed["result"], json!(true));
    assert_eq!(parsed["id"], json!("abc"));
}

#[test]
fn ipc_frame_skips_empty_fields() {
    let frame = IpcFrame {
        ts: 5,
        direction: "in",
        call_id: None,
        method: None,
        args: Value::Null,
        result: Value::Null,
        duration_ms: None,
    };
    assert_eq!(
        serde_json::to_value(&frame).unwrap(),
        json!({ "ts": 5, "direction": "in" })
    );

    let full = IpcFrame {
        ts: 6,
        direction: "out",
        call_id: Some("c1".into()),
        method: Some("greet".into()),
        args: json!([1]),
        result: json!("ok"),
        duration_ms: Some(3),
    };
    let value = serde_json::to_value(&full).unwrap();
    assert_eq!(value["call_id"], json!("c1"));
    assert_eq!(value["duration_ms"], json!(3));
    assert_eq!(value["args"], json!([1]));
}

#[test]
fn ipc_tap_caps_oversized_payloads() {
    use crate::ipc_tap::{cap_value, MAX_TAP_VALUE_BYTES};

    let small = json!({ "a": [1, 2, 3] });
    assert_eq!(cap_value(small.clone()), small);
    assert_eq!(cap_value(Value::Null), Value::Null);

    let big = Value::String("x".repeat(MAX_TAP_VALUE_BYTES + 1));
    let capped = cap_value(big);
    assert_eq!(capped["truncated"], json!(true));
    assert!(serde_json::to_vec(&capped).unwrap().len() < 100);
}

#[test]
fn debug_server_refuses_to_listen_without_a_token() {
    use crate::server::{spawn_accept_loop, CallOutcome, Endpoint};

    let router: crate::MethodRouter = std::sync::Arc::new(|_, _| CallOutcome::Result(Value::Null));
    for token in [None, Some(String::new())] {
        let error = spawn_accept_loop(Endpoint::Tcp(0), token, router.clone()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}

#[test]
fn unauthenticated_peer_is_dropped_after_the_handshake_timeout() {
    use std::{
        io::Read,
        net::TcpStream,
        time::{Duration, Instant},
    };

    use crate::server::{spawn_accept_loop, CallOutcome, Endpoint};

    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let router: crate::MethodRouter = std::sync::Arc::new(|_, _| CallOutcome::Result(Value::Null));
    spawn_accept_loop(Endpoint::Tcp(port), Some("secret".into()), router).unwrap();

    let mut stream = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let started = Instant::now();
    let mut buf = [0u8; 8];
    // The server closes the idle connection: EOF (or a reset), not our own timeout.
    let outcome = stream.read(&mut buf);
    assert!(
        matches!(outcome, Ok(0) | Err(_)) && started.elapsed() < Duration::from_secs(15),
        "idle peer was not dropped: {outcome:?}"
    );
}

#[test]
fn trickling_unauthenticated_peer_is_dropped_by_the_absolute_deadline() {
    use std::{
        io::{Read, Write},
        net::TcpStream,
        time::{Duration, Instant},
    };

    use crate::server::{spawn_accept_loop, CallOutcome, Endpoint};

    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let router: crate::MethodRouter = std::sync::Arc::new(|_, _| CallOutcome::Result(Value::Null));
    spawn_accept_loop(Endpoint::Tcp(port), Some("secret".into()), router).unwrap();

    let mut stream = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let started = Instant::now();
    // Announce a long frame and feed it one byte per second: every byte would
    // reset a per-read timeout, but the absolute deadline still closes the peer.
    let _ = stream.write_all(&1_000u32.to_le_bytes());
    let mut dropped = false;
    while started.elapsed() < Duration::from_secs(20) {
        if stream.write_all(b"x").is_err() {
            dropped = true;
            break;
        }
        let mut buf = [0u8; 8];
        match stream.read(&mut buf) {
            Ok(0) => {
                dropped = true;
                break;
            },
            Err(error)
                if !matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                dropped = true;
                break;
            },
            _ => {},
        }
    }
    assert!(dropped, "trickling peer kept the slot past the deadline");
}

/// Scripted transport with no read timeouts (like a Windows named pipe):
/// `peek_available` replays `peeks`, then reports an empty-but-connected pipe
/// forever; `read` hands out `data` once, then reports EOF.
struct ScriptedNoTimeouts {
    peeks: std::collections::VecDeque<io::Result<usize>>,
    data: &'static [u8],
}

impl io::Read for ScriptedNoTimeouts {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.data.len().min(buf.len());
        buf[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

impl crate::server::TimedRead for ScriptedNoTimeouts {
    fn set_read_timeout(&mut self, _timeout: Option<std::time::Duration>) -> bool {
        false
    }

    fn peek_available(&mut self) -> Option<io::Result<usize>> {
        Some(self.peeks.pop_front().unwrap_or(Ok(0)))
    }
}

#[test]
fn deadline_reader_polls_an_empty_transport_until_the_deadline() {
    use std::{
        io::Read,
        time::{Duration, Instant},
    };

    use crate::server::DeadlineReader;

    let started = Instant::now();
    let mut reader = DeadlineReader::new(
        Box::new(ScriptedNoTimeouts {
            peeks: std::collections::VecDeque::new(),
            data: b"",
        }),
        Some(started + Duration::from_millis(150)),
    );
    let error = reader
        .read(&mut [0u8; 4])
        .expect_err("idle peer must expire");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(started.elapsed() >= Duration::from_millis(140));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn deadline_reader_waits_through_empty_peeks_then_reads_data() {
    use std::{
        io::Read,
        time::{Duration, Instant},
    };

    use crate::server::DeadlineReader;

    let mut reader = DeadlineReader::new(
        Box::new(ScriptedNoTimeouts {
            peeks: [Ok(0), Ok(0), Ok(0), Ok(3)].into(),
            data: b"abc",
        }),
        Some(Instant::now() + Duration::from_secs(5)),
    );
    let mut buf = [0u8; 8];
    assert_eq!(reader.read(&mut buf).unwrap(), 3);
    assert_eq!(&buf[..3], b"abc");
}

#[test]
fn deadline_reader_treats_a_closed_peer_as_eof() {
    use std::{
        io::Read,
        time::{Duration, Instant},
    };

    use crate::server::DeadlineReader;

    for error in [
        io::Error::from(io::ErrorKind::BrokenPipe),
        io::Error::from_raw_os_error(109),
        io::Error::from_raw_os_error(233),
    ] {
        let mut reader = DeadlineReader::new(
            Box::new(ScriptedNoTimeouts {
                peeks: [Ok(0), Err(error)].into(),
                data: b"",
            }),
            Some(Instant::now() + Duration::from_secs(5)),
        );
        assert_eq!(reader.read(&mut [0u8; 4]).unwrap(), 0);
    }
}

/// Starts a debug server on a fresh named pipe (token `secret`) and connects.
#[cfg(all(windows, feature = "local-socket"))]
fn connect_named_pipe(tag: &str) -> interprocess::local_socket::Stream {
    use interprocess::local_socket::{prelude::*, GenericNamespaced, Stream};

    use crate::server::{spawn_accept_loop, CallOutcome, Endpoint};

    let pipe = format!("tynd-debug-{tag}-test-{}", std::process::id());
    let router: crate::MethodRouter = std::sync::Arc::new(|_, _| CallOutcome::Result(Value::Null));
    spawn_accept_loop(
        Endpoint::Unix(pipe.clone().into()),
        Some("secret".into()),
        router,
    )
    .unwrap();
    let name = pipe.as_str().to_ns_name::<GenericNamespaced>().unwrap();
    Stream::connect(name).unwrap()
}

#[cfg(all(windows, feature = "local-socket"))]
#[test]
fn idle_named_pipe_peer_is_dropped_at_the_pre_auth_deadline() {
    use std::{
        io::Read,
        time::{Duration, Instant},
    };

    let mut stream = connect_named_pipe("idle");
    let started = Instant::now();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8];
        let _ = tx.send(stream.read(&mut buf).map_err(|e| e.kind()));
    });
    let outcome = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("idle pipe peer was never dropped");
    assert!(matches!(outcome, Ok(0) | Err(_)), "{outcome:?}");
    // Dropped at the 5 s deadline, not immediately: an empty pipe is not EOF.
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(4_500) && elapsed < Duration::from_secs(15),
        "dropped after {elapsed:?}"
    );
}

#[cfg(all(windows, feature = "local-socket"))]
#[test]
fn named_pipe_hello_sent_in_two_chunks_authenticates() {
    use std::{io::Write, time::Duration};

    let mut stream = connect_named_pipe("chunked");
    let hello = encode(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "hello",
        "params": { "cookie": "secret" },
    }))
    .unwrap();
    let mut wire = Vec::new();
    write_frame(&mut wire, &hello).unwrap();
    let (first, second) = wire.split_at(3);

    stream.write_all(first).unwrap();
    // Well under the 5 s deadline, but long enough for the server to see an
    // empty pipe several times mid-frame.
    std::thread::sleep(Duration::from_millis(700));
    stream.write_all(second).unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let frame = read_frame(&mut stream);
        let _ = tx.send(frame.map_err(|e| e.to_string()));
    });
    let body = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("no hello response")
        .expect("hello read failed")
        .expect("pipe closed during the handshake");
    let response: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(response["result"]["cookie"], "secret", "{response}");
}

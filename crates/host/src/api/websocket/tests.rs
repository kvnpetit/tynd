use std::net::TcpListener;

use super::*;

fn session_owned_by(owner: Option<&str>) -> (u64, Receiver<Cmd>, Arc<AtomicBool>) {
    let (tx, rx) = crossbeam_channel::bounded::<Cmd>(4);
    let stopping = Arc::new(AtomicBool::new(false));
    let id = SESSIONS.insert(Session {
        tx,
        stopping: Arc::clone(&stopping),
        join: Arc::new(Mutex::new(None)),
        owner: owner
            .and_then(|label| crate::ipc::owner::with_caller(label, crate::ipc::owner::current)),
    });
    (id, rx, stopping)
}

#[test]
fn release_webview_closes_only_the_sockets_that_webview_opened() {
    let (mine, mine_rx, mine_stop) = session_owned_by(Some("ws-test-a"));
    let (other, other_rx, other_stop) = session_owned_by(Some("ws-test-b"));
    let (backend, backend_rx, backend_stop) = session_owned_by(None);

    release_webview("ws-test-a");

    assert!(mine_stop.load(Ordering::Acquire));
    assert!(matches!(mine_rx.try_recv(), Ok(Cmd::Close(None))));
    assert!(!other_stop.load(Ordering::Acquire));
    assert!(other_rx.try_recv().is_err());
    assert!(!backend_stop.load(Ordering::Acquire));
    assert!(backend_rx.try_recv().is_err());

    for id in [mine, other, backend] {
        SESSIONS.remove(id);
    }
}

#[test]
fn connect_after_the_document_was_replaced_is_refused_without_a_session() {
    let label = "ws-late";
    let error = crate::ipc::owner::with_caller(label, || {
        // The commit lands while the connect call is still running.
        crate::ipc::owner::bump_generation(label);
        WebsocketApi::connect("ws://127.0.0.1:9/".to_owned())
    })
    .unwrap_err();
    // Refused before any session thread or registry entry is created.
    assert!(error.contains("is gone"), "{error}");
}

#[test]
fn remote_iframe_cannot_use_the_local_pages_socket() {
    use crate::ipc::owner::{with_caller, with_caller_trust};

    let (id, rx, stopping) = session_owned_by(Some("ws-frame"));
    with_caller_trust("ws-frame", false, || {
        assert!(!WebsocketApi::list().contains(&Value::from(id)));
        assert!(send_impl(id, "text", Some("x")).is_err());
        assert!(close_impl(id, None, None).is_err());
    });
    assert!(rx.try_recv().is_err());
    assert!(!stopping.load(Ordering::Acquire));
    with_caller("ws-frame", || send_impl(id, "text", Some("x"))).expect("page send");
    SESSIONS.remove(id);
}

#[test]
fn send_close_and_list_by_another_webview_answer_like_an_unknown_id() {
    use crate::ipc::owner::with_caller;

    let masked = |error: String, id: u64| error.replace(&id.to_string(), "<id>");
    let (id, rx, stopping) = session_owned_by(Some("ws-own-a"));
    let unknown = u64::MAX;

    with_caller("ws-own-b", || {
        assert_eq!(
            masked(send_impl(id, "text", Some("x")).unwrap_err(), id),
            masked(send_impl(unknown, "text", Some("x")).unwrap_err(), unknown)
        );
        assert_eq!(
            masked(close_impl(id, None, None).unwrap_err(), id),
            masked(close_impl(unknown, None, None).unwrap_err(), unknown)
        );
        assert!(!WebsocketApi::list().contains(&Value::from(id)));
    });
    assert!(rx.try_recv().is_err());
    assert!(!stopping.load(Ordering::Acquire));

    with_caller("ws-own-a", || {
        send_impl(id, "text", Some("x")).expect("owner send");
        assert!(WebsocketApi::list().contains(&Value::from(id)));
        close_impl(id, None, None).expect("owner close");
    });
    assert!(matches!(rx.try_recv(), Ok(Cmd::SendText(_))));
    assert!(matches!(rx.try_recv(), Ok(Cmd::Close(None))));
    // The backend (no label) may operate on any socket.
    send_impl(id, "text", Some("y")).expect("backend send");
    SESSIONS.remove(id);
}

#[test]
fn remote_webview_cannot_list_or_use_a_backend_socket() {
    use crate::ipc::owner::with_caller_trust;

    let masked = |error: String, id: u64| error.replace(&id.to_string(), "<id>");
    let (id, rx, stopping) = session_owned_by(None);
    let unknown = u64::MAX;

    with_caller_trust("ws-remote", false, || {
        assert!(!WebsocketApi::list().contains(&Value::from(id)));
        assert_eq!(
            masked(send_impl(id, "text", Some("x")).unwrap_err(), id),
            masked(send_impl(unknown, "text", Some("x")).unwrap_err(), unknown)
        );
        assert_eq!(
            masked(close_impl(id, None, None).unwrap_err(), id),
            masked(close_impl(unknown, None, None).unwrap_err(), unknown)
        );
    });
    assert!(rx.try_recv().is_err());
    assert!(!stopping.load(Ordering::Acquire));

    with_caller_trust("ws-local", true, || {
        assert!(WebsocketApi::list().contains(&Value::from(id)));
        send_impl(id, "text", Some("x")).expect("local send");
    });
    assert!(matches!(rx.try_recv(), Ok(Cmd::SendText(_))));
    SESSIONS.remove(id);
}

/// Connect a client socket to a loopback server that runs `server` on the
/// accepted (handshaken) connection.
fn client_with_server<S>(
    server: S,
) -> (
    tungstenite::WebSocket<MaybeTlsStream<TcpStream>>,
    JoinHandle<()>,
)
where
    S: FnOnce(tungstenite::WebSocket<TcpStream>) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        server(tungstenite::accept(stream).unwrap());
    });
    let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let (ws, _resp) = tungstenite::client::client(
        format!("ws://127.0.0.1:{port}/"),
        MaybeTlsStream::Plain(tcp),
    )
    .unwrap();
    (ws, handle)
}

type Events = Vec<(&'static str, Value)>;

fn collect(events: &mut Events) -> impl FnMut(&'static str, Value) + '_ {
    move |name, payload| events.push((name, payload))
}

#[test]
fn unanswered_close_ends_the_session_at_the_deadline() {
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let (ws, server) = client_with_server(move |_ws| {
        // Peer that never answers the Close frame.
        let _ = release_rx.recv();
    });
    let (tx, rx) = crossbeam_channel::bounded::<Cmd>(4);
    tx.send(Cmd::Close(None)).unwrap();
    let mut events = Vec::new();
    let started = Instant::now();
    drive_session(
        ws,
        &rx,
        None,
        &mut collect(&mut events),
        Duration::from_millis(300),
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    let names: Vec<_> = events.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        names,
        ["websocket:open", "websocket:error", "websocket:close"]
    );
    assert_eq!(events[2].1["code"], json!(1006));
    release_tx.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn peer_close_ends_the_session_with_its_code() {
    let (ws, server) = client_with_server(|mut ws| {
        let _ = ws.close(Some(CloseFrame {
            code: tungstenite::protocol::frame::coding::CloseCode::Away,
            reason: "bye".into(),
        }));
        // Drain until the handshake completes.
        while ws.read().is_ok() {}
    });
    let (_tx, rx) = crossbeam_channel::bounded::<Cmd>(4);
    let mut events = Vec::new();
    drive_session(
        ws,
        &rx,
        None,
        &mut collect(&mut events),
        Duration::from_secs(5),
    );
    let last = events.last().unwrap();
    assert_eq!(last.0, "websocket:close");
    assert_eq!(last.1["code"], json!(1001));
    server.join().unwrap();
}

#[test]
fn commands_after_close_do_not_end_the_session_with_an_error() {
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let (ws, server) = client_with_server(move |_ws| {
        let _ = release_rx.recv();
    });
    let (tx, rx) = crossbeam_channel::bounded::<Cmd>(4);
    tx.send(Cmd::Close(None)).unwrap();
    tx.send(Cmd::SendText("late".into())).unwrap();
    let mut events = Vec::new();
    drive_session(
        ws,
        &rx,
        None,
        &mut collect(&mut events),
        Duration::from_millis(200),
    );
    // The only error is the handshake timeout, not a failed late send.
    let errors: Vec<_> = events
        .iter()
        .filter(|(name, _)| *name == "websocket:error")
        .collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].1["message"], json!("close handshake timed out"));
    release_tx.send(()).unwrap();
    server.join().unwrap();
}

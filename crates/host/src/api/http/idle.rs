//! Idle (no bytes received) timeout for `http.download`.
//!
//! ureq 3 only offers phase deadlines, so a body of any size either gets a
//! whole-call deadline or none at all, and a half-open connection then blocks
//! the read for ever. `timeout_per_read` caps every transport read (response
//! headers, body and the TLS handshake) at `idle` and restarts for each read:
//! a long download that keeps receiving data never trips it, while a stalled
//! one fails.

use std::time::Duration;

use ureq::{config::ConfigBuilder, typestate::AgentScope};

/// Cap each transport read of the agent at `idle`.
pub(super) fn apply(
    builder: ConfigBuilder<AgentScope>,
    idle: Duration,
) -> ConfigBuilder<AgentScope> {
    builder.timeout_per_read(Some(idle))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read as _, Write as _},
        net::TcpListener,
        time::Instant,
    };

    use super::*;

    fn agent(idle: Duration) -> ureq::Agent {
        let builder = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0);
        apply(builder, idle).build().new_agent()
    }

    /// Serve one response: headers + `first` bytes, then `stall` (or the rest).
    fn serve(first: &'static [u8], rest: Option<(&'static [u8], Duration)>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let total = first.len() + rest.map_or(10, |(more, _)| more.len());
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\r\n");
            let _ = stream.write_all(first);
            let _ = stream.flush();
            match rest {
                Some((more, delay)) => {
                    std::thread::sleep(delay);
                    let _ = stream.write_all(more);
                    let _ = stream.flush();
                    std::thread::sleep(Duration::from_millis(200));
                },
                None => std::thread::sleep(Duration::from_secs(10)),
            }
        });
        port
    }

    #[test]
    fn stalled_body_fails_after_idle_timeout() {
        let port = serve(b"abc", None);
        let agent = agent(Duration::from_millis(300));
        let mut response = agent
            .get(format!("http://127.0.0.1:{port}/"))
            .call()
            .unwrap();
        let started = Instant::now();
        let mut sink = Vec::new();
        let result = response.body_mut().as_reader().read_to_end(&mut sink);
        assert!(result.is_err(), "stalled body must error");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn stalled_tls_handshake_fails_after_idle_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(10));
        });
        let agent = agent(Duration::from_millis(300));
        let started = Instant::now();
        let result = agent.get(format!("https://localhost:{port}/")).call();
        assert!(result.is_err(), "stalled handshake must error");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn slow_but_progressing_body_completes() {
        let port = serve(b"abc", Some((b"def", Duration::from_millis(150))));
        let agent = agent(Duration::from_millis(1500));
        let mut response = agent
            .get(format!("http://127.0.0.1:{port}/"))
            .call()
            .unwrap();
        let mut sink = Vec::new();
        response
            .body_mut()
            .as_reader()
            .read_to_end(&mut sink)
            .unwrap();
        assert_eq!(sink, b"abcdef");
    }
}

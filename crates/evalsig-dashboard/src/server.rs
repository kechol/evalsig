//! The dashboard's HTTP server (D-019): standard library only, GET only, bound to
//! 127.0.0.1. A request is answered only when its Host header names this server and
//! its path starts with the per-launch token, which defeats DNS rebinding: a foreign
//! site's page can reach the port but cannot name it as its host or know the token.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const HEAD_LIMIT: u64 = 16 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 16;
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:";

/// A response the handler gives for a path under the token.
pub struct Page {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl Page {
    pub fn html(body: String) -> Page {
        Page {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body,
        }
    }

    pub fn not_found() -> Page {
        Page {
            status: 404,
            content_type: "text/plain; charset=utf-8",
            body: "not found\n".into(),
        }
    }
}

pub struct Server {
    listener: TcpListener,
    port: u16,
    token: String,
}

/// Bind 127.0.0.1 at `port` (0 picks a free one). `token` is the secret path prefix.
pub fn bind(port: u16, token: String) -> std::io::Result<Server> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    let port = listener.local_addr()?.port();
    Ok(Server {
        listener,
        port,
        token,
    })
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Why a request gets no page.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Method,
    Host,
    Token,
}

impl Refusal {
    fn status(&self) -> u16 {
        match self {
            Refusal::Method => 405,
            Refusal::Host => 421,
            Refusal::Token => 403,
        }
    }
}

/// The path under the token, if the request line and Host header are acceptable.
pub fn check<'a>(
    line: &'a str,
    host: Option<&str>,
    port: u16,
    token: &str,
) -> Result<&'a str, Refusal> {
    let mut parts = line.split_whitespace();
    if parts.next() != Some("GET") {
        return Err(Refusal::Method);
    }
    let host_ok = host.is_some_and(|h| {
        h.eq_ignore_ascii_case(&format!("127.0.0.1:{port}"))
            || h.eq_ignore_ascii_case(&format!("localhost:{port}"))
    });
    if !host_ok {
        return Err(Refusal::Host);
    }
    let target = parts.next().unwrap_or("/");
    let rest = target.strip_prefix('/').unwrap_or(target);
    let (given, path) = rest.split_once('/').unwrap_or((rest, ""));
    if !same(given.as_bytes(), token.as_bytes()) {
        return Err(Refusal::Token);
    }
    Ok(path)
}

struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        421 => "Misdirected Request",
        _ => "Error",
    }
}

/// The head and body in one write.
fn reply(out: &mut TcpStream, status: u16, content_type: &str, body: &str) {
    let message = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nContent-Security-Policy: {CSP}\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        reason(status),
        body.len()
    );
    let _ = out.write_all(message.as_bytes());
}

/// A stream whose every read gets only the time left before one deadline, so a client
/// trickling bytes cannot hold a connection past it.
struct Deadline {
    stream: TcpStream,
    until: Instant,
}

impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(left))?;
        self.stream.read(buf)
    }
}

fn exchange(
    mut stream: TcpStream,
    port: u16,
    token: &str,
    handler: &(dyn Fn(&str) -> Page + Sync),
) {
    let _ = stream.set_write_timeout(Some(DEADLINE));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(
        Deadline {
            stream: clone,
            until: Instant::now() + DEADLINE,
        }
        .take(HEAD_LIMIT),
    );
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut host = None;
    loop {
        let mut h = String::new();
        // A head cut off by the deadline, the size budget or bad bytes gets no page.
        match reader.read_line(&mut h) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((name, value)) = h.split_once(':')
            && name.trim().eq_ignore_ascii_case("host")
        {
            host = Some(value.trim().to_owned());
        }
    }
    match check(&line, host.as_deref(), port, token) {
        Ok(path) => {
            // The handler gets the path and its query; a fragment never reaches a server.
            let path = path.split('#').next().unwrap_or("");
            let page = handler(path);
            reply(&mut stream, page.status, page.content_type, &page.body);
        }
        Err(r) => reply(&mut stream, r.status(), "text/plain; charset=utf-8", ""),
    }
}

impl Server {
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The URL to open, token included.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/{}/", self.port, self.token)
    }

    /// Answer requests until the process ends, each on its own thread.
    pub fn serve(self, handler: impl Fn(&str) -> Page + Send + Sync + 'static) {
        let handler = Arc::new(handler);
        let token = Arc::new(self.token);
        let active = Arc::new(AtomicUsize::new(0));
        for stream in self.listener.incoming() {
            let Ok(stream) = stream else {
                // Out of descriptors and the like: wait instead of spinning.
                std::thread::sleep(Duration::from_millis(100));
                continue;
            };
            if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                active.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let slot = Slot(active.clone());
            let (handler, token, port) = (handler.clone(), token.clone(), self.port);
            // A thread that cannot start drops this connection, not the server.
            let _ = std::thread::Builder::new().spawn(move || {
                let _slot = slot;
                exchange(stream, port, &token, &*handler);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(port: u16, path: &str, host: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn only_the_token_and_this_host_get_a_page() {
        let server = bind(0, "a".repeat(64)).unwrap();
        let port = server.port();
        let url = server.url();
        std::thread::spawn(move || server.serve(|p| Page::html(format!("<p>{p}</p>"))));
        let token = "a".repeat(64);
        let ok = get(
            port,
            &format!("/{token}/runs"),
            &format!("127.0.0.1:{port}"),
        );
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        assert!(ok.contains("Content-Security-Policy: default-src 'none'"));
        assert!(ok.ends_with("<p>runs</p>"));
        assert!(
            get(port, &format!("/{token}/"), &format!("localhost:{port}"))
                .starts_with("HTTP/1.1 200")
        );
        // A rebinding attack names its own host.
        let foreign = get(
            port,
            &format!("/{token}/runs"),
            &format!("evil.example.com:{port}"),
        );
        assert!(foreign.starts_with("HTTP/1.1 421"), "{foreign}");
        assert!(!foreign.contains("<p>"));
        // Without the token, or with a wrong one, nothing is served.
        for path in ["/runs", "/", &format!("/{}/runs", "b".repeat(64))] {
            let r = get(port, path, &format!("127.0.0.1:{port}"));
            assert!(r.starts_with("HTTP/1.1 403"), "{path}: {r}");
        }
        assert!(url.ends_with(&format!("/{token}/")));
    }

    #[test]
    fn only_get_is_answered() {
        assert_eq!(
            check("POST /t/ HTTP/1.1", Some("127.0.0.1:1"), 1, "t"),
            Err(Refusal::Method)
        );
        assert_eq!(check("GET /t/x HTTP/1.1", None, 1, "t"), Err(Refusal::Host));
        assert_eq!(
            check("GET /t/x HTTP/1.1", Some("127.0.0.1:1"), 1, "t"),
            Ok("x")
        );
    }
}

//! `gate serve` and `gate submit` (SPEC §8.5, D-014). The referee answers one
//! submission at a time on a Unix socket in its home (mode 0600) and, when asked, on
//! token HTTP bound to 127.0.0.1. A submission is one JSON object; the answer is the
//! same envelope `gate` prints, with its exit code. The full result stays in the
//! referee's `results/` and ledger.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::gate::{Request, Submission, judge_request};
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::Output;
use crate::project::io_error;
use crate::referee::Referee;

#[derive(Debug, Clone, clap::Args)]
pub struct ServeArgs {
    /// The referee's home (default: EVALSIG_REFEREE).
    #[arg(long, value_name = "DIR")]
    pub referee: Option<PathBuf>,
    /// Also answer token HTTP on 127.0.0.1 at this port (0 picks a free one).
    #[arg(long, value_name = "PORT")]
    pub http: Option<u16>,
}

#[derive(Debug, Clone, clap::Args)]
pub struct SubmitArgs {
    #[command(flatten)]
    pub submission: Submission,
    /// The referee's Unix socket (`gate.sock` in its home).
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with = "url",
        required_unless_present = "url"
    )]
    pub socket: Option<PathBuf>,
    /// The referee's HTTP address, `http://127.0.0.1:<port>`.
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,
    /// A file holding the bearer token for --url (default: EVALSIG_GATE_TOKEN).
    #[arg(long, value_name = "FILE", requires = "url")]
    pub token_file: Option<PathBuf>,
}

/// A submission on the wire.
#[derive(Debug, Serialize, Deserialize)]
struct Wire {
    approved: String,
    candidate: String,
    records: String,
    mapping_hash: Option<String>,
    agent_ledger_head: Option<String>,
}

/// An answer on the wire: the exit code and the envelope or error object.
#[derive(Debug, Serialize, Deserialize)]
struct Answer {
    exit: u8,
    body: Value,
}

#[cfg(unix)]
const SOCKET: &str = "gate.sock";
const HEAD_LIMIT: u64 = 16 * 1024;
/// How long a connection may take, in all, to send its submission.
const SEND_DEADLINE: Duration = Duration::from_secs(60);
/// Connections read at once; more are closed on arrival.
const MAX_CONNECTIONS: usize = 32;
/// The largest submission read. Each of MAX_CONNECTIONS may buffer this much before
/// the judge lock is taken, so a referee holds at most 2 GiB of submissions at once.
const SUBMISSION_BYTES: u64 = 64 << 20;
/// How long `gate submit` waits for the referee's answer, judging included.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(600);

/// A stream that fails once its deadline passes, however the bytes trickle in.
struct Deadline<S> {
    stream: S,
    until: Instant,
}

trait Timed {
    fn set_timeout(&self, d: Duration) -> std::io::Result<()>;
}

impl Timed for TcpStream {
    fn set_timeout(&self, d: Duration) -> std::io::Result<()> {
        self.set_read_timeout(Some(d))
    }
}

#[cfg(unix)]
impl Timed for std::os::unix::net::UnixStream {
    fn set_timeout(&self, d: Duration) -> std::io::Result<()> {
        self.set_read_timeout(Some(d))
    }
}

impl<S: Read + Timed> Read for Deadline<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.stream.set_timeout(left)?;
        self.stream.read(buf)
    }
}

/// A connection slot, given back when the connection ends.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(active: &Arc<AtomicUsize>) -> Option<Slot> {
        (active.fetch_add(1, Ordering::SeqCst) < MAX_CONNECTIONS)
            .then(|| Slot(active.clone()))
            .or_else(|| {
                active.fetch_sub(1, Ordering::SeqCst);
                None
            })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
const TOKEN: &str = "token";

fn transport(message: impl Into<String>) -> CliError {
    CliError::new("gate_transport", message, Exit::Rejected)
}

/// Answer one submission, serialized with every other by `lock`.
fn answer(root: &Path, referee: &Referee, lock: &Mutex<()>, bytes: &[u8]) -> Answer {
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    let result = serde_json::from_slice::<Wire>(bytes)
        .map_err(|e| transport(format!("not a submission: {e}")))
        .and_then(|w| {
            judge_request(
                root,
                referee,
                &Request {
                    approved: w.approved,
                    candidate: w.candidate,
                    records: w.records,
                    mapping_hash: w.mapping_hash,
                    agent_ledger_head: w.agent_ledger_head,
                },
            )
        });
    match result {
        Ok(out) => Answer {
            exit: out.exit.code(),
            body: serde_json::to_value(&out.envelope).expect("envelope serializes"),
        },
        Err(e) => failed(&e),
    }
}

fn failed(e: &CliError) -> Answer {
    Answer {
        exit: e.exit.code(),
        body: json!({ "error": e }),
    }
}

/// Bytes compared in time that does not depend on where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// One HTTP exchange: `POST /submit` with a bearer token and a JSON body.
fn http_exchange(
    stream: TcpStream,
    token: &str,
    handle: impl FnOnce(&[u8]) -> Answer,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(Deadline {
        stream: stream.try_clone()?,
        until: Instant::now() + SEND_DEADLINE,
    });
    // The request line and headers share one small budget, so a client cannot make the
    // referee buffer much before its token is checked.
    let mut budget = HEAD_LIMIT;
    let mut next_line = |reader: &mut BufReader<Deadline<TcpStream>>| -> std::io::Result<String> {
        let mut l = String::new();
        let n = reader.by_ref().take(budget).read_line(&mut l)?;
        budget -= n as u64;
        if budget == 0 {
            return Err(std::io::Error::other("request head too large"));
        }
        Ok(l)
    };
    let line = next_line(&mut reader)?;
    let route_ok = line.starts_with("POST /submit ");
    let (mut length, mut authorized) = (None::<u64>, false);
    loop {
        let h = next_line(&mut reader)?;
        if h.is_empty() || h == "\r\n" || h == "\n" {
            break;
        }
        let (name, value) = h.split_once(':').unwrap_or((&h, ""));
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().ok(),
            "authorization" => {
                authorized = value
                    .strip_prefix("Bearer ")
                    .is_some_and(|t| same(t.as_bytes(), token.as_bytes()))
            }
            _ => {}
        }
    }
    let mut out = stream;
    let reply = |out: &mut TcpStream, status: &str, body: &str| {
        write!(
            out,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    if !route_ok {
        return reply(&mut out, "404 Not Found", "{}");
    }
    if !authorized {
        return reply(&mut out, "401 Unauthorized", "{}");
    }
    let Some(n) = length.filter(|n| *n <= SUBMISSION_BYTES) else {
        return reply(&mut out, "413 Content Too Large", "{}");
    };
    let mut body = Vec::new();
    reader.take(n).read_to_end(&mut body)?;
    let a = handle(&body);
    reply(
        &mut out,
        "200 OK",
        &serde_json::to_string(&a).expect("answer serializes"),
    )
}

#[cfg(unix)]
fn listen_socket(path: &Path) -> Result<std::os::unix::net::UnixListener, CliError> {
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(transport(format!(
                "a gate serve already listens on {}",
                path.display()
            )));
        }
        std::fs::remove_file(path).map_err(|e| io_error(path, &e))?;
    }
    let l = std::os::unix::net::UnixListener::bind(path).map_err(|e| io_error(path, &e))?;
    crate::referee::private(path)?;
    Ok(l)
}

/// The bearer token in the referee home, created (mode 0600) on first use.
fn token(referee: &Referee) -> Result<String, CliError> {
    let p = referee.path(TOKEN);
    if let Ok(t) = std::fs::read_to_string(&p) {
        return Ok(t.trim().to_owned());
    }
    let t = evalsig_store::seal::random_token();
    crate::referee::write_private(&p, format!("{t}\n").as_bytes())?;
    Ok(t)
}

/// Serve until killed. The first line on stdout names where it listens.
pub fn serve(root: &Path, args: &ServeArgs) -> Result<Output, CliError> {
    // Without Unix sockets the only listener is token HTTP (SPEC §14).
    #[cfg(not(unix))]
    if args.http.is_none() {
        return Err(CliError::usage(
            "gate serve needs --http <port> where Unix sockets are not available",
        ));
    }
    let referee = Arc::new(Referee::open(args.referee.as_deref())?);
    let root = Arc::new(root.to_path_buf());
    let lock = Arc::new(Mutex::new(()));
    let mut listening = json!({});
    let http = match args.http {
        Some(port) => {
            let t = token(&referee)?;
            let l = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
                .map_err(|e| transport(format!("cannot listen on 127.0.0.1:{port}: {e}")))?;
            let addr = l.local_addr().map_err(|e| transport(e.to_string()))?;
            listening["http"] = json!(format!("http://{addr}"));
            listening["token_file"] = json!(referee.path(TOKEN).display().to_string());
            Some((l, t))
        }
        None => None,
    };
    #[cfg(unix)]
    let socket = {
        let p = referee.path(SOCKET);
        let l = listen_socket(&p)?;
        listening["socket"] = json!(p.display().to_string());
        l
    };
    println!("{listening}");
    let _ = std::io::stdout().flush();

    // Each connection gets its own thread, a slot and a deadline, so slow clients
    // cannot hold a listener; `lock` and the home's lock still judge one at a time.
    let active = Arc::new(AtomicUsize::new(0));
    let mut threads = Vec::new();
    if let Some((l, t)) = http {
        let (referee, root, lock, active) =
            (referee.clone(), root.clone(), lock.clone(), active.clone());
        let t = Arc::new(t);
        threads.push(std::thread::spawn(move || {
            for stream in l.incoming().flatten() {
                let Some(slot) = Slot::take(&active) else {
                    continue;
                };
                let (referee, root, lock, t) =
                    (referee.clone(), root.clone(), lock.clone(), t.clone());
                std::thread::spawn(move || {
                    let _slot = slot;
                    let _ = stream.set_write_timeout(Some(SEND_DEADLINE));
                    let _ = http_exchange(stream, &t, |b| answer(&root, &referee, &lock, b));
                });
            }
        }));
    }
    #[cfg(unix)]
    threads.push(std::thread::spawn(move || {
        for stream in socket.incoming().flatten() {
            let Some(slot) = Slot::take(&active) else {
                continue;
            };
            let (referee, root, lock) = (referee.clone(), root.clone(), lock.clone());
            std::thread::spawn(move || {
                let _slot = slot;
                let _ = stream.set_write_timeout(Some(SEND_DEADLINE));
                let Ok(mut writer) = stream.try_clone() else {
                    return;
                };
                let mut body = Vec::new();
                let mut reader = Deadline {
                    stream,
                    until: Instant::now() + SEND_DEADLINE,
                };
                if (&mut reader)
                    .take(SUBMISSION_BYTES + 1)
                    .read_to_end(&mut body)
                    .is_ok()
                {
                    let a = if body.len() as u64 > SUBMISSION_BYTES {
                        failed(&transport(format!(
                            "the submission is larger than {} MiB",
                            SUBMISSION_BYTES >> 20
                        )))
                    } else {
                        answer(&root, &referee, &lock, &body)
                    };
                    let _ = writer.write_all(&serde_json::to_vec(&a).expect("answer serializes"));
                }
            });
        }
    }));
    for t in threads {
        let _ = t.join();
    }
    Err(transport("the listeners stopped"))
}

/// Send one submission and return the referee's answer as this process's output.
pub fn submit(args: &SubmitArgs) -> Result<Output, CliError> {
    let s = &args.submission;
    let (approved, candidate, records) = s.parts();
    let wire = Wire {
        approved: approved.to_owned(),
        candidate: candidate.to_owned(),
        records: std::fs::read_to_string(records).map_err(|e| io_error(records, &e))?,
        mapping_hash: s.mapping_hash.clone(),
        agent_ledger_head: s.agent_ledger_head()?,
    };
    let body = serde_json::to_vec(&wire).expect("submission serializes");
    // The referee stops reading past the cap, which the client would see as a reset.
    if body.len() as u64 > SUBMISSION_BYTES {
        return Err(CliError::usage(format!(
            "the submission is larger than {} MiB, the most the referee reads",
            SUBMISSION_BYTES >> 20
        )));
    }
    let reply = match (&args.socket, &args.url) {
        (Some(p), _) => send_socket(p, &body)?,
        (None, Some(url)) => send_http(url, &token_for(args)?, &body)?,
        (None, None) => unreachable!("clap requires --socket or --url"),
    };
    let a: Answer = serde_json::from_slice(&reply)
        .map_err(|e| transport(format!("the referee's answer is not readable: {e}")))?;
    let exit = Exit::ALL
        .into_iter()
        .find(|x| x.code() == a.exit)
        .unwrap_or(Exit::Rejected);
    if let Some(e) = a.body.get("error") {
        let text = |k: &str| e[k].as_str().unwrap_or_default().to_owned();
        let mut err = CliError::new(text("code"), text("message"), exit);
        err.hint = e["hint"].as_str().map(str::to_owned);
        return Err(err);
    }
    let envelope = serde_json::from_value(a.body)
        .map_err(|e| transport(format!("the referee's answer is not an envelope: {e}")))?;
    Ok(Output { envelope, exit })
}

fn token_for(args: &SubmitArgs) -> Result<String, CliError> {
    match &args.token_file {
        Some(p) => Ok(std::fs::read_to_string(p)
            .map_err(|e| io_error(p, &e))?
            .trim()
            .to_owned()),
        None => std::env::var("EVALSIG_GATE_TOKEN")
            .map_err(|_| CliError::usage("--url needs --token-file or EVALSIG_GATE_TOKEN")),
    }
}

#[cfg(unix)]
fn send_socket(path: &Path, body: &[u8]) -> Result<Vec<u8>, CliError> {
    let mut s = std::os::unix::net::UnixStream::connect(path)
        .map_err(|e| transport(format!("cannot reach {}: {e}", path.display())))?;
    s.set_read_timeout(Some(ANSWER_TIMEOUT))
        .map_err(|e| transport(e.to_string()))?;
    s.write_all(body).map_err(|e| transport(e.to_string()))?;
    s.shutdown(std::net::Shutdown::Write)
        .map_err(|e| transport(e.to_string()))?;
    let mut out = Vec::new();
    s.read_to_end(&mut out)
        .map_err(|e| transport(e.to_string()))?;
    Ok(out)
}

#[cfg(not(unix))]
fn send_socket(_: &Path, _: &[u8]) -> Result<Vec<u8>, CliError> {
    Err(CliError::usage(
        "Unix sockets are not available here; use --url",
    ))
}

fn send_http(url: &str, token: &str, body: &[u8]) -> Result<Vec<u8>, CliError> {
    let not_loopback = |why: &str| {
        CliError::usage(format!(
            "{url}: {why}; gate submit speaks plain HTTP to a loopback address"
        ))
        .with_hint("reach a remote referee through a tunnel to its 127.0.0.1 port")
    };
    let addr = url
        .strip_prefix("http://")
        .map(|a| a.trim_end_matches('/'))
        .ok_or_else(|| not_loopback("not an http:// URL"))?;
    let target = std::net::ToSocketAddrs::to_socket_addrs(addr)
        .ok()
        .and_then(|mut a| a.next())
        .ok_or_else(|| CliError::usage(format!("{url}: not a host and port")))?;
    // The token travels in clear text, so it never leaves the machine.
    if !target.ip().is_loopback() {
        return Err(not_loopback(&format!(
            "{} is not a loopback address",
            target.ip()
        )));
    }
    let mut s = TcpStream::connect_timeout(&target, Duration::from_secs(10))
        .map_err(|e| transport(format!("cannot reach {url}: {e}")))?;
    s.set_read_timeout(Some(ANSWER_TIMEOUT))
        .map_err(|e| transport(e.to_string()))?;
    write!(
        s,
        "POST /submit HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .and_then(|()| s.write_all(body))
    .map_err(|e| transport(e.to_string()))?;
    let mut reply = Vec::new();
    s.read_to_end(&mut reply)
        .map_err(|e| transport(e.to_string()))?;
    let text = String::from_utf8_lossy(&reply);
    let (head, rest) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| transport("the referee's answer is not HTTP"))?;
    let status = head.split_whitespace().nth(1).unwrap_or_default();
    if status != "200" {
        return Err(transport(format!("the referee answered HTTP {status}")));
    }
    Ok(rest.as_bytes().to_vec())
}

//! Primitives the two loopback servers share (D-019, D-014): a constant-time token
//! comparison, reads bounded by one deadline, and a cap on connections read at once.

use std::io::Read;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The longest one read waits before the deadline is looked at again.
const READ_SLICE: Duration = Duration::from_secs(1);

/// Bytes compared in time that does not depend on where they differ.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A stream whose read timeout can be set.
pub trait Timed {
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

/// A stream that fails once its deadline passes, however the bytes trickle in.
pub struct Deadline<S> {
    stream: S,
    until: Instant,
}

impl<S: Read + Timed> Deadline<S> {
    /// Reads time out in short slices set once: setting the timeout again after the
    /// peer has shut down its side makes macOS answer EAGAIN where EOF is due.
    pub fn new(stream: S, deadline: Duration) -> std::io::Result<Self> {
        stream.set_timeout(READ_SLICE.min(deadline))?;
        Ok(Deadline {
            stream,
            until: Instant::now() + deadline,
        })
    }
}

impl<S: Read + Timed> Read for Deadline<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if Instant::now() >= self.until {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            match self.stream.read(buf) {
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                r => return r,
            }
        }
    }
}

/// A connection slot, given back when the connection ends.
pub struct Slot(Arc<AtomicUsize>);

impl Slot {
    /// A slot when fewer than `max` connections hold one.
    pub fn take(active: &Arc<AtomicUsize>, max: usize) -> Option<Slot> {
        (active.fetch_add(1, Ordering::SeqCst) < max)
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

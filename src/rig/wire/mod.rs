//! The transport: one named pipe, joined by name by every shell.
//!
//! Every pipe is held open at both ends by its owner. The `up` pipe is the
//! operator's, so the operator holds it `O_RDWR`: the open never blocks, a
//! shell exiting never looks like end-of-stream, and the reader can simply
//! block in `read` instead of polling. A shell's reply pipe is that shell's,
//! and it holds it `O_RDWR` for the same reasons in mirror — which is why the
//! operator's write never blocks and never sees `ENXIO`.

pub mod control;
pub mod frame;
pub mod record;

pub use control::{Ask, Reply, ASK_TAG};
pub use frame::{Frame, Kind};
pub use record::{field, FromRecord, Line, Micros, Pid, Record, Stamp, Stamped, WireError};

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use crate::bash::rig::capture::Capture;

/// Below `PIPE_BUF` (4096) with room for the frame header, so every frame is
/// one atomic write and concurrent shells cannot interleave.
pub const FRAME_LIMIT: usize = 3900;

const PIPE_CAPACITY: libc::c_int = 1 << 20;
const READ_CHUNK: usize = 64 * 1024;

/// A message that could not be decoded, or one whose continuation never
/// arrived. Carried alongside the capture rather than dropped.
#[derive(Clone, Debug)]
pub struct Damage {
    pub reason: String,
    pub text: String,
}

pub struct Wire {
    dir: PathBuf,
    up_path: PathBuf,
    reader: File,
    replies: HashMap<Pid, File>,
    pending: String,
    partial: HashMap<(Pid, u32), String>,
    capture: Capture,
}

impl Wire {
    pub fn create(dir: &Path) -> std::io::Result<Self> {
        let up_path = dir.join("up");
        nix::unistd::mkfifo(&up_path, nix::sys::stat::Mode::S_IRWXU)?;

        // Held read-write, so the open never blocks and no shell exiting
        // ever looks like end-of-stream; non-blocking, because the caller
        // decides when to wait, with `poll`.
        let reader = OpenOptions::new().read(true).write(true).open(&up_path)?;
        unsafe {
            let raw = reader.as_raw_fd();
            libc::fcntl(raw, libc::F_SETPIPE_SZ, PIPE_CAPACITY);
            let flags = libc::fcntl(raw, libc::F_GETFL);
            libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        Ok(Self {
            dir: dir.to_path_buf(),
            up_path,
            reader,
            replies: HashMap::new(),
            pending: String::new(),
            partial: HashMap::new(),
            capture: Capture::default(),
        })
    }

    pub fn up_path(&self) -> &Path {
        &self.up_path
    }

    /// Everything recorded so far. What an answer sees when it is asked.
    pub fn seen(&self) -> &Capture {
        &self.capture
    }

    /// The descriptor to wait on. Readable exactly when the subject has
    /// said something.
    pub fn reader(&self) -> std::os::fd::RawFd {
        self.reader.as_raw_fd()
    }

    /// Takes everything the pipe currently holds, and hands back whoever is
    /// now blocked waiting for an answer.
    pub fn drain(&mut self) -> std::io::Result<Vec<Ask>> {
        let mut buffer = [0u8; READ_CHUNK];
        let mut asks = Vec::new();

        loop {
            match self.reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    self.pending.push_str(&String::from_utf8_lossy(&buffer[..count]));
                    while let Some(end) = self.pending.find('\n') {
                        let raw: String = self.pending.drain(..=end).collect();
                        self.accept(raw.trim_end_matches('\n'), &mut asks);
                    }
                }
                Err(cause) => match cause.kind() {
                    std::io::ErrorKind::WouldBlock => break,
                    std::io::ErrorKind::Interrupted => continue,
                    _ => return Err(cause),
                },
            }
        }
        Ok(asks)
    }

    /// Answers one shell. Its reply pipe already has a reader — the shell
    /// holds both ends — so this neither blocks nor fails to find one, and
    /// the descriptor is opened once however many times that shell asks.
    pub fn answer(&mut self, pid: Pid, reply: Reply) -> std::io::Result<()> {
        let line = format!("{}\n", Record::new(reply.words().to_vec()).to_message());
        let dir = &self.dir;

        let pipe = match self.replies.entry(pid) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(OpenOptions::new().write(true).open(dir.join(format!("rep.{pid}")))?)
            }
        };
        pipe.write_all(line.as_bytes())
    }

    pub fn finish(mut self) -> Capture {
        if !self.pending.is_empty() {
            let text = std::mem::take(&mut self.pending);
            self.capture.damage.push(Damage { reason: "truncated frame".into(), text });
        }
        for ((pid, seq), text) in std::mem::take(&mut self.partial) {
            self.capture
                .damage
                .push(Damage { reason: format!("message {pid}.{seq} never completed"), text });
        }
        self.capture
    }

    fn accept(&mut self, raw: &str, asks: &mut Vec<Ask>) {
        let frame = match Frame::parse(raw) {
            Ok(frame) => frame,
            Err(cause) => return self.hurt(cause, raw.to_string()),
        };

        let key = (frame.stamp.pid, frame.stamp.seq);
        let message = match self.partial.remove(&key) {
            Some(head) => head + &frame.chunk,
            None => frame.chunk,
        };
        if frame.kind == Kind::Continues {
            self.partial.insert(key, message);
            return;
        }

        let record = match Record::parse_message(&message) {
            Ok(record) => record,
            Err(cause) => return self.hurt(cause, message),
        };
        if frame.kind == Kind::Ask {
            let args = record.behind(ASK_TAG).unwrap_or(&record.words).to_vec();
            asks.push(Ask { stamp: frame.stamp, args });
        }
        self.capture.lines.push(Stamped { stamp: frame.stamp, value: record });
    }

    fn hurt(&mut self, cause: WireError, text: String) {
        self.capture.damage.push(Damage { reason: cause.to_string(), text });
    }
}

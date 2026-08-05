//! The transport: one named pipe, joined by name by every shell.
//!
//! Nothing is inherited. Each shell opens the pipe itself from a path baked
//! into the prelude, so no descriptor has to survive a fork and there is no
//! bash-version surface at all. The reader holds the pipe `O_RDWR`, which is
//! what keeps a writer from ever blocking on open and the reader from seeing
//! a spurious end-of-file between shells.
//!
//! A shell that asks a question gets its own reply pipe, created by that
//! shell the first time it asks — a shell that only emits never pays for one.

pub mod control;
pub mod frame;
pub mod record;

pub use control::{Ask, Reply};
pub use frame::{Frame, Kind};
pub use record::{FromRecord, Line, Micros, Pid, Record, Stamp, Stamped, WireError};

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use super::capture::Capture;

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
    reader: std::fs::File,
    pending: String,
    partial: HashMap<(Pid, u32), String>,
    capture: Capture,
    asks: Vec<Ask>,
    replies: Vec<(Pid, String)>,
    steps: usize,
}

impl Wire {
    pub fn create(dir: &Path) -> std::io::Result<Self> {
        let up_path = dir.join("up");
        nix::unistd::mkfifo(&up_path, nix::sys::stat::Mode::S_IRWXU)?;

        let reader = std::fs::OpenOptions::new().read(true).write(true).open(&up_path)?;
        let raw = reader.as_raw_fd();
        unsafe {
            libc::fcntl(raw, libc::F_SETPIPE_SZ, PIPE_CAPACITY);
            let flags = libc::fcntl(raw, libc::F_GETFL);
            libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        Ok(Self {
            dir: dir.to_path_buf(),
            up_path,
            reader,
            pending: String::new(),
            partial: HashMap::new(),
            capture: Capture::default(),
            asks: Vec::new(),
            replies: Vec::new(),
            steps: 0,
        })
    }

    pub fn up_path(&self) -> &Path {
        &self.up_path
    }

    /// Reads everything currently buffered, turning complete messages into
    /// lines and questions into pending asks.
    pub fn drain(&mut self) -> std::io::Result<()> {
        let mut buffer = [0u8; READ_CHUNK];
        loop {
            match self.reader.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    self.pending.push_str(&String::from_utf8_lossy(&buffer[..count]));
                    while let Some(end) = self.pending.find('\n') {
                        let raw: String = self.pending.drain(..=end).collect();
                        self.accept(raw.trim_end_matches('\n'));
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn accept(&mut self, raw: &str) {
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
            self.asks.push(Ask { stamp: frame.stamp, record: record.clone() });
        }
        self.capture.lines.push(Stamped { stamp: frame.stamp, value: record });
    }

    fn hurt(&mut self, cause: WireError, text: String) {
        self.capture.damage.push(Damage { reason: cause.to_string(), text });
    }

    /// Everything recorded so far. What a verb sees when it is asked.
    pub fn seen(&self) -> &Capture {
        &self.capture
    }

    pub fn take_asks(&mut self) -> Vec<Ask> {
        std::mem::take(&mut self.asks)
    }

    /// Queues an answer. A step body is written out here so the asking shell
    /// only ever has to source a path.
    pub fn answer(&mut self, pid: Pid, reply: Reply) -> std::io::Result<()> {
        let words = match reply {
            Reply::Source { body } => {
                self.steps += 1;
                let path = self.dir.join(format!("step.{pid}.{}.bash", self.steps));
                std::fs::write(&path, body.as_str())?;
                vec!["source".to_string(), path.to_string_lossy().into_owned()]
            }
            Reply::Continue { status } => vec!["continue".to_string(), status.to_string()],
            Reply::Fail { message, status } => {
                vec!["fail".to_string(), message, status.to_string()]
            }
        };
        let mut words = words.into_iter();
        let reply = Record::new(words.next().expect("a reply always names its form"), words);
        self.replies.push((pid, format!("{}\n", reply.to_message())));
        Ok(())
    }

    /// Writes what it can. A reply pipe with no reader yet yields `ENXIO`;
    /// that answer simply stays queued for the next turn.
    pub fn flush(&mut self) -> std::io::Result<()> {
        let queued = std::mem::take(&mut self.replies);
        for (pid, line) in queued {
            let path = self.dir.join(format!("rep.{pid}"));
            match std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&path)
            {
                Ok(mut pipe) => pipe.write_all(line.as_bytes())?,
                Err(error)
                    if matches!(error.raw_os_error(), Some(libc::ENXIO))
                        || error.kind() == ErrorKind::NotFound =>
                {
                    self.replies.push((pid, line));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
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
}

use std::os::unix::fs::OpenOptionsExt;

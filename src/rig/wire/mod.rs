//! The transport: one named pipe, joined by name by every shell.
//!
//! Every pipe is held open at both ends by its owner, so an open never blocks,
//! a shell exiting never looks like end-of-stream, and a write never sees
//! `ENXIO`. [`Wire::drain`] hands back what it read and remembers nothing.

pub mod frame;
pub mod record;
pub mod reply;

pub use frame::Frame;
pub use record::{field, FromRecord, Line, Micros, Pid, Record, Stamp, Stamped, ASK_TAG};
pub use reply::Reply;

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::bash::rig::error::{Doing, RigError};

/// Below `PIPE_BUF` (4096) with room for the frame header, so every frame is
/// one atomic write and concurrent shells cannot interleave.
pub const FRAME_LIMIT: usize = 3900;

const READ_CHUNK: usize = 64 * 1024;

pub struct Wire {
    dir: PathBuf,
    up_path: PathBuf,
    reader: File,
    replies: HashMap<Pid, File>,

    /// Bytes read but not yet terminated by a newline.
    pending: String,

    /// Messages whose frames are still arriving, keyed by `(pid, seq)`.
    partial: HashMap<(Pid, u32), String>,
}

impl Wire {
    pub fn create(dir: &Path) -> Result<Self, RigError> {
        let up_path = dir.join("up");
        let named = || format!("creating the instrumentation pipe {}", up_path.display());

        nix::unistd::mkfifo(&up_path, nix::sys::stat::Mode::S_IRWXU).doing(named)?;

        // Read-write: the open never blocks and no shell exiting looks like
        // end-of-stream. Non-blocking: the caller waits with `poll`.
        let reader = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&up_path)
            .doing(named)?;

        Ok(Self {
            dir: dir.to_path_buf(),
            up_path,
            reader,
            replies: HashMap::new(),
            pending: String::new(),
            partial: HashMap::new(),
        })
    }

    pub fn up_path(&self) -> &Path {
        &self.up_path
    }

    /// The descriptor to wait on. Readable exactly when the subject has
    /// said something.
    pub fn reader(&self) -> RawFd {
        self.reader.as_raw_fd()
    }

    /// Every message the pipe currently holds. A shell blocked on an answer
    /// is one whose record [`asked`](Record::asked).
    pub fn drain(&mut self) -> Result<Vec<Line>, RigError> {
        let mut buffer = [0u8; READ_CHUNK];
        let mut heard = Vec::new();

        loop {
            match self.reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    self.pending.push_str(&String::from_utf8_lossy(&buffer[..count]));
                    while let Some(end) = self.pending.find('\n') {
                        let raw: String = self.pending.drain(..=end).collect();
                        self.accept(raw.trim_end_matches('\n'), &mut heard)?;
                    }
                }
                Err(cause) => match cause.kind() {
                    io::ErrorKind::WouldBlock => break,
                    io::ErrorKind::Interrupted => continue,
                    _ => return Err(cause).doing(|| "reading the instrumentation pipe".into()),
                },
            }
        }
        Ok(heard)
    }

    /// The shell holds both ends of its reply pipe, so this never blocks and
    /// the descriptor is opened once however many times that shell asks.
    pub fn answer(&mut self, pid: Pid, reply: Reply) -> Result<(), RigError> {
        let Self { dir, replies, .. } = self;

        let pipe = match replies.entry(pid) {
            Entry::Occupied(seat) => seat.into_mut(),
            Entry::Vacant(seat) => {
                let path = dir.join(format!("rep.{pid}"));
                let pipe = OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .doing(|| format!("opening the reply pipe {}", path.display()))?;
                seat.insert(pipe)
            }
        };

        let line = format!("{}\n", Record::new(reply.words().to_vec()).to_message());
        pipe.write_all(line.as_bytes()).doing(|| format!("answering pid {pid}"))
    }

    /// Errors if a frame lacks its newline or a message its last chunk.
    pub fn finish(self) -> Result<(), RigError> {
        let cut = |what: String| Err(RigError::new("draining the instrumentation pipe", what));

        if !self.pending.is_empty() {
            return cut(format!("a frame was cut short: {:?}", self.pending));
        }
        match self.partial.into_iter().next() {
            Some(((pid, seq), text)) => cut(format!("message {pid}.{seq} stopped at {text:?}")),
            None => Ok(()),
        }
    }

    fn accept(&mut self, raw: &str, heard: &mut Vec<Line>) -> Result<(), RigError> {
        let frame = Frame::parse(raw)?;

        let key = (frame.stamp.pid, frame.stamp.seq);
        let message = match self.partial.remove(&key) {
            Some(head) => head + &frame.chunk,
            None => frame.chunk,
        };
        if frame.partial {
            self.partial.insert(key, message);
            return Ok(());
        }

        heard.push(Stamped { stamp: frame.stamp, value: Record::parse_message(&message)? });
        Ok(())
    }
}

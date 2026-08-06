//! One named pipe, joined by name by every shell.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use super::framing::{Reassembly, DELIMITER};
use super::{Answer, Line, Micros, Pid};
use crate::failure::{Doing, Failure};

const READ_CHUNK: usize = 64 * 1024;

pub struct Wire {
    dir: PathBuf,
    reader: File,
    replies: HashMap<Pid, File>,
    incoming: Reassembly,
}

impl Wire {
    pub fn create(dir: &Path) -> Result<Self, Failure> {
        let up = super::up(dir);
        let named = || format!("creating the instrumentation pipe {}", up.display());

        nix::unistd::mkfifo(&up, nix::sys::stat::Mode::S_IRWXU).doing(named)?;

        // Held `O_RDWR`, so the open never blocks and a shell exiting never
        // looks like end-of-stream.
        let reader = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&up)
            .doing(named)?;

        Ok(Self {
            dir: dir.to_path_buf(),
            reader,
            replies: HashMap::new(),
            incoming: Reassembly::default(),
        })
    }

    pub fn reader(&self) -> RawFd {
        self.reader.as_raw_fd()
    }

    pub fn drain(&mut self) -> Result<Vec<Line>, Failure> {
        let mut buffer = [0u8; READ_CHUNK];
        let mut heard = Vec::new();

        loop {
            match self.reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => heard.extend(self.incoming.feed(&buffer[..count], Micros::now()?)?),
                Err(cause) => match cause.kind() {
                    io::ErrorKind::WouldBlock => break,
                    io::ErrorKind::Interrupted => continue,
                    _ => return Err(cause).doing(|| "reading the instrumentation pipe".into()),
                },
            }
        }
        Ok(heard)
    }

    pub fn answer(&mut self, pid: Pid, answer: Answer) -> Result<(), Failure> {
        let Self { dir, replies, .. } = self;

        let pipe = match replies.entry(pid) {
            Entry::Occupied(seat) => seat.into_mut(),
            Entry::Vacant(seat) => seat.insert(reply_pipe(&super::reply(dir, pid))?),
        };

        let answering = || format!("answering pid {pid}");

        pipe.write_all(answer.to_message().as_bytes()).doing(answering)?;
        pipe.write_all(&[DELIMITER]).doing(answering)
    }

    /// Nothing may be left half-read.
    pub fn finish(self) -> Result<(), Failure> {
        self.incoming.finish()
    }
}

/// Opening a pipe to write blocks until someone reads it, and a shell that
/// asked and then died never will: `ENXIO` ends the run rather than hanging
/// it.
fn reply_pipe(path: &Path) -> Result<File, Failure> {
    let opening = || format!("opening the reply pipe {}", path.display());

    let pipe =
        OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(path).doing(opening)?;

    if unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, 0) } < 0 {
        return Err(io::Error::last_os_error()).doing(opening);
    }
    Ok(pipe)
}

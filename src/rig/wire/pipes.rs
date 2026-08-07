//! One named pipe, joined by name by every shell.

use std::fs::{self, File, OpenOptions};
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

        Ok(Self { dir: dir.to_path_buf(), reader, incoming: Reassembly::default() })
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

    /// Answer the shell blocked on one message, and take the pipe with it. The
    /// shell holds its own descriptor until it has read, so removing the name
    /// is what keeps a run's asks from leaving a pipe behind each.
    pub fn answer(&self, pid: Pid, seq: u32, answer: Answer) -> Result<(), Failure> {
        let path = super::reply(&self.dir, pid, seq);
        let answering = || format!("answering pid {pid}");

        let mut pipe = reply_pipe(&path)?;
        pipe.write_all(answer.to_message().as_bytes()).doing(answering)?;
        pipe.write_all(&[DELIMITER]).doing(answering)?;

        fs::remove_file(&path).doing(|| format!("removing the reply pipe {}", path.display()))
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

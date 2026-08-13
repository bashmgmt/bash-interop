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
                Ok(count) => heard.extend(self.incoming.feed(&buffer[..count], Micros::now())?),
                Err(cause) => match cause.kind() {
                    io::ErrorKind::WouldBlock => break,
                    io::ErrorKind::Interrupted => continue,
                    _ => return Err(cause).doing(|| "reading the instrumentation pipe".into()),
                },
            }
        }
        Ok(heard)
    }

    /// Answer the shell blocked on a question, and take its pipe with the
    /// answer. The shell holds its own descriptor until it has read, and it is
    /// blocked until then, so the name is free again before it asks anything
    /// else.
    pub fn answer(&self, pid: Pid, answer: Answer) -> Result<(), Failure> {
        let path = super::reply(&self.dir, pid);
        let answering = || format!("answering pid {pid}");

        let mut pipe = reply_pipe(&path)?;
        pipe.write_all(answer.to_string().as_bytes()).doing(answering)?;
        pipe.write_all(&[DELIMITER]).doing(answering)?;

        fs::remove_file(&path).doing(|| format!("removing the reply pipe {}", path.display()))
    }

    /// Nothing may be left half-read.
    pub fn finish(self) -> Result<(), Failure> {
        self.incoming.finish()
    }
}

/// Opening a pipe to write blocks until someone reads it, and a shell that
/// asked and then died never will: `O_NONBLOCK` turns that into `ENXIO`,
/// which ends the run rather than hanging it.
///
/// The flag is cleared once the open has succeeded. Writing is a different
/// question from opening: an answer past the pipe's 64 KB is handed over in
/// more than one go, and a descriptor still marked non-blocking would report
/// `EAGAIN` and lose the rest of it.
fn reply_pipe(path: &Path) -> Result<File, Failure> {
    let opening = || format!("opening the reply pipe {}", path.display());

    let pipe =
        OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(path).doing(opening)?;

    if unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, 0) } < 0 {
        return Err(io::Error::last_os_error()).doing(opening);
    }
    Ok(pipe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bash::value::parse_array;

    /// The shell's side of one question: it holds the pipe open before asking,
    /// and reads to the delimiter. Everything before that is one message.
    fn asking(path: &Path) -> std::thread::JoinHandle<String> {
        let mut reading =
            OpenOptions::new().read(true).write(true).open(path).expect("the asking shell's end");

        std::thread::spawn(move || {
            let mut got = Vec::new();
            let mut buffer = [0u8; 8 * 1024];

            while !got.contains(&DELIMITER) {
                let count = reading.read(&mut buffer).expect("reading the answer");
                got.extend_from_slice(&buffer[..count]);
            }
            got.pop();

            String::from_utf8(got).expect("the answer is text")
        })
    }

    /// A shell that asked and then died leaves a pipe nobody is reading.
    /// Opening one to write blocks until a reader arrives, and none will —
    /// `O_NONBLOCK` is what turns that into `ENXIO` instead of a run that
    /// never returns.
    ///
    /// Answering runs on a thread so a regression fails here rather than
    /// hanging the suite.
    #[test]
    fn answering_a_pipe_nobody_reads_fails_rather_than_blocking() {
        let dir = tempfile::tempdir().expect("a workspace");
        let wire = Wire::create(dir.path()).expect("the up pipe");

        let pid = Pid(4243);
        nix::unistd::mkfifo(&super::super::reply(dir.path(), pid), nix::sys::stat::Mode::S_IRWXU)
            .expect("a reply pipe with no reader");

        let (done, answered) = std::sync::mpsc::channel();
        std::thread::spawn(move || done.send(wire.answer(pid, Answer::status(0))));

        let outcome = answered
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the open blocked; a dead asker would hang the run");

        let why = outcome.expect_err("answering a pipe nobody reads cannot succeed");
        assert!(why.to_string().contains("os error 6"), "ENXIO, not something else: {why}");
    }

    /// An answer past the pipe's 64 KB cannot be handed over in one write, so
    /// the run has to block until the shell has taken the rest. It arrives
    /// whole, and the pipe goes with it.
    #[test]
    fn an_answer_larger_than_the_pipe_buffer_is_written_whole() {
        let dir = tempfile::tempdir().expect("a workspace");
        let wire = Wire::create(dir.path()).expect("the up pipe");

        let pid = Pid(4242);
        let path = super::super::reply(dir.path(), pid);
        nix::unistd::mkfifo(&path, nix::sys::stat::Mode::S_IRWXU).expect("the reply pipe");

        let shell = asking(&path);
        let payload = "x".repeat(100_000);

        wire.answer(pid, Answer::of("printf", [payload.clone()])).expect("answering");

        let got = shell.join().expect("the asking shell");
        assert_eq!(
            parse_array(&got).expect("one bash array literal"),
            ["printf".to_string(), payload],
            "byte for byte, across more writes than one"
        );
        assert!(!path.exists(), "the pipe goes with the answer");
    }
}

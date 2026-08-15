//! A fifo read end, cut at newlines. Bytes out; what a line means is the
//! reader's.
//!
//! Bytes go straight into the buffer at each read, so a `next` dropped
//! mid-await loses nothing; the only await is on readiness.

use std::collections::VecDeque;
use std::io;
use std::path::Path;

use tokio::net::unix::pipe;

use super::message::Micros;
use crate::failure::{Doing, Failure};

const READ_CHUNK: usize = 64 * 1024;

/// One line, without its newline, and the run's clock at the read that
/// completed it.
pub(crate) struct Raw {
    pub bytes: Vec<u8>,
    pub heard_at: Micros,
}

pub(crate) struct Lines {
    receiver: pipe::Receiver,
    bytes: Vec<u8>,
    ready: VecDeque<Raw>,
    what: String,
}

enum Read {
    Some,
    End,
    Nothing,
}

impl Lines {
    /// `O_RDONLY | O_NONBLOCK`: never blocks to open, quiet until a writer
    /// attaches, end of input once every writer that attached has gone.
    pub(crate) fn open(path: &Path) -> Result<Self, Failure> {
        let receiver = pipe::OpenOptions::new().open_receiver(path);

        Self::over(receiver, path)
    }

    /// `O_RDWR`: this process counts as a writer, so the fifo never reaches
    /// end of input however many writers come and go.
    pub(crate) fn open_read_write(path: &Path) -> Result<Self, Failure> {
        let receiver = pipe::OpenOptions::new().read_write(true).open_receiver(path);

        Self::over(receiver, path)
    }

    fn over(receiver: io::Result<pipe::Receiver>, path: &Path) -> Result<Self, Failure> {
        let what = path.display().to_string();
        let receiver = receiver.doing(|| format!("opening {what}"))?;

        Ok(Self { receiver, bytes: Vec::new(), ready: VecDeque::new(), what })
    }

    /// The fifo, as it names itself in a complaint.
    pub(crate) fn what(&self) -> &str {
        &self.what
    }

    /// The next whole line, or `None` at end of input.
    pub(crate) async fn next(&mut self) -> Result<Option<Raw>, Failure> {
        loop {
            if let Some(line) = self.ready.pop_front() {
                return Ok(Some(line));
            }
            match self.read()? {
                Read::Some => continue,
                Read::End => return Ok(None),
                Read::Nothing => {
                    self.receiver.readable().await.doing(|| format!("waiting on {}", self.what))?;
                }
            }
        }
    }

    /// Every whole line already there, without waiting for more.
    pub(crate) fn drain(&mut self) -> Result<Vec<Raw>, Failure> {
        while let Read::Some = self.read()? {}

        Ok(self.ready.drain(..).collect())
    }

    /// Nothing may be left half-read.
    pub(crate) fn finish(self) -> Result<(), Failure> {
        if self.bytes.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&self.bytes);

        Err(Failure::new(format!("closing {}", self.what), format!("a line was cut short: {text:?}")))
    }

    fn read(&mut self) -> Result<Read, Failure> {
        let mut buffer = [0u8; READ_CHUNK];

        match self.receiver.try_read(&mut buffer) {
            Ok(0) => Ok(Read::End),
            Ok(count) => {
                self.cut(&buffer[..count], Micros::now());
                Ok(Read::Some)
            }
            Err(cause) if cause.kind() == io::ErrorKind::WouldBlock => Ok(Read::Nothing),
            Err(cause) => Err(cause).doing(|| format!("reading {}", self.what)),
        }
    }

    /// Everything one read carried arrived at one moment.
    fn cut(&mut self, bytes: &[u8], heard_at: Micros) {
        self.bytes.extend_from_slice(bytes);

        let mut from = 0;
        while let Some(offset) = self.bytes[from..].iter().position(|byte| *byte == b'\n') {
            let end = from + offset;

            self.ready.push_back(Raw { bytes: self.bytes[from..end].to_vec(), heard_at });
            from = end + 1;
        }
        self.bytes.drain(..from);
    }
}

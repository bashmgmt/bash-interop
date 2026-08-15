//! One shell's two fifos: what it says, and what it is told.

use std::fs;
use std::path::PathBuf;

use tokio::io::AsyncWriteExt;
use tokio::net::unix::pipe;

use super::lines::{Lines, Raw};
use super::message::{Answer, Line};
use crate::failure::{Doing, Failure};

pub(crate) struct Pipe {
    lines: Lines,
    up: PathBuf,
    rep: PathBuf,
}

impl Pipe {
    /// Opening the read end is what releases the shell blocked in opening the
    /// write end.
    pub(crate) fn open(up: PathBuf, rep: PathBuf) -> Result<Self, Failure> {
        Ok(Self { lines: Lines::open(&up)?, up, rep })
    }

    /// The next line the shell wrote, or `None` once nobody can write there.
    /// One writer, so a whole line is a whole message.
    pub(crate) async fn next(&mut self) -> Result<Option<Line>, Failure> {
        self.lines.next().await?.map(|raw| self.line(raw)).transpose()
    }

    /// Every whole line already there, without waiting for more.
    pub(crate) fn drain(&mut self) -> Result<Vec<Line>, Failure> {
        self.lines.drain()?.into_iter().map(|raw| self.line(raw)).collect()
    }

    fn line(&self, raw: Raw) -> Result<Line, Failure> {
        let text = String::from_utf8(raw.bytes)
            .doing(|| format!("reading {} as text", self.lines.what()))?;

        Ok(Line { text, heard_at: raw.heard_at })
    }

    /// One line down the reply pipe. Opening it to write blocks until someone
    /// reads, and a shell that asked and then died never will: `open_sender`
    /// gives `ENXIO` instead. Writing awaits the shell taking it, so an answer
    /// past the pipe's buffer holds up nothing but this shell.
    pub(crate) async fn answer(&self, answer: Answer) -> Result<(), Failure> {
        let answering = || format!("answering on {}", self.rep.display());
        let mut sender = pipe::OpenOptions::new().open_sender(&self.rep).doing(answering)?;

        sender.write_all(format!("{answer}\n").as_bytes()).await.doing(answering)
    }

    /// Both fifos go; a line left half-read is reported.
    pub(crate) fn close(self) -> Result<(), Failure> {
        for path in [&self.up, &self.rep] {
            fs::remove_file(path).doing(|| format!("removing {}", path.display()))?;
        }

        self.lines.finish()
    }
}

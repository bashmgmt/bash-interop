//! The control fifo: every shell announces itself here, once, in frames.
//!
//! Many shells write to one fifo, and a write is atomic only up to `PIPE_BUF`
//! (4096 on Linux). So an announcement is one or more frames of at most that
//! many bytes, each `<token> + <bytes>` for one with more to come and `<token>
//! . <bytes>` for the last; the frames of one shell may interleave with
//! another's and are put back together here, per token, in bytes — a frame may
//! end inside a character. The reassembled bytes are the shell's [`Account`].

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::lines::{Lines, Raw};
use super::message::Account;
use crate::failure::{Doing, Failure};

pub(crate) struct Control {
    lines: Lines,
    dir: PathBuf,

    /// Announcements begun and not yet ended, by token.
    partial: HashMap<String, Vec<u8>>,
}

/// One shell, announced whole.
pub(crate) struct Announced {
    pub token: String,
    pub account: Account,
}

impl Control {
    pub(crate) fn open(dir: &Path) -> Result<Self, Failure> {
        let join = super::join(dir);
        super::mkfifo(&join)?;

        Ok(Self { lines: Lines::open_read_write(&join)?, dir: dir.to_path_buf(), partial: HashMap::new() })
    }

    /// The next shell announced whole. Never end of input: the fifo is held
    /// read-write. Cancellation-safe: the only await is on the fifo, and every
    /// frame already read is in `partial`.
    pub(crate) async fn next(&mut self) -> Result<Announced, Failure> {
        loop {
            let raw = self
                .lines
                .next()
                .await?
                .ok_or_else(|| Failure::new("reading the control fifo", "it reached end of input"))?;

            if let Some(announced) = self.frame(raw)? {
                return Ok(announced);
            }
        }
    }

    /// Everything announced whole and not yet opened is released — its pipe
    /// opened and closed, so the shell blocked on it goes on and takes
    /// `SIGPIPE` at its next write — an announcement left in the middle is
    /// dropped, and the fifo is unlinked, so a shell arriving later finds no
    /// session.
    pub(crate) fn close(mut self) -> Result<(), Failure> {
        for raw in self.lines.drain()? {
            if let Some(Announced { token, .. }) = self.frame(raw)? {
                let up = super::up(&self.dir, &token);

                drop(Lines::open(&up)?);
                fs::remove_file(&up).doing(|| format!("removing {}", up.display()))?;
            }
        }
        let join = super::join(&self.dir);
        fs::remove_file(&join).doing(|| format!("removing {}", join.display()))?;

        self.lines.finish()
    }

    /// One frame in; a whole announcement out when the frame was its last.
    fn frame(&mut self, raw: Raw) -> Result<Option<Announced>, Failure> {
        let frame = Frame::read(&raw.bytes)?;
        let mut bytes = self.partial.remove(frame.token).unwrap_or_default();
        bytes.extend_from_slice(frame.chunk);

        if !frame.last {
            self.partial.insert(frame.token.to_string(), bytes);
            return Ok(None);
        }
        let text = String::from_utf8(bytes)
            .doing(|| format!("reading the announcement of {} as text", frame.token))?;
        let account = Account::read(&text, raw.heard_at)?;

        Ok(Some(Announced { token: frame.token.to_string(), account }))
    }
}

/// `<token> <+|.> <chunk>`, one line on the control fifo.
struct Frame<'a> {
    token: &'a str,
    last: bool,
    chunk: &'a [u8],
}

impl<'a> Frame<'a> {
    /// A token names two files in the workspace, so it must be able to.
    fn read(bytes: &'a [u8]) -> Result<Self, Failure> {
        let refused = || {
            let shown = String::from_utf8_lossy(bytes);
            Failure::new("reading the control fifo", format!("{shown:?} is not a frame"))
        };
        let space = bytes.iter().position(|byte| *byte == b' ').ok_or_else(refused)?;
        let token = std::str::from_utf8(&bytes[..space]).map_err(|_| refused())?;
        let names_a_file =
            !token.is_empty() && !token.contains(['/', '\0']) && !token.contains(char::is_whitespace);
        if !names_a_file {
            return Err(refused());
        }

        let (last, chunk) = match &bytes[space..] {
            [b' ', b'+', b' ', chunk @ ..] => (false, chunk),
            [b' ', b'.', b' ', chunk @ ..] => (true, chunk),
            _ => return Err(refused()),
        };

        Ok(Self { token, last, chunk })
    }
}

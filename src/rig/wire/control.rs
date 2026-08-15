//! The control fifo: every shell writes its token here, once.

use std::fs;
use std::path::{Path, PathBuf};

use super::lines::Lines;
use crate::failure::{Doing, Failure};

pub(crate) struct Control {
    lines: Lines,
    dir: PathBuf,
}

impl Control {
    pub(crate) fn open(dir: &Path) -> Result<Self, Failure> {
        let join = super::join(dir);
        super::mkfifo(&join)?;

        Ok(Self { lines: Lines::open_read_write(&join)?, dir: dir.to_path_buf() })
    }

    /// The next token announced. Never end of input: the fifo is held
    /// read-write.
    pub(crate) async fn next(&mut self) -> Result<String, Failure> {
        let line = self
            .lines
            .next()
            .await?
            .ok_or_else(|| Failure::new("reading the control fifo", "it reached end of input"))?;

        token(line.text)
    }

    /// Everything announced and not yet opened is released — its pipe opened
    /// and closed, so the shell blocked on it goes on and takes `SIGPIPE` at
    /// its next write — and the fifo is unlinked, so a shell arriving later
    /// finds no session.
    pub(crate) fn close(mut self) -> Result<(), Failure> {
        for line in self.lines.drain()? {
            let up = super::up(&self.dir, &token(line.text)?);

            drop(Lines::open(&up)?);
            fs::remove_file(&up).doing(|| format!("removing {}", up.display()))?;
        }
        let join = super::join(&self.dir);
        fs::remove_file(&join).doing(|| format!("removing {}", join.display()))?;

        self.lines.finish()
    }
}

/// A token names two files in the workspace, so it must be able to.
fn token(text: String) -> Result<String, Failure> {
    let names_a_file =
        !text.is_empty() && !text.contains(['/', '\0']) && !text.contains(char::is_whitespace);

    match names_a_file {
        true => Ok(text),
        false => Err(Failure::new("reading the control fifo", format!("{text:?} is not a token"))),
    }
}

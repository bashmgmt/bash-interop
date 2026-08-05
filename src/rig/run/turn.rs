//! One question, and everything around it.

use std::path::Path;

use crate::bash::rig::error::{Doing, RigError};
use crate::bash::rig::source::BashSrc;
use crate::bash::rig::wire::{Line, Reply, Stamp};

/// The question and where to put what a reply refers to. Constructible only
/// where a shell is blocked.
pub struct Turn<'a> {
    asked: &'a Line,
    args: &'a [String],
    dir: &'a Path,
}

impl<'a> Turn<'a> {
    pub(crate) fn new(asked: &'a Line, args: &'a [String], dir: &'a Path) -> Self {
        Self { asked, args, dir }
    }

    /// The words the subject passed after `ask`, in order.
    pub fn args(&self) -> &[String] {
        self.args
    }

    /// Who asked, and when.
    pub fn stamp(&self) -> Stamp {
        self.asked.stamp
    }

    /// This run's workspace.
    pub fn dir(&self) -> &Path {
        self.dir
    }

    /// Writes `body` into the workspace and returns the command that sources
    /// it. The name is fixed by the ask, so it cannot collide.
    pub fn source(&self, body: &BashSrc) -> Result<Reply, RigError> {
        let stamp = self.stamp();
        let path = self.dir.join(format!("step.{}.{}.bash", stamp.pid, stamp.seq));

        std::fs::write(&path, body.as_str())
            .doing(|| format!("writing the step {}", path.display()))?;

        Ok(Reply::source(&path))
    }
}

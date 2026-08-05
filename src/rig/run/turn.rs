//! One question, and everything around it.

use std::path::Path;

use crate::bash::rig::capture::Capture;
use crate::bash::rig::source::BashSrc;
use crate::bash::rig::wire::{Ask, Reply, Stamp};

/// What an answer is given: the question, the history behind it, and the
/// place to put anything the reply needs to refer to.
pub struct Turn<'a> {
    ask: &'a Ask,
    seen: &'a Capture,
    dir: &'a Path,
}

impl<'a> Turn<'a> {
    pub(crate) fn new(ask: &'a Ask, seen: &'a Capture, dir: &'a Path) -> Self {
        Self { ask, seen, dir }
    }

    /// The words the subject passed after `ask`, in order.
    pub fn args(&self) -> &[String] {
        &self.ask.args
    }

    /// Who asked, and when.
    pub fn stamp(&self) -> Stamp {
        self.ask.stamp
    }

    /// Everything the run has recorded so far.
    pub fn seen(&self) -> &Capture {
        self.seen
    }

    /// This run's workspace.
    pub fn dir(&self) -> &Path {
        self.dir
    }

    /// Writes `body` into the workspace and returns the command that sources
    /// it. The name is fixed by the ask, so it cannot collide.
    ///
    /// Panics if the workspace cannot be written, which means the disk is
    /// gone. An answer that wants to survive that writes the file itself —
    /// [`dir`](Self::dir) is right here — and decides what to reply.
    pub fn source(&self, body: &BashSrc) -> Reply {
        let stamp = self.ask.stamp;
        let path = self.dir.join(format!("step.{}.{}.bash", stamp.pid, stamp.seq));

        std::fs::write(&path, body.as_str())
            .unwrap_or_else(|cause| panic!("step {}: {cause}", path.display()));

        Reply::source(&path)
    }
}

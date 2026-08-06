//! One question, and where to put what a reply refers to.

use std::path::Path;

use crate::bash::rig::error::{Doing, RigError};
use crate::bash::rig::source::BashSrc;
use crate::bash::rig::wire::{Line, Reply, Stamp};

pub struct Turn<'a> {
    asked: &'a Line,
    dir: &'a Path,
}

impl<'a> Turn<'a> {
    pub(crate) fn over(asked: &'a Line, dir: &'a Path) -> Option<Self> {
        asked.value.asked()?;
        Some(Self { asked, dir })
    }

    pub fn args(&self) -> &[String] {
        self.asked.value.asked().unwrap_or_default()
    }

    pub fn stamp(&self) -> Stamp {
        self.asked.stamp
    }

    pub fn source(&self, body: &BashSrc) -> Result<Reply, RigError> {
        let stamp = self.stamp();
        let path = self.dir.join(format!("step.{}.{}.bash", stamp.pid, stamp.seq));

        std::fs::write(&path, body.as_str())
            .doing(|| format!("writing the step {}", path.display()))?;

        Ok(Reply::source(&path))
    }
}

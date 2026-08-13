//! Where the protocol's files sit in a run's workspace.
//!
//! Every path here is derived once. The bash builds the reply path from
//! `__BC__DIR` itself, which is why the run tells it the directory rather than
//! the file.

use std::fs;
use std::path::{Path, PathBuf};

use crate::failure::{Doing, Failure};

use super::Pid;

/// The client half, shipped verbatim. It locates the workspace from its own
/// path, so there is nothing to substitute into it.
pub(super) const PRELUDE: &str = include_str!("prelude.bash");

/// The one FIFO every shell joins. `Wire` makes it and the bash names it, so
/// it is derived here and nowhere else.
pub(crate) fn up(dir: &Path) -> PathBuf {
    dir.join("up")
}

/// Where a shell blocked on an ask is listening. Made for one question and
/// removed with its answer, so a run holds no descriptor and leaves no file
/// per ask. The bash builds the same path from `__BC__DIR`, which is why the
/// run tells it the directory and not this.
pub(crate) fn reply(dir: &Path, pid: Pid) -> PathBuf {
    dir.join(format!("rep.{pid}"))
}

/// Lays the protocol's bash into `dir` with the rig's beside it, and returns
/// the file `BASH_ENV` must name. Both are written as they are: `dir` must be
/// absolute, since that path is what every shell reads its own location from.
pub fn prelude(dir: &Path, bash: &str) -> Result<PathBuf, Failure> {
    let entry = dir.join("prelude.bash");

    for (file, body) in [(&entry, PRELUDE), (&dir.join("rig.bash"), bash)] {
        fs::write(file, body).doing(|| format!("writing {}", file.display()))?;
    }

    Ok(entry)
}


//! The protocol: the bash that speaks it, the pipe it travels on, the frame
//! around a message, and the answer that goes back.

mod framing;
mod message;
mod pipes;

use std::fs;
use std::path::{Path, PathBuf};

use crate::failure::{Doing, Failure};

pub use message::{field, Answer, Kind, Line, Micros, Pid};
pub use pipes::Wire;

/// The client half, shipped verbatim. It locates the workspace from its own
/// path, so there is nothing to substitute into it.
const PRELUDE: &str = include_str!("prelude.bash");

/// The one FIFO every shell joins. `Wire` makes it and the bash names it, so
/// it is derived here and nowhere else.
fn up(dir: &Path) -> PathBuf {
    dir.join("up")
}

/// A shell's reply pipe, named after the pid that asks. The bash builds the
/// same path from `__BC__DIR`, which is why the run tells it the directory
/// and not this.
fn reply(dir: &Path, pid: Pid) -> PathBuf {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The subject's shell is left as it was found. Every one of these is a
    /// property of the file, so none of them needs bash to run.
    #[test]
    fn the_protocol_half_touches_nothing_of_the_subject_s() {
        let code: Vec<&str> =
            PRELUDE.lines().filter(|line| !line.trim_start().starts_with('#')).collect();

        for forbidden in ["eval", "trap", "export", "shopt"] {
            assert!(!code.join("\n").contains(forbidden), "the protocol contains {forbidden:?}");
        }

        // `set --` rebinds a function's positional parameters and is scoped to
        // the call; `set -e` and friends change the shell the subject runs in.
        for line in &code {
            let after_set = line.split("set -").nth(1);
            assert!(after_set.is_none_or(|rest| rest.starts_with('-')), "changes an option: {line}");
        }

        // A line that *starts* `NAME=value` and stops there sets a global.
        // `IFS= read …` is a command prefix, binding only for that command.
        for line in &code {
            let Some((name, rest)) = line.trim_start().split_once('=') else { continue };
            let global = !name.is_empty()
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                && rest.split_whitespace().count() <= 1;

            assert!(!global || name.starts_with("__BC__"), "a name outside __BC__: {line}");
        }
    }
}

//! The protocol: the bash that speaks it, the pipe it travels on, the frame
//! around a message, and the answer that goes back.

mod framing;
mod message;
mod pipes;

use std::fs;
use std::path::{Path, PathBuf};

use crate::failure::{Doing, Failure};

pub use message::{field, Answer, Kind, Line, Micros, Pid, Sent};
pub use pipes::Wire;

/// The client half, shipped verbatim. It locates the workspace from its own
/// path, so there is nothing to substitute into it.
const PRELUDE: &str = include_str!("prelude.bash");

/// The one FIFO every shell joins. `Wire` makes it and the bash names it, so
/// it is derived here and nowhere else.
fn up(dir: &Path) -> PathBuf {
    dir.join("up")
}

/// Where a shell blocked on an ask is listening. Made for one question and
/// removed with its answer, so a run holds no descriptor and leaves no file
/// per ask. The bash builds the same path from `__BC__DIR`, which is why the
/// run tells it the directory and not this.
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

    /// The namespace. Everything the protocol brings is called `__BC_…`, so
    /// nothing it does can collide with a name the subject chose.
    const OURS: &str = "__BC_";

    /// What the subject's shell keeps. Every one of these is a property of
    /// the file, so none of them needs bash to run — and every one of them is
    /// about names and options the subject owns, never about ours.
    ///
    /// `expand_aliases` is the one option the protocol turns on, and it stays
    /// on: the guards have to be aliases, because `return` must act in the
    /// frame that failed. A subject's own aliases therefore expand where they
    /// otherwise would not — the single change this makes, made once and
    /// written down rather than asserted away.
    #[test]
    fn the_protocol_half_touches_little_of_the_subject_s() {
        let code: Vec<&str> =
            PRELUDE.lines().filter(|line| !line.trim_start().starts_with('#')).collect();

        // Only what reaches past the namespace counts: a word of ours may be
        // spelled however it likes.
        let theirs: Vec<String> = code
            .iter()
            .map(|line| {
                let words = line.split_whitespace().filter(|word| !word.contains(OURS));

                words.collect::<Vec<_>>().join(" ")
            })
            .collect();

        for forbidden in ["eval", "trap", "export"] {
            let found = theirs.iter().find(|line| line.contains(forbidden));

            assert!(found.is_none(), "the protocol {forbidden}s: {found:?}");
        }

        // `set --` rebinds a function's positional parameters and is scoped to
        // the call; `set -e` and friends change the shell the subject runs in.
        for line in &theirs {
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

            assert!(!global || name.starts_with(OURS), "a name outside {OURS}: {line}");
        }
    }
}

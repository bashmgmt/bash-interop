//! The protocol: the bash that speaks it, where its files sit, the fifos it
//! travels on, and the lines on them.

mod control;
mod lines;
mod message;
mod pipe;

use std::fs;
use std::path::Path;

use bash_strings::emit_scalar;
use super::Layout;
use crate::failure::{Doing, Failure};

pub(crate) use control::{Announced, Control};
pub(crate) use message::{Account, Line};
pub(crate) use pipe::Pipe;
pub use message::{field, Answer, Message, Micros, Pid, Stamp, Verb};

/// The client half, shipped verbatim.
const PRELUDE: &str = include_str!("prelude.bash");

pub(crate) fn mkfifo(path: &Path) -> Result<(), Failure> {
    nix::unistd::mkfifo(path, nix::sys::stat::Mode::S_IRWXU)
        .doing(|| format!("making the fifo {}", path.display()))
}

/// Lays the session's bash into the workspace — the protocol's, the rig's,
/// and the generated invocation naming both, written last: once it is there,
/// the session is sourceable. The rig's text bakes the coordinate itself,
/// so the invocation passes nothing.
pub(crate) fn lay(at: &Layout, bash: &str) -> Result<(), Failure> {
    let invocation =
        format!("source {}\nsource {}\n", emit_scalar(&at.prelude()), emit_scalar(&at.rig()));

    for (file, body) in [(at.prelude(), PRELUDE), (at.rig(), bash), (at.session(), &invocation)] {
        fs::write(&file, body).doing(|| format!("writing {file}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The namespace: everything the protocol brings is called `__BC_…` or
    /// `BC_…`, so nothing it does can collide with a name the subject chose.
    const OURS: &str = "BC_";

    /// What the subject's shell keeps. Every one of these is a property of the
    /// file, so none needs bash to run.
    ///
    /// `expand_aliases` is the one option the protocol turns on, and it stays
    /// on: the guards have to be aliases, because `return` must act in the
    /// frame that failed.
    #[test]
    fn the_protocol_half_touches_little_of_the_subject_s() {
        let code: Vec<&str> =
            PRELUDE.lines().filter(|line| !line.trim_start().starts_with('#')).collect();

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

            assert!(!global || name.contains(OURS), "a name outside {OURS}: {line}");
        }
    }

    /// The invocation names the two files, quoted: a workspace path bash
    /// would split or expand still sources. The rig's text is laid as given —
    /// the coordinate is already baked into it.
    #[test]
    fn the_invocation_names_the_session_s_files() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("it's $HERE");
        std::fs::create_dir(&dir).unwrap();
        let at = Layout::new(dir.clone()).unwrap();

        lay(&at, "words\n").unwrap();

        let dir = dir.to_str().unwrap();
        assert_eq!(
            std::fs::read_to_string(at.session()).unwrap(),
            format!(
                "source '{q}/prelude.bash'\nsource '{q}/rig.bash'\n",
                q = dir.replace('\'', r"'\''"),
            ),
        );
        assert_eq!(std::fs::read_to_string(format!("{dir}/rig.bash")).unwrap(), "words\n");
    }
}

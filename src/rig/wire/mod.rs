//! The protocol: the bash that speaks it, where its files sit, the fifos it
//! travels on, and the lines on them.

mod control;
mod lines;
mod message;
mod pipe;

use std::fs;
use std::path::{Path, PathBuf};

use super::Setup;
use crate::bash::value::emit_scalar;
use crate::failure::{Doing, Failure};

pub(crate) use control::{Announced, Control};
pub(crate) use message::{Account, Line};
pub(crate) use pipe::Pipe;
pub use message::{field, Answer, Message, Micros, Pid, Stamp, Verb};

/// The client half, shipped verbatim.
const PRELUDE: &str = include_str!("prelude.bash");

/// The generated invocation, and the address: what a shell sources to join.
const SESSION: &str = "session.bash";

/// The control fifo, made and held by the run; the bash names it the same way.
pub(crate) fn join(dir: &Path) -> PathBuf {
    dir.join("join")
}

/// One shell's pipe, made by the shell.
pub(crate) fn up(dir: &Path, token: &str) -> PathBuf {
    dir.join(format!("up.{token}"))
}

/// One shell's reply pipe, made by the run before the shell can ask.
pub(crate) fn rep(dir: &Path, token: &str) -> PathBuf {
    dir.join(format!("rep.{token}"))
}

pub(crate) fn mkfifo(path: &Path) -> Result<(), Failure> {
    nix::unistd::mkfifo(path, nix::sys::stat::Mode::S_IRWXU)
        .doing(|| format!("making the fifo {}", path.display()))
}

/// Lays the session's bash into `dir` — the protocol's, the rig's, and the
/// generated invocation naming both — and returns the address as text: it
/// crosses into bash and onto the announce line, so it is validated whole
/// here. `dir` must be absolute: the invocation spells it into every path.
pub(crate) fn lay(dir: &Path, setup: &Setup) -> Result<String, Failure> {
    let laying = || format!("laying the session at {}", dir.display());
    let Setup { label, bash } = setup;

    // The same predicate `BC_JOIN` applies: the label names a fifo and sits
    // in a space-delimited frame token.
    if label.is_empty() || label.contains('/') || label.contains(char::is_whitespace) {
        return Err(Failure::new(laying(), format!("label {label:?} will not name a file")));
    }
    let dir = dir
        .to_str()
        .filter(|dir| !dir.contains('\n'))
        .ok_or_else(|| Failure::new(laying(), "the workspace path is not one line of text"))?;

    let invocation = format!(
        "source {}\nBC_JOIN {} {}\nsource {}\n",
        emit_scalar(&format!("{dir}/prelude.bash")),
        emit_scalar(label),
        emit_scalar(dir),
        emit_scalar(&format!("{dir}/rig.bash")),
    );

    for (name, body) in [("prelude.bash", PRELUDE), ("rig.bash", bash), (SESSION, &invocation)] {
        let file = format!("{dir}/{name}");
        fs::write(&file, body).doing(|| format!("writing {file}"))?;
    }

    Ok(format!("{dir}/{SESSION}"))
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

    /// The invocation is where the coordinate is spelled, quoted: a workspace
    /// path bash would split or expand still joins.
    #[test]
    fn the_invocation_spells_the_coordinate() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("it's $HERE");
        std::fs::create_dir(&dir).unwrap();

        let setup = Setup { label: "KEEP".into(), bash: "words\n".into() };
        let address = lay(&dir, &setup).unwrap();

        let dir = dir.to_str().unwrap();
        assert_eq!(address, format!("{dir}/session.bash"));
        assert_eq!(
            std::fs::read_to_string(&address).unwrap(),
            format!(
                "source '{q}/prelude.bash'\nBC_JOIN 'KEEP' '{q}'\nsource '{q}/rig.bash'\n",
                q = dir.replace('\'', r"'\''"),
            ),
        );
        assert_eq!(std::fs::read_to_string(format!("{dir}/rig.bash")).unwrap(), "words\n");
    }

    /// What `BC_JOIN` would refuse never reaches a shell.
    #[test]
    fn a_label_that_will_not_name_a_file_is_refused() {
        let temp = tempfile::tempdir().unwrap();

        for label in ["", "a/b", "two words"] {
            let setup = Setup { label: label.into(), bash: String::new() };
            let refused = lay(temp.path(), &setup).unwrap_err();

            assert!(refused.to_string().contains("will not name a file"), "{refused}");
        }
    }
}

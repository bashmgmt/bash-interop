//! What a blocked shell is told to do next.

use std::path::Path;

/// One command, as an arglist — the same shape a message has. Its status is
/// what `BC_INSTR ask` returns.
///
/// ```text
/// ["return", "1"]                          resume with a status
/// ["exit", "9"]                            end the shell
/// ["source", "/…/step.bash"]               run code
/// ["declare", "-g", "picked=elderberry"]   assign
/// ["eval", "picked=x; note ready"]         interim, for debugging
/// ["WITH_BASHCAP", "-BCS:probe", "deploy"] a call into the tool's own words
/// ```
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reply(Vec<String>);

impl Reply {
    pub fn of(words: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self(words.into_iter().map(Into::into).collect())
    }

    /// Return this status from `BC_INSTR ask`.
    pub fn status(code: i32) -> Self {
        Self::of(["return".to_string(), code.to_string()])
    }

    /// Source a file. See [`Turn::source`](crate::bash::rig::Turn::source),
    /// which writes one into the run's workspace first.
    pub fn source(path: &Path) -> Self {
        Self::of(["source", &path.to_string_lossy()])
    }

    /// For an interim answer not worth a file.
    pub fn eval(code: &str) -> Self {
        Self::of(["eval", code])
    }

    pub fn words(&self) -> &[String] {
        &self.0
    }
}

//! The control protocol: what a blocked shell asked, and what it is told.

use std::path::Path;

/// How `BC_INSTR` labels a question on the wire, so that an ask is
/// recognisable in the capture. Stripped before the ask reaches an answer:
/// it is the transport's word, not the subject's.
pub const ASK_TAG: &str = "__ASK__";

use super::record::Stamp;

/// A shell blocked on its reply pipe until this is answered.
///
/// `args` is exactly what the subject passed after `ask`, in order, with
/// nothing read into any position. What those words mean is the business of
/// whoever answers.
#[derive(Clone, Debug)]
pub struct Ask {
    pub stamp: Stamp,
    pub args: Vec<String>,
}

/// What the shell runs next.
///
/// One command, as an arglist — the same shape a message has — and its status
/// is what `BC_INSTR ask` returns. There is no second form and there never
/// will be: a bash command array can reach anything the shell knows, so the
/// fidelity comes from the vocabulary the prelude defined rather than from
/// variants here.
///
/// ```text
/// [":"]                                    nothing
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

    /// Carry on, changing nothing.
    pub fn nothing() -> Self {
        Self::of([":"])
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

    /// For an interim answer that is not worth a file. The one place `eval`
    /// appears anywhere in this system, and it is the operator's own words
    /// rather than anything the subject produced.
    pub fn eval(code: &str) -> Self {
        Self::of(["eval", code])
    }

    pub fn words(&self) -> &[String] {
        &self.0
    }
}

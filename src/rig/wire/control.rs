//! The control protocol: what a blocked shell asked, and what it is told.

use super::record::Stamp;
use crate::bash::rig::codegen::BashSrc;

/// How `BC_INSTR` labels a question on the wire, so that an ask is
/// recognisable in the capture. Stripped before the ask reaches an answer:
/// it is the transport's word, not the subject's.
pub const ASK_TAG: &str = "__ASK__";

/// A shell blocked on its reply pipe until this is answered.
///
/// `args` is exactly what the subject passed to `BC_INSTR`, in order, with
/// nothing read into any position. What those words mean is the business of
/// whoever answers.
#[derive(Clone, Debug)]
pub struct Ask {
    pub stamp: Stamp,
    pub args: Vec<String>,
}

/// The two ways a blocked shell can be let go: with a status, or with code to
/// run in its own scope.
///
/// There is no third variant for refusal. Anything an answer wants to say to
/// the subject it says in code the subject runs — the rig never writes to the
/// subject's own streams, and a client that wants `Result`-shaped answers
/// wraps that itself.
pub enum Reply {
    Continue { status: i32 },
    Source { body: BashSrc },
}

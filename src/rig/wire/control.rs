//! The control protocol: what a blocked shell asked, and what it is told.

use super::record::{Record, Stamp};
use super::super::codegen::BashSrc;

/// A shell blocked on its reply pipe until this is answered.
#[derive(Clone, Debug)]
pub struct Ask {
    pub stamp: Stamp,
    pub record: Record,
}

impl Ask {
    /// An ask names its verb first; the rest is that verb's arguments.
    pub fn verb(&self) -> &str {
        self.record.args.first().map_or("", String::as_str)
    }

    pub fn args(&self) -> &[String] {
        self.record.args.get(1..).unwrap_or_default()
    }
}

/// `Source` hands back code the asking shell runs in its own scope; the other
/// two only set the status `BC_INSTR` returns.
pub enum Reply {
    Continue { status: i32 },
    Source { body: BashSrc },
    Fail { message: String, status: i32 },
}

impl Reply {
    /// The answer for a verb nobody claimed. Loud, and distinguishable from
    /// anything a verb would choose.
    pub fn unknown_verb(verb: &str) -> Self {
        Self::Fail { message: format!("unknown verb {verb:?}"), status: 127 }
    }
}

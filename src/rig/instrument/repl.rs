//! A REPL, for driving a shell one call at a time.
//!
//! Nothing is invented for it: the loop asks over the ordinary gateway, the
//! next call arrives as the ordinary sourced continuation, and its outcome
//! comes back as an ordinary message. What the operator supplies is one
//! function of the history — `&Capture -> Turn` — and everything a steering
//! decision needs is already in that history, typed.

use super::{Instrument, Verb};
use super::super::capture::Capture;
use super::super::codegen::{Asset, BashSrc, Codegen};
use super::super::wire::{FromRecord, Record, Reply};

const REPL_SRC: Asset = Asset::new("rig/repl.bash");

pub const RAN_TAG: &str = "__RAN__";
const VERB: &str = "__repl";

/// What a dispatched call did. The step writes it before the next turn is
/// asked for, so a handler always sees the outcome of what it last chose.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ran {
    pub command: String,
    pub status: i32,
}

impl FromRecord for Ran {
    const TAG: &'static str = RAN_TAG;
    type Err = String;

    fn from_record(record: &Record) -> Result<Self, Self::Err> {
        let field =
            |key: &str| record.field(key).ok_or_else(|| format!("{RAN_TAG} is missing {key:?}"));
        Ok(Self {
            command: field("command")?.to_string(),
            status: field("status")?.parse().map_err(|_| "status is not a number")?,
        })
    }
}

/// What the operator wants next.
pub enum Turn {
    Run(String),
    Stop,
}

/// The step the shell sources: run the call, keep its status for the next
/// ask, and report what happened over the wire.
fn step(command: &str, codegen: &Codegen) -> BashSrc {
    let quoted = crate::bash::value::emit_scalar(command);
    BashSrc::seq([
        BashSrc::raw(command.to_string()),
        BashSrc::raw("__BC__repl_rc=$?"),
        BashSrc::raw(format!(
            "declare -a __bc_ran=({RAN_TAG} command {quoted} status \"$__BC__repl_rc\")"
        )),
        codegen.emit("__bc_ran"),
    ])
}

/// Turns one function of the history into a `BC_REPL` loop.
pub fn repl(next: impl Fn(&Capture) -> Turn + Send + Sync + 'static) -> Instrument {
    Instrument::new("repl", |_| {
        BashSrc::raw(REPL_SRC.read().unwrap_or_else(|error| panic!("{error}")))
    })
    .answering(Verb::new(VERB, move |_ask, seen| match next(seen) {
        Turn::Run(command) => Reply::Source { body: step(&command, &Codegen::new()) },
        Turn::Stop => Reply::Continue { status: 1 },
    }))
}

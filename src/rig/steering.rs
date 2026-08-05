//! A steering loop, for driving a shell one call at a time.
//!
//! The client supplies a [`Steering`] impl; [`Repl`] turns it into an
//! instrument. Nothing new is invented for it — the loop asks over the
//! ordinary gateway, the next call arrives as an ordinary sourced
//! continuation, and its outcome goes back as an ordinary message.

use std::sync::{Arc, Mutex};

use super::asset::Asset;
use super::control::{Reply, Verb};
use super::instrument::{Codegen, Instrument};
use super::src::BashSrc;
use super::wire::Ask;
use super::record::{FromRecord, Record};

const REPL_SRC: Asset = Asset::new("rig/repl.bash");

pub const RAN_TAG: &str = "__RAN__";
const VERB: &str = "__repl";

/// What a dispatched call did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ran {
    pub command: String,
    pub status: i32,
}

impl FromRecord for Ran {
    const TAG: &'static str = RAN_TAG;
    type Err = String;

    fn from_record(record: &Record) -> Result<Self, Self::Err> {
        let field = |key: &str| {
            record.field(key).ok_or_else(|| format!("{RAN_TAG} is missing {key:?}"))
        };
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

pub trait Steering: Send + Sync {
    /// `last` is `None` on the first turn, and afterwards describes the call
    /// that just finished.
    fn next(&self, last: Option<&Ran>) -> Turn;
}

impl<F> Steering for F
where
    F: Fn(Option<&Ran>) -> Turn + Send + Sync,
{
    fn next(&self, last: Option<&Ran>) -> Turn {
        self(last)
    }
}

pub struct Repl<S> {
    steering: Arc<S>,
    dispatched: Arc<Mutex<Option<String>>>,
}

impl<S: Steering + 'static> Repl<S> {
    pub fn new(steering: S) -> Self {
        Self { steering: Arc::new(steering), dispatched: Arc::new(Mutex::new(None)) }
    }
}

/// The step the shell sources: run the call, remember its status for the next
/// ask, and report what happened over the wire.
fn step_body(command: &str, codegen: &Codegen) -> String {
    let quoted = crate::bash::value::emit_scalar(command);
    BashSrc::seq([
        BashSrc::raw(command.to_string()),
        BashSrc::raw("__BC__repl_rc=$?"),
        BashSrc::raw(format!(
            "declare -a __bc_ran=({RAN_TAG} command {quoted} status \"$__BC__repl_rc\")"
        )),
        codegen.emit("__bc_ran"),
    ])
    .as_str()
    .to_string()
}

impl<S: Steering + 'static> Instrument for Repl<S> {
    fn name(&self) -> &str {
        "repl"
    }

    fn bash(&self, _codegen: &Codegen) -> BashSrc {
        BashSrc::raw(REPL_SRC.read().unwrap_or_else(|error| {
            panic!("{error}");
        }))
    }

    fn verbs(&self) -> Vec<Verb> {
        let state = self.dispatched.clone();
        let steering = self.steering.clone();

        vec![Verb::new(VERB, move |ask: &Ask| {
            let status: i32 =
                ask.record.args.get(1).and_then(|raw| raw.parse().ok()).unwrap_or(0);
            let last = state.lock().unwrap().take().map(|command| Ran { command, status });

            match steering.next(last.as_ref()) {
                Turn::Run(command) => {
                    let body = step_body(&command, &Codegen::new());
                    *state.lock().unwrap() = Some(command);
                    Reply::Source { body }
                }
                Turn::Stop => Reply::Continue { status: 1 },
            }
        })]
    }
}

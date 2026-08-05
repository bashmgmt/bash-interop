//! What an instrument may answer when a shell asks.

use super::wire::Ask;

/// The answer to a breakpoint. `Source` hands back code the asking shell
/// runs in its own scope; the others just set the status `BC_INSTR` returns.
pub enum Reply {
    Continue { status: i32 },
    Source { body: String },
    Fail { message: String, status: i32 },
}

impl Reply {
    /// The answer for a verb nobody claimed. Loud, and distinguishable from
    /// anything a verb would choose.
    pub fn unknown_verb(verb: &str) -> Self {
        Self::Fail { message: format!("unknown verb {verb:?}"), status: 127 }
    }
}

pub struct Verb {
    pub name: String,
    handle: Box<dyn Fn(&Ask) -> Reply + Send + Sync>,
}

impl Verb {
    pub fn new(
        name: impl Into<String>,
        handle: impl Fn(&Ask) -> Reply + Send + Sync + 'static,
    ) -> Self {
        Self { name: name.into(), handle: Box::new(handle) }
    }

    pub fn answer(&self, ask: &Ask) -> Reply {
        (self.handle)(ask)
    }
}

/// An ask names its verb first; the rest is that verb's arguments.
pub fn verb_of(ask: &Ask) -> &str {
    ask.record.args.first().map_or("", String::as_str)
}

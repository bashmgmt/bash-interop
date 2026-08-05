//! Composable bash instrumentation.
//!
//! A [`Rig`] is a set of instruments folded into one bash prelude that runs
//! before user code in every participating shell. There are exactly three
//! moments: setup writes the prelude, then the subject *says* something or
//! *asks* something — the two operations of `BC_INSTR`, the only name client
//! code ever calls.
//!
//! Nothing is injected behind the subject's back — no traps, no shadowed
//! builtins, no exported variables, no global shell state — and there is no
//! `eval` anywhere.
//!
//! A message in either direction is an **arglist**. The rig reads no position
//! of one and attaches no meaning to any word; a leading discriminator is a
//! convention a tool opts into with [`Record::behind`], not something the
//! transport knows about.
//!
//! The four concerns are one directory each:
//!
//! | | |
//! |---|---|
//! | [`wire`] | the pipes, the framing, and the message codec |
//! | [`source`] | the bash that gets injected |
//! | [`capture`] | reading a run back, and every view over it |
//! | [`run`] | the behaviour, the rig, and the shape a tool takes |
//!
//! Adding a tool is one [`Behaviour`] and one [`FromRecord`].

pub mod capture;
pub mod run;
pub mod source;
pub mod wire;

#[cfg(test)]
mod tests;

pub use capture::{Capture, Origin, Shell, ShellNode};
pub use run::{
    ExitStatus, Outcome, Report, Rig, Rigging, RigError, Setup, ToolError, Turn, Workspace,
};
pub use source::{Asset, AssetError, BashSrc};
pub use wire::{
    field, Ask, Damage, FromRecord, Line, Micros, Pid, Record, Reply, Stamp, Stamped, WireError,
};

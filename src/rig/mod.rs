//! Composable bash instrumentation.
//!
//! A [`Rig`] is a set of instruments folded into one bash prelude that runs
//! before user code in every participating shell. There are exactly three
//! moments: setup writes the prelude, the subject *speaks* by calling a
//! function that ships a message, and the subject *asks* by calling
//! `BC_INSTR` and blocking for a continuation.
//!
//! Nothing is injected behind the subject's back — no traps, no shadowed
//! builtins, no exported variables, no global shell state — and there is no
//! `eval` anywhere.
//!
//! The four concerns are one directory each:
//!
//! | | |
//! |---|---|
//! | [`wire`] | the pipes, the framing, and the message codec |
//! | [`codegen`] | producing the bash that gets injected |
//! | [`capture`] | reading a run back, and every view over it |
//! | [`instrument`] | a mechanism, as a value |
//! | [`run`] | driving a run, and the shape a tool takes |
//!
//! Adding a tool is one [`Instrument`] and one [`FromRecord`].

pub mod capture;
pub mod codegen;
pub mod instrument;
pub mod run;
pub mod wire;

#[cfg(test)]
mod tests;

pub use capture::{Capture, Origin, Shell, ShellNode};
pub use codegen::{Asset, AssetError, BashSrc, Codegen};
pub use instrument::{repl, Dispatch, Instrument, Ran, Turn, Verb};
pub use run::{capture_into, ExitStatus, Outcome, Report, Rig, RigError, ToolError};
pub use wire::{
    Ask, Damage, FromRecord, Line, Micros, Pid, Record, Reply, Stamp, Stamped, WireError,
};

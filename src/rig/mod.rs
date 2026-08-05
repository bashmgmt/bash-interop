//! Composable bash instrumentation. `KB/mb_resolver/bash/` documents it.
//!
//! | | |
//! |---|---|
//! | [`wire`] | the pipes, the framing, and the message codec |
//! | [`source`] | the bash that gets injected |
//! | [`capture`] | reading a run back, and every view over it |
//! | [`run`] | the rig itself, and what running one means |
//! | [`error`] | [`RigError`], which every fallible path returns |
//!
//! `BC_INSTR say` and `BC_INSTR ask` are the whole client surface. A message
//! is an arglist in both directions; only [`ASK_TAG`] and
//! [`ORIGIN_TAG`](capture::origin::ORIGIN_TAG) are reserved.

pub mod capture;
pub mod error;
pub mod run;
pub mod source;
pub mod wire;

#[cfg(test)]
mod tests;

pub use capture::{Capture, Origin, Shell, ShellNode};
pub use error::{Doing, RigError};
pub use run::{ExitStatus, Outcome, Report, Rig, Rigging, Setup, Turn, Workspace};
pub use source::{Asset, BashSrc};
pub use wire::{field, FromRecord, Line, Micros, Pid, Record, Reply, Stamp, Stamped, ASK_TAG};

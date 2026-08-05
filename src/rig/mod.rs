//! Composable bash instrumentation.
//!
//! A [`Rig`] is a tool folded into one bash prelude that runs before user code
//! in every participating shell. There are exactly three moments: setup writes
//! the prelude, then the subject *says* something or *asks* something — the two
//! operations of `BC_INSTR`, the only name client code ever calls.
//!
//! Nothing is injected behind the subject's back — no traps, no shadowed
//! builtins, no exported variables, no global shell state — and there is no
//! `eval` anywhere.
//!
//! A message in either direction is an **arglist**. The rig reads no position
//! of one and attaches no meaning to any word; a leading discriminator is a
//! convention a tool opts into with [`Record::behind`], not something the
//! transport knows about. Only [`ASK_TAG`] and
//! [`ORIGIN_TAG`](capture::origin::ORIGIN_TAG) are reserved, and both are the
//! transport describing itself in an ordinary message.
//!
//! A run yields an [`Outcome`] or a [`RigError`]. There is no third channel
//! and no partial success: the first thing that cannot be read or written
//! ends the run, and the subject is killed on the way out.
//!
//! The concerns are one directory each, over one [`error`]:
//!
//! | | |
//! |---|---|
//! | [`wire`] | the pipes, the framing, and the message codec |
//! | [`source`] | the bash that gets injected |
//! | [`capture`] | reading a run back, and every view over it |
//! | [`run`] | the rig itself, and what running one means |
//!
//! Adding a tool is one [`Rig`] and one [`FromRecord`].

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

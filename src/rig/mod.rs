//! Composable bash instrumentation. `KB/mb_resolver/bash/` documents it.
//!
//! `BC_INSTR say` and `BC_INSTR ask` are the whole client surface. A message
//! is an arglist in both directions; only [`ASK_TAG`] and [`ORIGIN_TAG`] are
//! reserved.
//!
//! | | |
//! |---|---|
//! | `error` | [`RigError`], which every fallible path returns |
//! | `source` | the bash that gets injected, and the prelude it folds into |
//! | `wire` | the pipes, the framing, the message and the reply |
//! | `capture` | what a rig kept, and every view over it |
//! | `run` | the [`Rig`] trait, the driver, the turn |
//! | `listen` | the two calls that cover a rig with nothing of its own |
//!
//! The modules are private: this list is the surface, so the transport can
//! move without moving anyone's imports.

mod capture;
mod error;
mod listen;
mod run;
mod source;
mod wire;

#[cfg(test)]
mod tests;

pub use capture::{Capture, Origin, Shell, ShellNode, ORIGIN_TAG};
pub use error::{Doing, RigError};

pub use run::{ExitStatus, Rig, Setup, Turn, Workspace};
pub use listen::{converse, listen};
pub use source::{prelude, Asset, BashSrc};
pub use wire::{field, FromRecord, Line, Micros, Pid, Record, Reply, Stamp, Stamped, ASK_TAG};

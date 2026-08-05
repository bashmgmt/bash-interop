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
//! A message is one bash array literal, so structure survives the trip in
//! both directions. Adding a tool is one [`Instrument`] and one
//! [`FromRecord`].

pub mod asset;
pub mod capture;
pub mod control;
pub mod dispatch;
pub mod frame;
pub mod instrument;
pub mod origin;
pub mod record;
pub mod rig;
pub mod src;
pub mod steering;
pub mod wire;

#[cfg(test)]
mod tests;

pub use asset::{Asset, AssetError};
pub use capture::{Capture, Shell, ShellNode};
pub use control::{Reply, Verb};
pub use dispatch::Dispatch;

pub use instrument::{Codegen, Instrument};
pub use origin::Origin;
pub use record::{FromRecord, Line, Micros, Pid, Record, Stamp, Stamped, WireError};
pub use rig::{ExitStatus, Outcome, Rig, RigError};
pub use src::BashSrc;
pub use steering::{Ran, Repl, Steering, Turn};
pub use wire::{Ask, Damage, Wire};

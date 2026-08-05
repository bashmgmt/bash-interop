//! Run bash under instrumentation, and hear what it says.

mod capture;
mod error;
mod run;
mod source;
mod wire;

pub use run::{converse, listen, ExitStatus, Rig, Setup, Turn, Workspace};

pub use capture::{Capture, Origin, Shell, ShellNode};
pub use wire::{field, FromRecord, Line, Micros, Pid, Record, Stamp, Stamped};

pub use wire::Reply;

pub use source::{prelude, BashSrc};

pub use error::{Doing, RigError};

pub use capture::ORIGIN_TAG;
pub use wire::ASK_TAG;

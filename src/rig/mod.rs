//! Run bash under instrumentation, and hear what it says.
//!
//! ```bash
//! BC_INSTR say REC compiled "$target"    # ship an arglist and carry on
//! BC_INSTR ask which target              # ship one, block, run the answer
//! ```
//!
//! ```no_run
//! use mb_resolver::bash::rig::{listen, RigError, Setup};
//!
//! let (seen, status) = listen(Setup::new(), &["build.bash"])?;
//!
//! for line in seen.chronological() {
//!     println!("pid {} said {:?}", line.stamp.pid, line.value.words);
//! }
//! # Ok::<(), RigError>(())
//! ```
//!
//! An answer is a command the shell runs, so its expressiveness is bash's:
//!
//! ```no_run
//! use mb_resolver::bash::rig::{converse, Reply, RigError, Setup};
//!
//! let (seen, status) = converse(Setup::new(), &["deploy.bash"], |seen, asked| {
//!     Ok(match asked.args().first().map(String::as_str) {
//!         Some("target") => Reply::of(["declare", "-g", "target=staging"]),
//!         _ => Reply::status(1),
//!     })
//! })?;
//! # Ok::<(), RigError>(())
//! ```
//!
//! Implement [`Rig`] when a run needs resources of its own, or an output that
//! is not a capture.

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

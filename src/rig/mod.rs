//! Run bash under instrumentation, and hear what it says.
//!
//! A script talks to the rig through one name and two operations:
//!
//! ```bash
//! BC_INSTR say REC compiled "$target"    # ship an arglist and carry on
//! BC_INSTR ask which target              # ship one, block, run the answer
//! ```
//!
//! Every shell in the process tree reaches the same pipe, including subshells,
//! command substitutions and child processes. Nothing else enters the shell:
//! no traps, no shadowed builtins, no exported variables, no `eval`.
//!
//! # Listening
//!
//! ```no_run
//! use mb_resolver::bash::rig::{listen, RigError, Setup};
//!
//! let (seen, status) = listen(Setup::new(), &["build.bash"])?;
//!
//! for line in seen.chronological() {
//!     println!("pid {} said {:?}", line.stamp.pid, line.value.words);
//! }
//! println!("{status}");
//! # Ok::<(), RigError>(())
//! ```
//!
//! # Answering
//!
//! An answer is a **command the shell runs**, so its expressiveness is bash's
//! rather than a set of variants here. It may return a status, assign, source
//! a file, exit, or call any word the injected bash defined.
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
//! The closure keeps whatever state it needs by capturing it, so most tools
//! never declare a type of their own.
//!
//! # A tool of your own
//!
//! Implement [`Rig`] when a run needs resources of its own, or an output that
//! is not a capture — a profiler streaming to a file, say. [`Rig::start`]
//! allocates the session, [`Rig::heard`] and [`Rig::answer`] fold into it, and
//! [`Rig::ended`] turns it into the run's output and releases what it held.
//!
//! # Guarantees
//!
//! A run yields its output or a [`RigError`]; there is no second channel and
//! no partial success. The subject gets its own process group, released on
//! every exit path, so nothing it started outlives the call.
//!
//! # Inside
//!
//! | | |
//! |---|---|
//! | `error` | [`RigError`], which every fallible path returns |
//! | `source` | the bash that gets injected, and the prelude it folds into |
//! | `wire` | the pipes, the framing, the message and the reply |
//! | `capture` | what a rig kept, and the process forest |
//! | `run` | the [`Rig`] trait, the driver, the turn, and [`converse`] |
//!
//! Those modules are private: the list below is the surface, so the transport
//! can change without moving anyone's imports.

mod capture;
mod error;
mod run;
mod source;
mod wire;

// running one
pub use run::{converse, listen, ExitStatus, Rig, Setup, Turn, Workspace};

// what it said
pub use capture::{Capture, Origin, Shell, ShellNode};
pub use wire::{field, FromRecord, Line, Micros, Pid, Record, Stamp, Stamped};

// what it is told
pub use wire::Reply;

// the bash a tool contributes
pub use source::{prelude, Asset, BashSrc};

// how it fails
pub use error::{Doing, RigError};

// the only two words the transport reserves, both it describing itself
pub use capture::ORIGIN_TAG;
pub use wire::ASK_TAG;

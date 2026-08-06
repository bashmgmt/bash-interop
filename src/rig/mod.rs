//! Run bash under instrumentation, and hear what it says.
//!
//! ```bash
//! BC_INSTR say REC compiled "$target"    # ship an arglist and carry on
//! BC_INSTR ask which target              # ship one, block, run the answer
//! ```
//!
//! A [`Rig`] is the functional definition of a run: the bash it needs, the
//! session it keeps, and how that session reacts. Everything else — the
//! workspace, the pipes, the prelude, the subject's process group — belongs
//! to [`run()`], which hands the session back when bash is gone.
//!
//! ```no_run
//! use mb_resolver::bash::rig::{run, Answer, Failure, Line, Rig};
//!
//! /// Keeps every message, and tells a shell that asks to use staging.
//! struct Deploying;
//!
//! impl Rig for Deploying {
//!     type Session = Vec<Line>;
//!
//!     /// A word the subject's scripts can call, in every shell.
//!     fn bash(&self) -> String {
//!         "STAGE() { BC_INSTR say STAGE \"$@\"; }".into()
//!     }
//!
//!     fn open(&self) -> Result<Vec<Line>, Failure> {
//!         Ok(Vec::new())
//!     }
//!
//!     fn hear(&self, heard: &mut Vec<Line>, said: Line) -> Result<(), Failure> {
//!         heard.push(said);
//!         Ok(())
//!     }
//!
//!     fn answer(&self, heard: &mut Vec<Line>, asked: Line) -> Result<Answer, Failure> {
//!         let target = asked.asked().unwrap_or_default().first().cloned();
//!         heard.push(asked);
//!
//!         Ok(match target.as_deref() {
//!             Some("target") => Answer::of(["declare", "-g", "target=staging"]),
//!             _ => Answer::status(1),
//!         })
//!     }
//! }
//!
//! let (heard, status) = run(&Deploying, &["deploy.bash"])?;
//!
//! for line in &heard {
//!     println!("pid {} said {:?}", line.pid, line.words);
//! }
//! # Ok::<(), Failure>(())
//! ```
//!
//! An answer is a command the shell runs, so its expressiveness is bash's.
//! Only `open` is required: `bash`, `hear`, `answer` and `end` default to no
//! bash, keeping nothing, saying the word is unknown, and doing nothing.

mod run;
mod tree;
mod wire;

use std::fmt;

pub use run::{run, run_in};
pub use tree::{forest, shells, BadField, Origin, Shell, ShellNode};
pub use wire::{field, Answer, Line, Micros, Pid};

pub use crate::failure::{Doing, Failure};

/// The functional definition of a run. A rig says what bash it needs, how its
/// session is initialised, and how that session reacts; the run does
/// everything else.
pub trait Rig {
    /// The client's state. No bounds, no lifetime: the run stores nothing of
    /// its own in it, and hands it back when the run is over.
    type Session;

    /// Bash this rig needs, injected into every shell after the protocol's
    /// own and before the subject runs.
    fn bash(&self) -> String {
        String::new()
    }

    fn open(&self) -> Result<Self::Session, Failure>;

    /// A message nobody is waiting on.
    fn hear(&self, _session: &mut Self::Session, _said: Line) -> Result<(), Failure> {
        Ok(())
    }

    /// A message a shell is blocked on; the run frames what comes back and
    /// writes it to that shell. Hearing the question and telling the shell
    /// the word is unknown, unless a rig says otherwise.
    fn answer(&self, session: &mut Self::Session, asked: Line) -> Result<Answer, Failure> {
        self.hear(session, asked)?;

        Ok(Answer::status(127))
    }

    /// The subject is gone; release what the session holds.
    fn end(&self, _session: &mut Self::Session, _status: ExitStatus) -> Result<(), Failure> {
        Ok(())
    }
}

/// How bash ended. `wait(2)` yields exactly one of these, and both fields are
/// the width the kernel gives them.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ExitStatus {
    Code(u8),
    Signal(u8),
}

impl ExitStatus {
    /// Ended cleanly, or the status saying it did not.
    pub fn ok(self) -> Result<(), Self> {
        match self {
            Self::Code(0) => Ok(()),
            ended => Err(ended),
        }
    }

    /// What a shell would report for it: `128 + n` for a signal.
    pub fn shell_code(self) -> i32 {
        match self {
            Self::Code(code) => i32::from(code),
            Self::Signal(signal) => 128 + i32::from(signal),
        }
    }
}

impl fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code(code) => write!(f, "exit {code}"),
            Self::Signal(signal) => write!(f, "killed by signal {signal}"),
        }
    }
}

impl std::error::Error for ExitStatus {}

impl From<std::process::ExitStatus> for ExitStatus {
    /// After `wait(2)` a process has either exited or been signalled, so
    /// reading the two fields out of the raw status is total: `WTERMSIG` is
    /// the low seven bits, `WEXITSTATUS` the second byte, and there is no
    /// third outcome to default to.
    fn from(status: std::process::ExitStatus) -> Self {
        use std::os::unix::process::ExitStatusExt;

        let raw = status.into_raw();
        match status.signal() {
            Some(_) => Self::Signal((raw & 0x7f) as u8),
            None => Self::Code(((raw >> 8) & 0xff) as u8),
        }
    }
}

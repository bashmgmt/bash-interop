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
//! use mb_resolver::bash::rig::{run, Answer, Failure, Line, Rig, Startup};
//!
//! /// Keeps every message, and tells a shell that asks to use staging.
//! struct Deploying;
//!
//! impl Rig for Deploying {
//!     type Session = Vec<Line>;
//!
//!     /// A word the subject's scripts can call, in every shell.
//!     fn startup(&self) -> Startup {
//!         Startup { bash: "STAGE() { BC_INSTR say STAGE \"$@\"; }".into(), ..Default::default() }
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
//!         let target = asked.words.first().cloned();
//!         heard.push(asked);
//!
//!         Ok(match target.as_deref() {
//!             Some("target") => Answer::of("declare", ["-g", "target=staging"]),
//!             _ => Answer::status(1),
//!         })
//!     }
//! }
//!
//! let (heard, status) = run(&Deploying, &["bash", "deploy.bash"])?.whole()?;
//!
//! for line in &heard {
//!     println!("pid {} said {:?}", line.pid, line.words);
//! }
//! # Ok::<(), Failure>(())
//! ```
//!
//! An answer is a command the shell runs, so its expressiveness is bash's.
//! Only `open` is required: `startup`, `transform_command`, `hear`, `answer`
//! and `end` default to injecting nothing, running the command line as asked,
//! keeping nothing, saying the word is unknown, and doing nothing.
//!
//! The command line carries its own program, so a run is not bound to bash at
//! the top: instrumentation travels by `BASH_ENV`, and any bash the subject
//! starts joins the wire whether or not the subject is one.

mod run;
mod tree;
mod wire;

use std::ffi::OsString;
use std::fmt;

/// What a run produced.
///
/// Reaching one of these means bash was started and seen out. A `Failure`
/// instead means the run never got that far: it could not be set up, or the
/// rig could not do its work and the subject was killed — and then how the
/// subject would have ended is not something anyone can say.
pub struct Run<S> {
    /// The client's own state, whatever it made of what it heard.
    pub session: S,

    /// How bash ended. Always its own: the run serves until the subject
    /// leaves of its own accord, whether or not anything went wrong.
    pub subject: ExitStatus,

    /// What went wrong closing up, if anything: a message left half-read, or
    /// a session that would not let go. Both happen after the subject reached
    /// its own end, so `subject` is news of its own either way.
    pub failed: Option<Failure>,
}

impl<S> Run<S> {
    /// The session, if nothing went wrong — the shape a caller wants when a
    /// partial reading is no use to it.
    pub fn whole(self) -> Result<(S, ExitStatus), Failure> {
        match self.failed {
            Some(why) => Err(why),
            None => Ok((self.session, self.subject)),
        }
    }
}

/// What a rig tells the run about the process it is about to start.
#[derive(Default)]
pub struct Startup {
    /// Injected into every shell, after the protocol's own. This is the only
    /// half descendants see: `BASH_ENV` reaches them, a command line does
    /// not.
    pub bash: String,

    /// Added to the environment the subject is started with, beside the
    /// `BASH_ENV` the run sets itself.
    pub env: Vec<(OsString, OsString)>,
}

pub use run::{run, run_in};

pub use tree::{forest, shells, Shell, ShellNode};
pub use wire::{field, Answer, Kind, Line, Micros, Pid};

pub use crate::failure::{Doing, Failure};

/// The functional definition of a run. A rig says what bash it needs, how its
/// session is initialised, and how that session reacts; the run does
/// everything else.
pub trait Rig {
    /// The client's state. No bounds, no lifetime: the run stores nothing of
    /// its own in it, and hands it back when the run is over.
    type Session;

    /// What the run needs before there is a shell to talk to.
    fn startup(&self) -> Startup {
        Startup::default()
    }

    /// The command line actually run, given the one the caller asked for —
    /// which carries its own program, so a rig may put a launcher in front,
    /// wrap the payload, or replace it outright. Identity by default.
    fn transform_command(&self, argv: Vec<OsString>) -> Vec<OsString> {
        argv
    }

    fn open(&self) -> Result<Self::Session, Failure>;

    /// A message nobody is waiting on.
    ///
    /// A `Failure` from either this or [`answer`](Rig::answer) ends the run:
    /// the subject is killed and `run` yields that reason. It is not told and
    /// not given a status to interpret — a rig that cannot do its work is not
    /// something bash can be asked about.
    fn hear(&self, _session: &mut Self::Session, _said: Line) -> Result<(), Failure> {
        Ok(())
    }

    /// A message a shell is blocked on; the run frames what comes back and
    /// writes it to that shell. Hearing the question and telling the shell
    /// the word is unknown, unless a rig says otherwise.
    ///
    /// An answer is a command, and every answer is the same kind of thing.
    /// Saying no is a command that returns non-zero — what the subject makes
    /// of that is its own business, and the run only waits to see.
    fn answer(&self, session: &mut Self::Session, asked: Line) -> Result<Answer, Failure> {
        self.hear(session, asked)?;

        Ok(Answer::status(127))
    }

    /// The subject is gone; release what the session holds. Reached once per
    /// run that started a subject; a session opened before a setup failure is
    /// dropped instead.
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

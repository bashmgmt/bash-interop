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
//!     println!("pid {} said {:?}", line.sent.pid, line.words);
//! }
//! # Ok::<(), Failure>(())
//! ```
//!
//! An answer is a command the shell runs, so its expressiveness is bash's.
//! Only `open` is required: `startup`, `hear`, `answer` and `end` default to
//! injecting nothing, keeping nothing, saying the word is unknown, and doing
//! nothing.
//!
//! The command line is run as it is given, and carries its own program — so a
//! run is not bound to bash at the top, and a caller wanting a launcher puts
//! one there. Instrumentation travels by `BASH_ENV` instead, which is what
//! reaches the shells a command line never could.

mod run;
mod status;
mod tree;
mod wire;

use std::ffi::OsString;
use std::path::PathBuf;

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

/// Where the run lays its bash and its pipes, and how long that outlives the
/// run.
///
/// A frame's source path is only as readable as the file it names, and the
/// instrument's own frames name a file in here — so anything that reads a walk
/// after the run has to say where the run put it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Workspace {
    /// A directory of the run's own, removed when it ends.
    #[default]
    Temporary,

    /// One of the caller's, created if it is not there and left behind.
    At(PathBuf),
}

/// Everything a rig tells the run about the process it is about to start.
#[derive(Default)]
pub struct Startup {
    /// Injected into every shell, after the protocol's own. This is the only
    /// half descendants see: `BASH_ENV` reaches them, a command line does
    /// not.
    pub bash: String,

    /// Added to the environment the subject is started with, beside the
    /// `BASH_ENV` the run sets itself.
    pub env: Vec<(OsString, OsString)>,

    /// Where the run lays that bash, and how long it outlives the run. A rig
    /// whose reading resolves a frame's source afterwards names a directory it
    /// keeps.
    pub workspace: Workspace,
}

pub use run::run;
pub use status::ExitStatus;

pub use tree::{forest, shells, Shell, ShellNode};
pub use wire::{field, Answer, Kind, Line, Micros, Pid, Sent};

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

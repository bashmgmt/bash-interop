//! Run bash under instrumentation, and hear what it says.
//!
//! ```bash
//! BC_INSTR say REC compiled "$target"    # ship an arglist and carry on
//! BC_INSTR ask which target              # ship one, block, run the answer
//! ```
//!
//! A [`Rig`] is the reaction: the bash it gives the subject, the session it
//! keeps, and what it does with what arrives. It says nothing about who
//! started what — that is a second question with exactly two answers, and each
//! is a trait that carries its own orchestration:
//!
//! | | who started the subject | what ends it | what comes back |
//! |---|---|---|---|
//! | [`Master`] | the run — `BASH_ENV`, own process group | bash reached its own end | [`Run`], with the subject's [`ExitStatus`] |
//! | [`Slave`] | a bash script, which took the address | the rig said so | [`Served`], with [`Closed`] |
//!
//! Both have both exits: a rig may [`Halt::Done`] under `Master` too, and a
//! `Slave` session whose initiator vanished ends on its handle. Which of the
//! two is the ordinary one is a fact about who started what, so the serving
//! loop knows neither and reports which was taken.
//!
//! ```no_run
//! use mb_resolver::bash::rig::{Answer, Failure, Halt, Line, Master, Rig};
//!
//! /// Keeps every message, and tells a shell that asks to use staging.
//! struct Deploying;
//!
//! impl Rig for Deploying {
//!     type Session = Vec<Line>;
//!
//!     /// A word the subject's scripts can call, in every shell.
//!     fn bash(&self) -> String {
//!         "STAGE() { BC_INSTR say STAGE \"$@\"; }".to_string()
//!     }
//!
//!     fn open(&self) -> Result<Vec<Line>, Failure> {
//!         Ok(Vec::new())
//!     }
//!
//!     fn hear(&self, heard: &mut Vec<Line>, said: Line) -> Result<(), Halt> {
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
//! impl Master for Deploying {}
//!
//! let (heard, status) = Deploying.run(&["bash", "deploy.bash"])?.whole()?;
//!
//! for line in &heard {
//!     println!("pid {} said {:?}", line.sent.pid, line.words);
//! }
//! # Ok::<(), Failure>(())
//! ```
//!
//! An answer is a command the shell runs, so its expressiveness is bash's.
//! Only `open` is required: the rest default to giving the subject no words of
//! its own, a workspace the run throws away, keeping nothing, saying the word
//! is unknown, and doing nothing at the end.

mod master;
mod serving;
mod slave;
mod status;
mod tree;
mod wire;

use std::path::PathBuf;

/// Where a session lays its bash and its pipes, and how long that outlives it.
///
/// A frame's source path is only as readable as the file it names, and the
/// instrument's own frames name a file in here — so anything that reads a walk
/// afterwards has to say where the run put it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Workspace {
    /// A directory of the session's own, removed when it ends.
    #[default]
    Temporary,

    /// One of the caller's, created if it is not there and left behind.
    At(PathBuf),
}

/// Why a rig stopped: because it is finished, or because nothing more can be
/// done. A `Failure` raised anywhere in a rig's own code becomes `Failed`
/// through `?`, so only `Done` is ever written out.
#[derive(Debug)]
pub enum Halt {
    Done,
    Failed(Failure),
}

impl From<Failure> for Halt {
    fn from(why: Failure) -> Self {
        Self::Failed(why)
    }
}

pub use master::{Master, Run};
pub use serving::{Closed, Served};
pub use slave::{Held, Slave};
pub use status::ExitStatus;

pub use tree::{forest, shells, Shell, ShellNode};
pub use wire::{field, Answer, Kind, Line, Micros, Pid, Sent};

pub use crate::failure::{Doing, Failure};

/// The reaction inside the protocol a rig defines: what bash it gives the
/// subject, how its session is opened, and what it does with what arrives.
///
/// Nothing here knows who started the subject. [`Master`] and [`Slave`] are
/// where that is decided, and a rig declares which of them it supports by
/// implementing it.
pub trait Rig {
    /// The client's state. No bounds, no lifetime: the session stores nothing
    /// of its own in it, and hands it back when the conversation is over.
    type Session;

    /// The words this rig gives the subject, laid beside the protocol's own
    /// and sourced by it. The same text in either orchestration.
    fn bash(&self) -> String {
        String::new()
    }

    /// Where the session's files go, and how long they outlive it. A rig whose
    /// reading resolves a frame's source afterwards names a directory it keeps.
    fn workspace(&self) -> Workspace {
        Workspace::Temporary
    }

    fn open(&self) -> Result<Self::Session, Failure>;

    /// A message nobody is waiting on.
    ///
    /// [`Halt::Done`] ends the conversation cleanly and is the whole of what a
    /// `Slave` client's closing word means. [`Halt::Failed`] ends it as a
    /// fault: under `Master` the subject is killed and the run yields that
    /// reason. Neither is negotiated with bash — a rig that cannot do its work
    /// is not something bash can be asked about.
    fn hear(&self, _session: &mut Self::Session, _said: Line) -> Result<(), Halt> {
        Ok(())
    }

    /// A message a shell is blocked on; the session frames what comes back and
    /// writes it to that shell. Telling the shell the word is unknown, unless
    /// a rig says otherwise.
    ///
    /// It cannot halt, and that is a rule the signature carries: the asking
    /// shell is blocked on a pipe it holds read-write, so there is no end of
    /// input and no timeout, and a rig that walked away from a question would
    /// hang it for good.
    ///
    /// An answer is a command, and every answer is the same kind of thing.
    /// Saying no is a command that returns non-zero — what the subject makes of
    /// that is its own business, and the session only waits to see.
    fn answer(&self, _session: &mut Self::Session, _asked: Line) -> Result<Answer, Failure> {
        Ok(Answer::status(127))
    }

    /// The conversation is over; release what the session holds. Reached once
    /// per serving that got as far as opening one.
    fn end(&self, _session: &mut Self::Session) -> Result<(), Failure> {
        Ok(())
    }
}

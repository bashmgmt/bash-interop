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
//! | | who started the shells | what the session lasts for | what comes back |
//! |---|---|---|---|
//! | [`Master`] | the run — `BASH_ENV`, own process group | that process group | [`Run`], with the subject's [`ExitStatus`] |
//! | [`Slave`] | a bash script, which took the address | whoever holds the handle | [`Served`] |
//!
//! One sentence covers both: **a session lasts as long as anyone who could
//! still speak.** Nothing inside a rig ends one.
//!
//! How a shell learns the address is its own business and neither role's: a
//! driven run puts it in `BASH_ENV`, a client that started the server sources
//! what it was handed, and a shell that wants in for its own reasons — an
//! interactive child, say — sources either. However it got there, the first
//! thing it says is [`Kind::Join`], its own account of itself, and [`Shells`]
//! is where that is read.
//!
//! ```no_run
//! use mb_resolver::bash::rig::{Answer, Failure, Line, Master, Rig};
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
//! its own, a workspace the run throws away, keeping nothing, hearing a
//! question and saying the word is unknown, and doing nothing at the end.

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

pub use master::{Master, Run};
pub use slave::{Served, Slave};
pub use status::ExitStatus;

pub use tree::{forest, shells, At, Joined, Shell, ShellNode, Shells};
pub use wire::{field, Answer, Kind, Line, Micros, Pid, Sent};

pub use crate::bash::shell::{Bash, Flags, Started, State, Version};

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

    /// A message nobody is waiting on, which is a [`Say`](Kind::Say) or the
    /// [`Join`](Kind::Join) a shell opens with. A rig that reads walks keeps
    /// the joins, since a walk is read against the shell it was taken in; one
    /// that claims its messages by tag ignores them without trying.
    ///
    /// A `Failure` from either this or [`answer`](Rig::answer) ends the
    /// conversation: under `Master` the subject is killed and the run yields
    /// that reason. It is not negotiated with bash — a rig that cannot do its
    /// work is not something bash can be asked about.
    fn hear(&self, _session: &mut Self::Session, _said: Line) -> Result<(), Failure> {
        Ok(())
    }

    /// A message a shell is blocked on; the session frames what comes back and
    /// writes it to that shell. Hearing the question and telling the shell the
    /// word is unknown, unless a rig says otherwise.
    ///
    /// An answer is a command, and every answer is the same kind of thing.
    /// Saying no is a command that returns non-zero — what the subject makes of
    /// that is its own business, and the session only waits to see.
    fn answer(&self, session: &mut Self::Session, asked: Line) -> Result<Answer, Failure> {
        self.hear(session, asked)?;

        Ok(Answer::status(127))
    }

    /// The conversation is over; release what the session holds. Reached once
    /// per serving that got as far as opening one.
    fn end(&self, _session: &mut Self::Session) -> Result<(), Failure> {
        Ok(())
    }
}

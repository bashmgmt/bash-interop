//! Run bash under instrumentation, and hear what it says.
//!
//! ```bash
//! BC_INSTR say REC compiled "$target"    # ship an arglist and carry on
//! BC_INSTR ask which target              # ship one, block, run the answer
//! ```
//!
//! A [`Rig`] is a **description**: the bash it gives the subject, where the
//! session's files go, and how to build a reaction once a shell is there. The
//! reaction is [`Reacting`], and it is made **per shell**, at the moment that
//! shell announces itself — so which bash it is, how it was started and what it
//! had switched on are members from construction, never parameters.
//!
//! ```no_run
//! use std::sync::Arc;
//! use mb_resolver::bash::rig::{Answer, Failure, Laid, Line, Master, Reacting, Rig, Shell};
//!
//! /// Keeps what one shell said, and tells it to use staging.
//! struct Deploying;
//!
//! struct Told { shell: Arc<Shell>, heard: Vec<Line> }
//!
//! impl Rig for Deploying {
//!     type Attending = Told;
//!
//!     /// A word the subject's scripts can call, in every shell.
//!     fn bash(&self) -> String {
//!         "STAGE() { BC_INSTR say STAGE \"$@\"; }".to_string()
//!     }
//!
//!     fn joined(&self, _at: &Laid, shell: Arc<Shell>) -> Result<Told, Failure> {
//!         Ok(Told { shell, heard: Vec::new() })
//!     }
//! }
//!
//! impl Reacting for Told {
//!     type Kept = Self;
//!
//!     fn hear(&mut self, said: Line) -> Result<(), Failure> {
//!         self.heard.push(said);
//!         Ok(())
//!     }
//!
//!     fn answer(&mut self, asked: Line) -> Result<Answer, Failure> {
//!         Ok(match asked.words.first().map(String::as_str) {
//!             Some("target") => Answer::of("declare", ["-g", "target=staging"]),
//!             _ => Answer::status(1),
//!         })
//!     }
//!
//!     fn finish(self) -> Result<Self, Failure> { Ok(self) }
//! }
//!
//! impl Master for Deploying {}
//!
//! for shell in Deploying.run(&["bash", "deploy.bash"])?.whole()?.shells {
//!     println!("pid {} said {} things", shell.shell.pid, shell.kept.heard.len());
//! }
//! # Ok::<(), Failure>(())
//! ```
//!
//! Who started the shells is a second question with exactly two answers, and
//! each is a trait that carries its own orchestration:
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
//! thing it says is its account of itself, and that is what makes a [`Shell`].

mod master;
mod serving;
mod slave;
mod tree;
pub(crate) mod wire;

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

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

/// Where the session's files ended up. Handed to every reaction at
/// construction, so one that resolves paths afterwards knows where the
/// instrument's own frames point without being told twice.
#[derive(Clone, Debug)]
pub struct Laid {
    pub dir: PathBuf,

    /// The file a shell sources to join — the session's only address.
    pub prelude: PathBuf,
}

/// What one shell's reaction leaves behind, for a given rig.
pub type Kept<R> = <<R as Rig>::Attending as Reacting>::Kept;

/// One shell, and what its reaction left behind.
#[derive(Debug)]
pub struct Attended<K> {
    pub shell: Arc<Shell>,
    pub kept: K,
}

/// One message, and the shell that sent it.
///
/// A reaction has both by construction. Anything reading a run afterwards needs
/// them together too, since a frame walk means nothing without the shell it was
/// taken in.
#[derive(Copy, Clone, Debug, Serialize)]
pub struct Said<'a> {
    pub shell: &'a Arc<Shell>,
    pub line: &'a Line,
}

/// Everything the shells said, in the order the run heard it.
///
/// A run folds per shell, so each shell's own order is kept and nothing else.
/// `Sent::nth` counts messages over the whole run and is what puts them back
/// together.
pub fn heard<K: AsRef<[Line]>>(shells: &[Attended<K>]) -> Vec<Said<'_>> {
    let mut said: Vec<Said<'_>> = shells
        .iter()
        .flat_map(|at| at.kept.as_ref().iter().map(|line| Said { shell: &at.shell, line }))
        .collect();

    said.sort_by_key(|said| said.line.sent.nth);
    said
}

pub use master::{ExitStatus, Master, Run, Whole};
pub use slave::{Served, Slave};
pub use tree::{forest, ShellNode};

pub use wire::{field, Answer, Kind, Line, Micros, Pid, Sent};

pub use crate::bash::shell::Shell;
pub use crate::failure::{Doing, Failure};

/// What bash a rig gives the subject, where the session's files go, and how a
/// reaction is made once a shell is there.
///
/// A description: `&self` throughout, because nothing about it changes by
/// running.
pub trait Rig {
    /// What reacts to one shell.
    type Attending: Reacting;

    /// The words this rig gives the subject, laid beside the protocol's own and
    /// sourced by it. The same text in either orchestration.
    fn bash(&self) -> String {
        String::new()
    }

    fn workspace(&self) -> Workspace {
        Workspace::Temporary
    }

    /// A shell has joined, and everything about it is known. This is where it
    /// enters, and the last time it is a parameter.
    fn joined(&self, at: &Laid, shell: Arc<Shell>) -> Result<Self::Attending, Failure>;
}

/// One shell's reaction, for as long as that shell can speak.
///
/// It owns its state, so nothing is threaded through it. What is shared across
/// shells — a sink, an index — is the caller's own and comes in through
/// [`Rig::joined`], which is why the core names no sharing discipline.
pub trait Reacting: Sized {
    /// What is left when the shell can no longer speak. `Self` where nothing is
    /// released at the end.
    type Kept;

    /// A message nobody is waiting on.
    ///
    /// A `Failure` from either this or [`answer`](Reacting::answer) ends the
    /// conversation: under `Master` the subject is killed and the run yields
    /// that reason. It is not negotiated with bash — a reaction that cannot do
    /// its work is not something bash can be asked about.
    fn hear(&mut self, _said: Line) -> Result<(), Failure> {
        Ok(())
    }

    /// A message this shell is blocked on; the session frames what comes back
    /// and writes it there.
    ///
    /// An answer is a command, and every answer is the same kind of thing.
    /// Saying no is a command that returns non-zero — what the subject makes of
    /// that is its own business, and the session only waits to see.
    fn answer(&mut self, asked: Line) -> Result<Answer, Failure> {
        self.hear(asked)?;

        Ok(Answer::status(127))
    }

    /// The conversation is over; release what this held.
    fn finish(self) -> Result<Self::Kept, Failure>;
}

/// A reaction that keeps every message.
impl Reacting for Vec<Line> {
    type Kept = Self;

    fn hear(&mut self, said: Line) -> Result<(), Failure> {
        self.push(said);

        Ok(())
    }

    fn finish(self) -> Result<Self, Failure> {
        Ok(self)
    }
}

/// A reaction that keeps nothing and answers nothing.
impl Reacting for () {
    type Kept = Self;

    fn finish(self) -> Result<Self, Failure> {
        Ok(())
    }
}

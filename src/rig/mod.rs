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
//! use mb_resolver::bash::rig::{
//!     Answer, Driving, Failure, Layout, Message, Reacting, Rig, Shell, Workspace,
//! };
//!
//! /// Keeps what one shell said, and tells it to use staging.
//! struct Deploying;
//!
//! struct Told { shell: Arc<Shell>, heard: Vec<Message> }
//!
//! impl Rig for Deploying {
//!     type Reaction = Told;
//!
//!     /// A word the subject's scripts can call, in every shell.
//!     fn bash(&self) -> String {
//!         "STAGE() { BC_INSTR say STAGE \"$@\"; }".to_string()
//!     }
//!
//!     fn workspace(&self) -> Workspace { Workspace::Temporary }
//!
//!     fn joined(&self, _at: &Layout, shell: Arc<Shell>) -> Result<Told, Failure> {
//!         Ok(Told { shell, heard: Vec::new() })
//!     }
//! }
//!
//! impl Reacting for Told {
//!     type Kept = Self;
//!
//!     fn hear(&mut self, said: Message) -> Result<(), Failure> {
//!         self.heard.push(said);
//!         Ok(())
//!     }
//!
//!     fn answer(&mut self, asked: Message) -> Result<Answer, Failure> {
//!         Ok(match asked.words.first().map(String::as_str) {
//!             Some("target") => Answer::of("declare", ["-g", "target=staging"]),
//!             _ => Answer::unknown(),
//!         })
//!     }
//!
//!     fn finish(self) -> Result<Self, Failure> { Ok(self) }
//! }
//!
//! impl Driving for Deploying {}
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
//! | [`Driving`] | the run — `BASH_ENV`, own process group | that process group | [`Run`], with the subject's [`ExitStatus`] |
//! | [`Serving`] | a bash script, which took the address | whoever holds the handle | [`Served`] |
//!
//! One sentence covers both: **a session lasts as long as anyone who could
//! still speak.** Nothing inside a rig ends one.
//!
//! How a shell learns the address is its own business and neither role's: a
//! driven run puts it in `BASH_ENV`, a client that started the server sources
//! what it was handed, and a shell that wants in for its own reasons — an
//! interactive child, say — sources either. However it got there, the first
//! thing it says is its account of itself, and that is what makes a [`Shell`].
//!
//! | | |
//! |---|---|
//! | `attended` | [`Workspace`], [`Layout`], [`Attended`], [`Kept`], [`Said`], [`heard`] |
//! | `forest` | [`ShellNode`] and [`forest`] — who started whom |
//! | `session` | the conversation: the workspace, the wire, one reaction per shell |
//! | `watch` | the descriptor a session ends on |
//! | `driving`, `serving` | the two roles, and what each hands back |
//! | `wire` | [`Message`], [`Answer`], and the protocol that carries them |

mod attended;
mod driving;
mod forest;
mod serving;
mod session;
mod watch;
pub(crate) mod wire;

use std::sync::Arc;

pub use attended::{heard, Attended, Kept, Layout, Said, Workspace};
pub use driving::{Driving, ExitStatus, Run, Whole};
pub use forest::{forest, ShellNode};
pub use serving::{Served, Serving};

pub use wire::{field, Answer, Message, Micros, Pid, Stamp, Verb};

pub use crate::bash::shell::Shell;
pub use crate::failure::{Doing, Failure};

/// What bash a rig gives the subject, where the session's files go, and how a
/// reaction is made once a shell is there.
///
/// A description: `&self` throughout, because nothing about it changes by
/// running. **No method has a default body** — an `impl Rig` block is the whole
/// contract, and a rig with nothing of its own writes `String::new()` and
/// `Workspace::Temporary` where it means them.
///
/// | it is handed | it produces |
/// |---|---|
/// | [`&Layout`](Layout) — `dir`, and `prelude`, the address a shell sources | [`Self::Reaction`](Rig::Reaction) |
/// | [`Arc<Shell>`](Shell) — `bash: Bash`, `options: Options`, `joined: Stamp` | |
///
/// [`bash`](Rig::bash) is laid beside the protocol's own by the session;
/// [`stack::with_walk`](crate::bash::stack::with_walk) composes it where the
/// rig reports a frame walk.
pub trait Rig {
    /// What reacts to one shell.
    type Reaction: Reacting;

    /// The words this rig gives the subject, laid beside the protocol's own and
    /// sourced by it. The same text in either orchestration.
    fn bash(&self) -> String;

    /// Where the session's files go, and how long they outlive it.
    fn workspace(&self) -> Workspace;

    /// A shell has joined, and everything about it is known. This is where it
    /// enters, and the last time it is a parameter.
    fn joined(&self, at: &Layout, shell: Arc<Shell>) -> Result<Self::Reaction, Failure>;
}

/// One shell's reaction, for as long as that shell can speak.
///
/// It owns its state, so nothing is threaded through it. What is shared across
/// shells — a sink, a merged view — is the caller's own and comes in through
/// [`Rig::joined`], which is why the core names no sharing discipline.
///
/// | | |
/// |---|---|
/// | [`hear`](Reacting::hear) | a [`Message`] nobody is waiting on |
/// | [`answer`](Reacting::answer) | one the shell blocks on; the session writes the [`Answer`] back to it |
/// | [`finish`](Reacting::finish) | what is left, which lands in [`Attended::kept`] |
///
/// **No method has a default body.** A reaction that drops what it hears, or
/// refuses every question, says so itself; the two implementations below are
/// the templates to copy. [`heard`] puts the per-shell foldings back into
/// arrival order, by [`Stamp::nth`].
pub trait Reacting: Sized {
    /// What is left when the shell can no longer speak. `Self` where nothing is
    /// released at the end.
    type Kept;

    /// A message nobody is waiting on.
    ///
    /// A `Failure` from either this or [`answer`](Reacting::answer) ends the
    /// conversation: under [`Driving`] the subject is killed and the run yields
    /// that reason. It is not negotiated with bash — a reaction that cannot do
    /// its work is not something bash can be asked about.
    fn hear(&mut self, said: Message) -> Result<(), Failure>;

    /// A message this shell is blocked on; the session frames what comes back
    /// and writes it there.
    ///
    /// An answer is a command, and every answer is the same kind of thing.
    /// Saying no is a command that returns non-zero — [`Answer::unknown`] is
    /// the one for a word this rig has no answer for — and what the subject
    /// makes of that is its own business.
    fn answer(&mut self, asked: Message) -> Result<Answer, Failure>;

    /// The conversation is over; release what this held.
    fn finish(self) -> Result<Self::Kept, Failure>;
}

/// A reaction that keeps every message, and has no answer to any of them.
impl Reacting for Vec<Message> {
    type Kept = Self;

    fn hear(&mut self, said: Message) -> Result<(), Failure> {
        self.push(said);

        Ok(())
    }

    fn answer(&mut self, asked: Message) -> Result<Answer, Failure> {
        self.hear(asked)?;

        Ok(Answer::unknown())
    }

    fn finish(self) -> Result<Self, Failure> {
        Ok(self)
    }
}

/// A reaction that keeps nothing and answers nothing.
impl Reacting for () {
    type Kept = Self;

    fn hear(&mut self, _said: Message) -> Result<(), Failure> {
        Ok(())
    }

    fn answer(&mut self, _asked: Message) -> Result<Answer, Failure> {
        Ok(Answer::unknown())
    }

    fn finish(self) -> Result<Self, Failure> {
        Ok(())
    }
}

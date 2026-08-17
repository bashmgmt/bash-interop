//! Run bash under instrumentation, and hear what it says.
//!
//! ```bash
//! declare -- BC_SAY__ARG_LABEL=DEPLOY
//! BC_SAY REC compiled "$target"                 # ship an arglist and carry on
//!
//! declare -- BC_ASK__ARG_LABEL=DEPLOY           # ask: block, then run the
//! declare -a BC_ASK__ARGS=(which target)        # answer here, in this frame
//! BC_ASK
//! ```
//!
//! A [`Rig`] is a **description**: the bash it gives the subject, where the
//! session's files go, and how to build a reaction once a shell is there. The
//! reaction is [`Reacting`], and it is made **per shell**, at the moment that
//! shell announces itself — so which bash it is, how it was started and what
//! it had switched on are members from construction, never parameters. Every
//! shell has a pipe of its own and a task of its own, so serving many shells
//! is many straight-line loops that interleave.
//!
//! ```no_run
//! use std::sync::Arc;
//! use bash_interop::rig::{
//!     Answer, Driving, Failure, Layout, Message, Provision, Reacting, Rig, Shell,
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
//!     /// A word the subject's scripts can call, as one command over the
//!     /// core's, so it composes where any command does. Definitions only:
//!     /// sourcing this joins nothing.
//!     fn bash(&self, _at: &Layout) -> String {
//!         "alias STAGE='BC_SAY__ARG_LABEL=DEPLOY BC_SAY STAGE'\n".to_string()
//!     }
//!
//!     async fn joined(&self, _at: &Layout, shell: Arc<Shell>) -> Result<Told, Failure> {
//!         Ok(Told { shell, heard: Vec::new() })
//!     }
//! }
//!
//! /// The standard initiation — data the run's closure hands to `bash_env`;
//! /// run only where a client or a provisioned file says so.
//! fn deploy_join(at: &Layout) -> String {
//!     format!("BC_JOIN DEPLOY {}\n", bash_strings::emit_scalar(at.text()))
//! }
//!
//! impl Reacting for Told {
//!     type Kept = Self;
//!
//!     async fn hear(&mut self, said: Message) -> Result<(), Failure> {
//!         self.heard.push(said);
//!         Ok(())
//!     }
//!
//!     async fn answer(&mut self, asked: Message) -> Result<Answer, Failure> {
//!         Ok(match asked.words.first().map(String::as_str) {
//!             Some("target") => Answer::of("declare", ["target=staging"]),
//!             _ => Answer::unknown(),
//!         })
//!     }
//!
//!     async fn finish(self) -> Result<Self, Failure> { Ok(self) }
//! }
//!
//! impl Driving for Deploying {}
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), Failure> {
//! // The closure's return is the subject's whole environment. Provisioning
//! // a joining file is the one auto-initiation there is, and it is stated
//! // here, at the fringe — every other shell's initiation is its own code.
//! let ran = Deploying
//!     .run(&["bash", "deploy.bash"], |at| {
//!         Ok(vec![at.bash_env(Provision::Joining(&deploy_join(at)))?])
//!     })
//!     .await?;
//! for shell in ran.whole()?.shells {
//!     println!("pid {} said {} things", shell.shell.pid, shell.kept.heard.len());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Who started the shells is a second question with exactly two answers, and
//! each is a trait that carries its own orchestration:
//!
//! | | who started the shells | how they find the address | what the session lasts for | what comes back |
//! |---|---|---|---|---|
//! | [`Driving`] | the run, in a process group of its own | exactly what the run's environment closure returned — [`Layout::bash_env`] with a stated [`Provision`] the usual pair | that process group | [`Run`], with the subject's [`ExitStatus`] |
//! | [`Serving`] | a bash script, which named and made the workspace and started the server | its own choice: it feeds the same directory to start, probe, load and initiate | whoever holds the handle | [`Served`] |
//!
//! Either way, the address is the workspace directory. Loading its laid
//! files defines; initiation is the client's own line — except where a
//! provisioned `bash_env.bash` states [`Provision::Joining`], the one
//! auto-initiation there is. The book's `docs/joining.md` shows every way
//! a script joins, each as a whole script.
//!
//! A session lasts as long as anyone who could still speak, and nothing inside
//! a rig ends one.
//!
//! The session is single-threaded: one `current_thread` runtime, one task per
//! shell, and no `Send` bound anywhere. What shells share — a sink, a merged
//! view — belongs to the caller and is handed in through [`Rig::joined`] as an
//! `Rc<RefCell<_>>` or whatever it likes; a `RefCell` borrow must not be held
//! across an `.await`.
//!
//! | | |
//! |---|---|
//! | `attended` | [`Layout`], [`Attended`], [`Kept`], [`Said`], [`heard`] |
//! | `session`, `attend` | the conversation: the workspace, the control fifo, one task per shell |
//! | `watch` | the descriptor a session ends on |
//! | `driving`, `serving` | the two roles, and what each hands back |
//! | `wire` | [`Message`], [`Answer`], and the protocol that carries them |

mod attend;
mod attended;
mod driving;
mod serving;
mod session;
mod watch;
pub(crate) mod wire;

use std::sync::Arc;

pub use attended::{Attended, Kept, Layout, Provision, Said, heard};
pub use driving::{Driving, ExitStatus, Run, Whole};
pub use serving::{Served, Serving};

pub use wire::{Answer, Message, Micros, Pid, Stamp, Verb, field};

pub use crate::failure::{Doing, Failure};
pub use crate::shell::Shell;

/// What bash a rig gives the subject, and how a reaction is made once a
/// shell is there.
///
/// A description: `&self` throughout, because nothing about it changes by
/// running. **No method has a default body.**
///
/// | it is handed | it produces |
/// |---|---|
/// | [`&Layout`](Layout) — the workspace, and the files in it | [`Self::Reaction`](Rig::Reaction) |
/// | [`Arc<Shell>`](Shell) — `bash: Bash`, `options: Options`, `brought`, `joined: Stamp` | |
///
/// The rig's bash is laid beside the protocol's own by the session;
/// [`stack::with_walk`](crate::stack::with_walk) composes it where the
/// rig reports a frame walk.
// ANCHOR: rig-trait
#[expect(async_fn_in_trait, reason = "single-threaded by design: no Send bound")]
pub trait Rig {
    /// What reacts to one shell.
    type Reaction: Reacting;

    /// The rig's own bash: **definitions only**. Its words, and at most a
    /// channel-init function; sourcing it has no effect on a shell beyond
    /// names coming into being, so it is inert, re-sourceable, and free of
    /// the coordinate unless its author bakes one in.
    fn bash(&self, at: &Layout) -> String;

    /// A shell has joined, and everything about it is known. Awaited in the
    /// accept loop, so a slow `joined` delays the next join and nothing else.
    async fn joined(&self, at: &Layout, shell: Arc<Shell>) -> Result<Self::Reaction, Failure>;
}
// ANCHOR_END: rig-trait

/// One shell's reaction, for as long as that shell can speak.
///
/// It runs as a task of its own, so it owns what it holds (`'static`) and is
/// never sent to another thread. Awaiting inside a method yields to the other
/// shells' tasks; synchronous work blocks them for its duration.
///
/// | | |
/// |---|---|
/// | [`hear`](Reacting::hear) | a [`Message`] nobody is waiting on |
/// | [`answer`](Reacting::answer) | one the shell blocks on; the task writes the [`Answer`] back to it |
/// | [`finish`](Reacting::finish) | what is left, which lands in [`Attended::kept`] |
///
/// **No method has a default body.** The two implementations below are the
/// templates to copy.
// ANCHOR: reacting-trait
#[expect(async_fn_in_trait, reason = "single-threaded by design: no Send bound")]
pub trait Reacting: Sized + 'static {
    /// What is left when the shell can no longer speak. `Self` where nothing
    /// is released at the end.
    type Kept: 'static;

    /// A `Failure` from this or [`answer`](Reacting::answer) ends the
    /// conversation: under [`Driving`] the subject is killed and the run
    /// yields that reason.
    async fn hear(&mut self, said: Message) -> Result<(), Failure>;

    /// An answer is a command, and every answer is the same kind of thing.
    /// Saying no is a command that returns non-zero — [`Answer::unknown`] for
    /// a word this rig has no answer for.
    async fn answer(&mut self, asked: Message) -> Result<Answer, Failure>;

    /// The conversation is over; release what this held.
    async fn finish(self) -> Result<Self::Kept, Failure>;
}
// ANCHOR_END: reacting-trait

/// A reaction that keeps every message, and has no answer to any of them.
impl Reacting for Vec<Message> {
    type Kept = Self;

    async fn hear(&mut self, said: Message) -> Result<(), Failure> {
        self.push(said);

        Ok(())
    }

    async fn answer(&mut self, asked: Message) -> Result<Answer, Failure> {
        self.hear(asked).await?;

        Ok(Answer::unknown())
    }

    async fn finish(self) -> Result<Self, Failure> {
        Ok(self)
    }
}

/// A reaction that keeps nothing and answers nothing.
impl Reacting for () {
    type Kept = Self;

    async fn hear(&mut self, _said: Message) -> Result<(), Failure> {
        Ok(())
    }

    async fn answer(&mut self, _asked: Message) -> Result<Answer, Failure> {
        Ok(Answer::unknown())
    }

    async fn finish(self) -> Result<Self, Failure> {
        Ok(())
    }
}

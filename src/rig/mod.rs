//! Run bash under instrumentation, and hear what it says.
//!
//! ```bash
//! BC_INSTR DEPLOY say REC compiled "$target"    # ship an arglist and carry on
//! BC_INSTR DEPLOY ask which target              # ship one, block, run the answer
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
//! use mb_resolver::bash::rig::{
//!     Answer, Driving, Failure, Layout, Message, Reached, Reaching, Reacting, Rig, Setup, Shell,
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
//!     /// A word the subject's scripts can call, in every shell, and the label
//!     /// it speaks under.
//!     fn setup(&self) -> Setup {
//!         Setup {
//!             label: "DEPLOY".to_string(),
//!             bash: "STAGE() { BC_INSTR DEPLOY say STAGE \"$@\"; }\n".to_string(),
//!         }
//!     }
//!
//!     async fn joined(&self, _at: &Layout, shell: Arc<Shell>) -> Result<Told, Failure> {
//!         Ok(Told { shell, heard: Vec::new() })
//!     }
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
//!             Some("target") => Answer::of("declare", ["-g", "target=staging"]),
//!             _ => Answer::unknown(),
//!         })
//!     }
//!
//!     async fn finish(self) -> Result<Self, Failure> { Ok(self) }
//! }
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), Failure> {
//! // Driven with the usual reach: `BASH_ENV`, so every non-interactive bash
//! // in the tree joins as it starts. A rig with an environment of its own
//! // implements `Driving` instead.
//! let ran = Reached { rig: Deploying, reaching: Reaching::BashEnv };
//! for shell in ran.run(&["bash", "deploy.bash"]).await?.whole()?.shells {
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
//! | [`Driving`] | the run, in a process group of its own | `BC_SESSION` in the environment, plus what [`Driving::environment`] adds | that process group | [`Run`], with the subject's [`ExitStatus`] |
//! | [`Serving`] | a bash script, which named the workspace and started the server | its own choice: `<dir>/session.bash`, echoed back once the session is laid | whoever holds the handle | [`Served`] |
//!
//! Either way, the address is the file a shell sources to join, and
//! [`JOINING`] shows every way a script does that.
//!
//! **A session lasts as long as anyone who could still speak.** Nothing inside
//! a rig ends one.
//!
//! The session is single-threaded: one `current_thread` runtime, one task per
//! shell, and no `Send` bound anywhere. What shells share — a sink, a merged
//! view — is the caller's own, handed in through [`Rig::joined`] as an
//! `Rc<RefCell<_>>` or whatever it likes; a `RefCell` borrow must not be held
//! across an `.await`.
//!
//! | | |
//! |---|---|
//! | `attended` | [`Setup`], [`Layout`], [`Attended`], [`Kept`], [`Said`], [`heard`] |
//! | `session`, `attend` | the conversation: the workspace, the control fifo, one task per shell |
//! | `watch` | the descriptor a session ends on |
//! | `driving`, `serving` | the two roles, [`Reached`] and [`Reaching`], and what each hands back |
//! | `wire` | [`Message`], [`Answer`], and the protocol that carries them |

mod attend;
mod attended;
mod driving;
mod serving;
mod session;
mod watch;
pub(crate) mod wire;

use std::sync::Arc;

pub use attended::{heard, Attended, Kept, Layout, Said, Setup};
pub use driving::{Driving, ExitStatus, Reached, Reaching, Run, Whole};
pub use serving::{Served, Serving};

pub use wire::{field, Answer, Message, Micros, Pid, Stamp, Verb};

pub use crate::bash::shell::Shell;
pub use crate::failure::{Doing, Failure};

/// How a bash script joins a session, in every way there is. Both tools print
/// it under `--help`.
///
/// ```bash
#[doc = include_str!("joining.txt")]
/// ```
pub const JOINING: &str = include_str!("joining.txt");

/// What bash a rig gives the subject, where the session's files go, and how a
/// reaction is made once a shell is there.
///
/// A description: `&self` throughout, because nothing about it changes by
/// running. **No method has a default body.**
///
/// | it is handed | it produces |
/// |---|---|
/// | [`&Layout`](Layout) — `dir`, and `address`, what a shell sources | [`Self::Reaction`](Rig::Reaction) |
/// | [`Arc<Shell>`](Shell) — `bash: Bash`, `options: Options`, `joined: Stamp` | |
///
/// [`Setup::bash`] is laid beside the protocol's own by the session;
/// [`stack::with_walk`](crate::bash::stack::with_walk) composes it where the
/// rig reports a frame walk.
#[expect(async_fn_in_trait, reason = "single-threaded by design: no Send bound")]
pub trait Rig {
    /// What reacts to one shell.
    type Reaction: Reacting;

    fn setup(&self) -> Setup;

    /// A shell has joined, and everything about it is known. Awaited in the
    /// accept loop, so a slow `joined` delays the next join and nothing else.
    async fn joined(&self, at: &Layout, shell: Arc<Shell>) -> Result<Self::Reaction, Failure>;
}

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

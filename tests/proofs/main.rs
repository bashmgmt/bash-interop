//! Bash-level proofs: each spawns real bash to cover one mechanism that
//! cannot be checked by reading the generated source.
//!
//! They use nothing but the public surface, which is the point — if a proof
//! needed an internal, the surface would be missing something.
//!
//! | | |
//! |---|---|
//! | [`attaching`] | a shell's pipe: the rendezvous, a fork's own, two labels in one shell |
//! | [`transport`] | every shell reaches the run, and a message arrives whole |
//! | [`transparency`] | what the subject keeps: its status, its trap, its `IFS` |
//! | [`answering`] | every form of answer, from two shells at once, one waiting on the other |
//! | [`starting`] | what a driven run starts, and what a rig puts in every shell |
//! | [`serving`] | a session a script joins for itself, and how far it reaches |
//! | [`owning`] | the run's workspace, and the process group it takes with it |
//! | [`failing`] | what a fault on either side does to the run and to the subject |
//! | [`malformed`] | what the reader does with a line the protocol did not write |
//!
//! This file holds what more than one of them needs.
//!
//! `cargo test --test proofs`

mod answering;
mod attaching;
mod failing;
mod malformed;
mod owning;
mod serving;
mod starting;
mod transparency;
mod transport;

#[path = "../support/mod.rs"]
mod support;

use std::ffi::OsString;
use std::sync::Arc;

use mb_resolver::bash::rig::{
    heard, Attended, Driving, Failure, Layout, Message, Reaching, Rig, Setup, Shell, Whole,
};

use support::{bash, Scripts};

/// Every proof starts the same script, beside whatever else it wrote.
pub const ENTRY: &str = "main.bash";

/// The label the proofs' scripts speak under: `BC_INSTR KEEP say …`.
pub const LABEL: &str = "KEEP";

/// Keeps every message, and answers nothing.
pub struct Keeping {
    reaching: Reaching,
}

impl Keeping {
    /// Every shell of the subject's tree joins as it starts.
    pub fn bash_env() -> Self {
        Self { reaching: Reaching::BashEnv }
    }

    /// The address alone: a script joins where it says `source "$BC_SESSION"`.
    pub fn by_hand() -> Self {
        Self { reaching: Reaching::ByHand }
    }
}

impl Rig for Keeping {
    type Reaction = Vec<Message>;

    /// No words of its own in the subject's shells: only the label.
    fn setup(&self) -> Setup {
        Setup { label: LABEL.to_string(), bash: String::new() }
    }

    async fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Vec<Message>, Failure> {
        Ok(Vec::new())
    }
}

impl Driving for Keeping {
    fn environment(&self, at: &Layout) -> Vec<(OsString, OsString)> {
        self.reaching.environment(at)
    }
}

/// A run of that rig, taken whole: every proof that expects a run to go through
/// takes it that way, since a partial reading proves nothing.
pub type Ran = Whole<Vec<Message>>;

pub async fn running(files: &[(&str, &str)]) -> Ran {
    let scripts = Scripts::of(files);
    let ran = Keeping::bash_env()
        .run(&bash(scripts.at(ENTRY)))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    ran.whole().unwrap_or_else(|error| panic!("{error}"))
}

pub async fn script(body: &str) -> Ran {
    running(&[(ENTRY, body)]).await
}

/// Every message the run heard, whichever shell said it, in the order it was
/// said.
pub fn lines<K: AsRef<[Message]>>(shells: &[Attended<K>]) -> Vec<&Message> {
    heard(shells).into_iter().map(|said| said.message).collect()
}

/// Every message that begins with `lead`, as the words behind it. Words, not
/// a joined string: the boundaries are what the wire is for.
pub fn behind<'a, K: AsRef<[Message]>>(shells: &'a [Attended<K>], lead: &str) -> Vec<&'a [String]> {
    lines(shells).into_iter().filter_map(|message| message.behind(lead)).collect()
}

/// How many of those begin with `word`.
pub fn beginning(messages: &[&[String]], word: &str) -> usize {
    messages.iter().filter(|words| words.first().is_some_and(|first| first == word)).count()
}

/// Whether a pid is gone. The kill is immediate; the reaping is init's and
/// takes a moment.
pub fn gone(pid: i32) -> bool {
    for _ in 0..100 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

/// Everything that happened, for an assertion message.
pub fn report<K: AsRef<[Message]>>(shells: &[Attended<K>]) -> String {
    let lines: Vec<String> = heard(shells)
        .iter()
        .map(|said| format!("  pid {:>7} | {}", said.shell.pid, said.message.words.join(" ")))
        .collect();

    format!("\ncapture:\n{}", lines.join("\n"))
}

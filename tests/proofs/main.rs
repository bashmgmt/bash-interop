//! Bash-level proofs: each spawns real bash to cover one mechanism that
//! cannot be checked by reading the generated source.
//!
//! They use nothing but the public surface, which is the point — if a proof
//! needed an internal, the surface would be missing something.
//!
//! | | |
//! |---|---|
//! | [`transport`] | every shell reaches the wire, and a message arrives whole |
//! | [`transparency`] | what the subject keeps: its status, its trap, its `IFS` |
//! | [`answering`] | every form of answer, under load, from two shells at once |
//! | [`owning`] | the run's workspace, and the process group it takes with it |
//! | [`failing`] | what a rig's own failure does to the run and to the subject |
//!
//! This file holds what more than one of them needs.
//!
//! `cargo test --test proofs`

mod answering;
mod failing;
mod owning;
mod transparency;
mod transport;

#[path = "../support/mod.rs"]
mod support;

use mb_resolver::bash::rig::{run, ExitStatus, Failure, Line, Rig};

use support::{bash, Scripts};

/// Every proof starts the same script, beside whatever else it wrote.
pub const ENTRY: &str = "main.bash";

/// Keeps every message in arrival order, and answers nothing — the default
/// `answer` hears the question and tells the shell the word is unknown.
pub struct Keeping;

impl Rig for Keeping {
    type Session = Vec<Line>;

    fn open(&self) -> Result<Vec<Line>, Failure> {
        Ok(Vec::new())
    }

    fn hear(&self, heard: &mut Vec<Line>, said: Line) -> Result<(), Failure> {
        heard.push(said);

        Ok(())
    }
}

/// Every proof that expects a run to go through takes it whole: a partial
/// reading proves nothing.
pub fn heard(files: &[(&str, &str)]) -> (Vec<Line>, ExitStatus) {
    let scripts = Scripts::of(files);
    let ran = run(&Keeping, &bash(scripts.at(ENTRY))).unwrap_or_else(|error| panic!("{error}"));

    ran.whole().unwrap_or_else(|error| panic!("{error}"))
}

pub fn script(body: &str) -> (Vec<Line>, ExitStatus) {
    heard(&[(ENTRY, body)])
}

/// Every message that begins with `lead`, as the words behind it. Words, not
/// a joined string: the boundaries are what the wire is for.
pub fn behind<'a>(heard: &'a [Line], lead: &str) -> Vec<&'a [String]> {
    heard.iter().filter_map(|line| line.behind(lead)).collect()
}

/// How many of those begin with `word`.
pub fn beginning(messages: &[&[String]], word: &str) -> usize {
    messages.iter().filter(|words| words.first().is_some_and(|first| first == word)).count()
}

/// Everything that happened, for an assertion message.
pub fn report(heard: &[Line]) -> String {
    let lines: Vec<String> = heard
        .iter()
        .map(|line| format!("  pid {:>7} | {}", line.pid, line.words.join(" ")))
        .collect();

    format!("\ncapture:\n{}", lines.join("\n"))
}

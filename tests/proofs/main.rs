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
//! | [`starting`] | what a rig tells the run about the process it starts |
//! | [`owning`] | the run's workspace, and the process group it takes with it |
//! | [`failing`] | what a fault on either side does to the run and to the subject |
//! | [`malformed`] | what the reader does with a stream the protocol did not write |
//!
//! This file holds what more than one of them needs.
//!
//! `cargo test --test proofs`

mod answering;
mod failing;
mod malformed;
mod owning;
mod serving;
mod starting;
mod transparency;
mod transport;

#[path = "../support/mod.rs"]
mod support;

use mb_resolver::bash::rig::{Answer, ExitStatus, Failure, Halt, Line, Master, Rig, Workspace};

use support::{bash, Scripts};

/// Every proof starts the same script, beside whatever else it wrote.
pub const ENTRY: &str = "main.bash";

/// Keeps every message in arrival order, questions included, and tells a shell
/// that asks that the word is unknown.
#[derive(Default)]
pub struct Keeping {
    workspace: Workspace,
}

impl Keeping {
    /// A workspace of the caller's, left behind to read.
    pub fn at(path: &std::path::Path) -> Self {
        Self { workspace: Workspace::At(path.to_path_buf()) }
    }
}

impl Rig for Keeping {
    type Session = Vec<Line>;

    fn workspace(&self) -> Workspace {
        self.workspace.clone()
    }

    fn open(&self) -> Result<Vec<Line>, Failure> {
        Ok(Vec::new())
    }

    fn hear(&self, heard: &mut Vec<Line>, said: Line) -> Result<(), Halt> {
        heard.push(said);

        Ok(())
    }

    fn answer(&self, heard: &mut Vec<Line>, asked: Line) -> Result<Answer, Failure> {
        heard.push(asked);

        Ok(Answer::status(127))
    }
}

impl Master for Keeping {}

/// Every proof that expects a run to go through takes it whole: a partial
/// reading proves nothing.
pub fn heard(files: &[(&str, &str)]) -> (Vec<Line>, ExitStatus) {
    let scripts = Scripts::of(files);
    let ran = Keeping::default().run(&bash(scripts.at(ENTRY))).unwrap_or_else(|error| panic!("{error}"));

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
pub fn report(heard: &[Line]) -> String {
    let lines: Vec<String> = heard
        .iter()
        .map(|line| format!("  pid {:>7} | {}", line.sent.pid, line.words.join(" ")))
        .collect();

    format!("\ncapture:\n{}", lines.join("\n"))
}

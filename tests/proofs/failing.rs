//! When the rig cannot do its work. That is the run's failure, not a
//! conversation with the subject: `run` ends in the reason, and the subject is
//! killed rather than told something and left to interpret it.

use std::sync::Arc;
use std::time::Instant;

use mb_resolver::bash::rig::{
    Answer, Driving, ExitStatus, Failure, Layout, Message, Reacting, Rig, Shell, Verb, Workspace,
};

use crate::support::{bash, Scripts};
use crate::{behind, gone, report, script, Keeping, ENTRY};

/// Fails the first time it is given a message of the kind it breaks on.
struct Breaking {
    on: Verb,
}

struct Breaks {
    on: Verb,
    heard: Vec<Message>,
}

impl Rig for Breaking {
    type Reaction = Breaks;

    /// No words of its own in the subject's shells.
    fn bash(&self) -> String {
        String::new()
    }

    fn workspace(&self) -> Workspace {
        Workspace::Temporary
    }

    fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Breaks, Failure> {
        Ok(Breaks { on: self.on, heard: Vec::new() })
    }
}

impl Reacting for Breaks {
    type Kept = Vec<Message>;

    fn hear(&mut self, said: Message) -> Result<(), Failure> {
        if self.on == Verb::Say {
            return Err(Failure::new("keeping what was said", "the sink is on fire"));
        }
        self.heard.push(said);

        Ok(())
    }

    fn answer(&mut self, _: Message) -> Result<Answer, Failure> {
        Err(Failure::new("deciding an answer", "the operator is on fire"))
    }

    fn finish(self) -> Result<Vec<Message>, Failure> {
        Ok(self.heard)
    }
}

impl Driving for Breaking {}

/// The subject reports its own pid before the message that breaks the rig, so
/// a proof can ask whether it outlived the run.
const REPORTING: &str = r#"echo $BASHPID > "${BASH_SOURCE[0]%/*}/pid"
"#;

fn blocked(scripts: &Scripts) -> i32 {
    std::fs::read_to_string(scripts.at("pid"))
        .expect("the subject reported its pid")
        .trim()
        .parse()
        .expect("a pid")
}

/// A shell blocked on an ask can never be answered once the rig has failed,
/// so it is killed. Nothing is written back: an answer is a command, and there
/// is no command that means "the operator broke".
#[test]
fn a_rig_that_cannot_answer_ends_the_run_and_kills_the_subject() {
    let scripts = Scripts::of(&[(ENTRY, &format!("{REPORTING}BC_INSTR ask anything"))]);

    let failure = Breaking { on: Verb::Ask }.run(&bash(scripts.at(ENTRY)))
        .err()
        .expect("the run must end in the rig's failure");

    assert!(failure.to_string().contains("the operator is on fire"), "{failure}");
    assert!(gone(blocked(&scripts)), "the shell was left waiting for an answer never coming");
}

/// `hear` has nobody waiting on it, so there was never anything to write back.
/// The run ends the same way, and does not wait for a subject that would have
/// gone on for another half minute.
#[test]
fn a_failure_while_hearing_ends_the_run_and_kills_the_subject() {
    let scripts = Scripts::of(&[(
        ENTRY,
        &format!(
            r#"{REPORTING}
            BC_INSTR say REC one
            sleep 30
            "#
        ),
    )]);

    let started = Instant::now();
    let failure = Breaking { on: Verb::Say }.run(&bash(scripts.at(ENTRY)))
        .err()
        .expect("the run must end in the rig's failure");

    assert!(failure.to_string().contains("the sink is on fire"), "{failure}");
    assert!(started.elapsed().as_secs() < 5, "the run must not wait the subject out");
    assert!(gone(blocked(&scripts)), "the subject outlived the run");
}

/// A reply pipe's name is a shell's own and is free again before that shell
/// can ask anything else, so one already taken was left by something else.
///
/// Adopting it would in fact work — a pipe is a rendezvous and does not care
/// who made it. `mkfifo` is a single attempt anyway: the protocol does not
/// build on state it did not create, and the shell is told at its own call
/// site rather than carrying on over it.
#[test]
fn a_reply_pipe_name_already_taken_is_the_subjects_to_handle() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        exec 2> "${BASH_SOURCE[0]%/*}/err"
        mkfifo "$__BC__DIR/rep.$BASHPID"

        BC_INSTR ask something
        echo "ask returned $?" >&2
        BC_INSTR say REC "still running"
        "#,
    )]);

    let ran = Keeping::default().run(&bash(scripts.at(ENTRY))).unwrap().whole().unwrap();

    assert_eq!(ran.subject, ExitStatus::Code(0), "the subject carried on and ended its own way");
    assert_eq!(behind(&ran.shells, "REC"), [["still running"]], "{}", report(&ran.shells));

    let said = std::fs::read_to_string(scripts.at("err")).unwrap();
    assert!(said.contains("ask returned 125"), "the instrumentation failed: {said:?}");
    assert!(said.contains("__bc_ask"), "naming what broke: {said:?}");
    assert!(said.contains(&format!("{ENTRY}:5")), "at the subject's own call site: {said:?}");
}

/// A verb the protocol does not define is the client's mistake and stays in
/// the client's shell: it is named on stderr, returns 125, and the run carries
/// on knowing nothing about it.
#[test]
fn an_unknown_verb_is_reported_rather_than_ignored() {
    let ran = script(
        r#"
        complaint="$(BC_INSTR mumble something 2>&1)"
        BC_INSTR say REC "returned $?" "$complaint"
        "#,
    );

    assert_eq!(ran.subject, ExitStatus::Code(0), "the subject carries on");

    let said = behind(&ran.shells, "REC");
    assert_eq!(said.len(), 1, "{}", report(&ran.shells));
    assert_eq!(said[0][0], "returned 125", "the instrumentation failed{}", report(&ran.shells));
    assert!(said[0][1].contains("unknown verb mumble"), "it says which: {:?}", said[0][1]);
}

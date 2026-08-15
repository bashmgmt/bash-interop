//! When the rig cannot do its work. That is the run's failure, not a
//! conversation with the subject: `run` ends in the reason, and the subject is
//! killed rather than told something and left to interpret it. What the
//! subject gets wrong stays the subject's.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::Instant;

use mb_resolver::bash::rig::{
    Answer, Driving, ExitStatus, Failure, Layout, Message, Reaching, Reacting, Rig, Setup, Shell,
    Verb,
};

use crate::support::{bash, Scripts};
use crate::{behind, gone, report, script, ENTRY, LABEL};

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

    fn setup(&self) -> Setup {
        Setup { label: LABEL.to_string(), bash: String::new() }
    }

    async fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Breaks, Failure> {
        Ok(Breaks { on: self.on, heard: Vec::new() })
    }
}

impl Reacting for Breaks {
    type Kept = Vec<Message>;

    async fn hear(&mut self, said: Message) -> Result<(), Failure> {
        if self.on == Verb::Say {
            return Err(Failure::new("keeping what was said", "the sink is on fire"));
        }
        self.heard.push(said);

        Ok(())
    }

    async fn answer(&mut self, _: Message) -> Result<Answer, Failure> {
        match self.on {
            Verb::Ask => Err(Failure::new("deciding an answer", "the operator is on fire")),
            Verb::Say => Ok(Answer::status(0)),
        }
    }

    async fn finish(self) -> Result<Vec<Message>, Failure> {
        Ok(self.heard)
    }
}

impl Driving for Breaking {
    fn environment(&self, at: &Layout) -> Vec<(OsString, OsString)> {
        Reaching::BashEnv.environment(at)
    }
}

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
#[tokio::test]
async fn a_rig_that_cannot_answer_ends_the_run_and_kills_the_subject() {
    let scripts = Scripts::of(&[(ENTRY, &format!("{REPORTING}BC_INSTR KEEP ask anything"))]);

    let failure = Breaking { on: Verb::Ask }
        .run(&bash(scripts.at(ENTRY)))
        .await
        .err()
        .expect("the run must end in the rig's failure");

    assert!(failure.to_string().contains("the operator is on fire"), "{failure}");
    assert!(gone(blocked(&scripts)), "the shell was left waiting for an answer never coming");
}

/// `hear` has nobody waiting on it, so there was never anything to write back.
/// The run ends the same way, promptly, while another shell is mid-message —
/// a task's failure reaches the session on its next turn, not at the end.
#[tokio::test]
async fn a_failure_while_hearing_ends_the_run_and_kills_the_subject() {
    let scripts = Scripts::of(&[(
        ENTRY,
        &format!(
            r#"{REPORTING}
            bash -c 'while :; do BC_INSTR KEEP ask nothing || :; sleep 0.01; done' &
            sleep 0.1
            BC_INSTR KEEP say REC one
            sleep 30
            "#
        ),
    )]);

    let started = Instant::now();
    let failure = Breaking { on: Verb::Say }
        .run(&bash(scripts.at(ENTRY)))
        .await
        .err()
        .expect("the run must end in the rig's failure");

    assert!(failure.to_string().contains("the sink is on fire"), "{failure}");
    assert!(started.elapsed().as_secs() < 5, "the run must not wait the subject out");
    assert!(gone(blocked(&scripts)), "the subject outlived the run");
}

/// A verb the protocol does not define is the client's mistake and stays in
/// the client's shell: it is named on stderr, returns 125, and the run carries
/// on knowing nothing about it.
#[tokio::test]
async fn an_unknown_verb_is_reported_rather_than_ignored() {
    let ran = script(
        r#"
        complaint="$(BC_INSTR KEEP mumble something 2>&1)"
        BC_INSTR KEEP say REC "returned $?" "$complaint"
        "#,
    )
    .await;

    assert_eq!(ran.subject, ExitStatus::Code(0), "the subject carries on");

    let said = behind(&ran.shells, "REC");
    assert_eq!(said.len(), 1, "{}", report(&ran.shells));
    assert_eq!(said[0][0], "returned 125", "the instrumentation failed{}", report(&ran.shells));
    assert!(said[0][1].contains("unknown verb mumble"), "it says which: {:?}", said[0][1]);
}

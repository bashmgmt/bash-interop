//! When the instrumentation itself is what failed. Every one of these ends
//! in status 125 at the subject's call site, and in a reason on its stderr.

use mb_resolver::bash::rig::{run, Answer, ExitStatus, Failure, Kind, Line, Rig};

use crate::support::{bash, Scripts};
use crate::{behind, report, script, ENTRY};

/// Fails the first time it is asked anything, and keeps whatever it heard.
struct Breaking {
    on: Kind,
}

impl Rig for Breaking {
    type Session = Vec<Line>;

    fn open(&self) -> Result<Vec<Line>, Failure> {
        Ok(Vec::new())
    }

    fn hear(&self, heard: &mut Vec<Line>, said: Line) -> Result<(), Failure> {
        if self.on == Kind::Say {
            return Err(Failure::new("keeping what was said", "the sink is on fire"));
        }
        heard.push(said);

        Ok(())
    }

    fn answer(&self, _: &mut Vec<Line>, _: Line) -> Result<Answer, Failure> {
        Err(Failure::new("deciding an answer", "the operator is on fire"))
    }
}

/// The reason reaches the shell that was blocked, at its own call site, and
/// `BC_INSTR ask` reports that the instrumentation failed rather than that
/// the answer returned something. Killing the subject first would lose all of
/// it: a refusal written and then followed by a signal never arrives.
#[test]
fn a_rig_that_cannot_answer_tells_the_shell_why() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        exec 2> "${BASH_SOURCE[0]%/*}/err"
        BC_INSTR ask anything
        echo "ask returned $?" >&2
        BC_INSTR say REC still running
        "#,
    )]);

    let ran = run(&Breaking { on: Kind::Ask }, &bash(scripts.at(ENTRY))).unwrap();
    let failure = ran.failed.expect("the run must report the rig's failure");

    assert!(failure.to_string().contains("the operator is on fire"), "{failure}");
    assert_eq!(ran.subject, ExitStatus::Code(0), "and the subject's own status survives it");

    let said = std::fs::read_to_string(scripts.at("err")).unwrap();
    assert!(said.contains("the operator is on fire"), "the shell was told why: {said:?}");
    assert!(said.contains(&format!("{ENTRY}:3")), "at its own call site: {said:?}");
    assert!(said.contains("ask returned 125"), "the instrumentation failed, not the answer");
}

/// A subject that asked for `set -e` gets it: the refusal is an ordinary
/// failing command, so its own error handling ends the script.
#[test]
fn a_refusal_is_an_ordinary_failure_the_subject_may_act_on() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        set -e
        exec 2> "${BASH_SOURCE[0]%/*}/err"
        BC_INSTR ask anything
        echo "NOT REACHED" >&2
        "#,
    )]);

    let ran = run(&Breaking { on: Kind::Ask }, &bash(scripts.at(ENTRY))).unwrap();
    assert!(ran.failed.is_some(), "the run reports the rig's failure");
    assert_eq!(ran.subject, ExitStatus::Code(125), "set -e ended the subject at the refusal");

    let said = std::fs::read_to_string(scripts.at("err")).unwrap();
    assert!(said.contains("the operator is on fire"), "{said:?}");
    assert!(!said.contains("NOT REACHED"), "set -e ended the script at the ask: {said:?}");
}

/// `hear` has no one waiting, so nothing can be said at the time. The run
/// still ends in the failure, and the next shell to ask is told the reason.
#[test]
fn a_failure_while_hearing_still_ends_the_run_and_refuses_later_asks() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        exec 2> "${BASH_SOURCE[0]%/*}/err"
        BC_INSTR say REC one
        BC_INSTR ask anything
        echo "ask returned $?" >&2
        "#,
    )]);

    let ran = run(&Breaking { on: Kind::Say }, &bash(scripts.at(ENTRY))).unwrap();
    let failure = ran.failed.expect("the run must report the rig's failure");

    assert!(failure.to_string().contains("the sink is on fire"), "{failure}");

    let said = std::fs::read_to_string(scripts.at("err")).unwrap();
    assert!(said.contains("the sink is on fire"), "the later ask carried it: {said:?}");
    assert!(said.contains("ask returned 125"), "{said:?}");
}

/// A verb the protocol does not define is a client's mistake, reported the
/// same way and leaving the shell able to carry on.
#[test]
fn an_unknown_verb_is_reported_rather_than_ignored() {
    let (seen, status) = script(
        r#"
        complaint="$(BC_INSTR mumble something 2>&1)"
        BC_INSTR say REC "returned $?" "$complaint"
        "#,
    );

    assert_eq!(status, ExitStatus::Code(0), "the subject carries on");

    let said = behind(&seen, "REC");
    assert_eq!(said.len(), 1, "{}", report(&seen));
    assert_eq!(said[0][0], "returned 125", "the instrumentation failed{}", report(&seen));
    assert!(said[0][1].contains("unknown verb mumble"), "it says which: {:?}", said[0][1]);
}

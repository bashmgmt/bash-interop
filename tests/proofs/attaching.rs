//! A shell's pipe: made by the shell, opened by the run, held for the shell's
//! life. Every process that sources the prelude attaches; a fork attaches on
//! its first word.

use std::sync::Arc;

use mb_resolver::bash::rig::{
    Driving, ExitStatus, Failure, Layout, Message, Rig, Setup, Shell, Workspace,
};

use crate::support::{bash, Scripts};
use crate::{behind, report, running, script, ENTRY};

/// The blocking open is the rendezvous: a shell that joins, says one thing and
/// exits within microseconds loses nothing, because it cannot write before the
/// run has its pipe in hand.
#[tokio::test]
async fn a_shell_that_speaks_once_and_leaves_loses_nothing() {
    for round in 0..10 {
        let ran = script("bash -c 'BC_INSTR KEEP say REC quick'\n").await;

        assert_eq!(behind(&ran.shells, "REC"), [["quick"]], "round {round}{}", report(&ran.shells));
        assert_eq!(ran.shells.len(), 2, "the script's shell and the one it started");
    }
}

/// A fork that speaks takes a pipe of its own and parts when it exits; the
/// process it forked from is a shell of its own, and lives on.
#[tokio::test]
async fn a_fork_that_speaks_is_a_shell_of_its_own_and_parts_on_its_own() {
    let ran = script(
        r#"
        BC_INSTR KEEP say REC parent "$BASHPID"
        ( BC_INSTR KEEP say REC fork "$BASHPID" )
        sleep 0.2
        BC_INSTR KEEP say REC still-here
        "#,
    )
    .await;

    assert_eq!(ran.subject, ExitStatus::Code(0));
    assert_eq!(ran.shells.len(), 2, "{}", report(&ran.shells));

    let (parent, fork) = (&ran.shells[0], &ran.shells[1]);
    assert_ne!(parent.shell.pid, fork.shell.pid);
    assert_eq!(fork.shell.subshell, 1, "what bash says of it");
    assert_eq!(fork.kept.len(), 1, "one word, on its own pipe");
    assert_eq!(parent.kept.len(), 2, "and the parent's own stay the parent's");

    let (fork_parted, parent_parted) =
        (fork.parted.expect("the fork exited"), parent.parted.expect("the subject exited"));
    assert!(fork_parted < parent_parted, "the fork went first, by its own pipe's end of input");
    assert!(fork_parted < parent.kept[1].stamp.sent_at, "and before the parent's last word");
}

/// Two labels in one shell are two sessions of one process: `BC_JOIN` twice
/// gives it two pipes, and Rust hears two shells with the same pid.
struct Twice;

impl Rig for Twice {
    type Reaction = Vec<Message>;

    fn setup(&self) -> Setup {
        Setup { bash: "BC_JOIN ONE\nBC_JOIN TWO\n".to_string(), workspace: Workspace::Temporary }
    }

    async fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Vec<Message>, Failure> {
        Ok(Vec::new())
    }
}

impl Driving for Twice {}

#[tokio::test]
async fn two_labels_in_one_process_are_two_shells() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        BC_INSTR ONE say REC one
        BC_INSTR TWO say REC two
        BC_INSTR ONE say REC one-again
        "#,
    )]);

    let ran = Twice.run(&bash(scripts.at(ENTRY))).await.unwrap().whole().unwrap();

    assert_eq!(ran.shells.len(), 2, "{}", report(&ran.shells));
    assert_eq!(ran.shells[0].shell.pid, ran.shells[1].shell.pid, "one process");
    assert_eq!(behind(&ran.shells[..1], "REC"), [["one"], ["one-again"]]);
    assert_eq!(behind(&ran.shells[1..], "REC"), [["two"]]);
}

/// A label nobody joined is the client's mistake and stays in the client's
/// shell: named on stderr, at the call site, status 125, and the run knows
/// nothing about it.
#[tokio::test]
async fn a_label_nobody_joined_is_an_error_by_absence() {
    let ran = running(&[(
        ENTRY,
        r#"
        complaint="$(BC_INSTR NOPE say REC lost 2>&1)"
        BC_INSTR KEEP say REC "returned $?" "$complaint"
        "#,
    )])
    .await;

    assert_eq!(ran.subject, ExitStatus::Code(0), "the subject carries on");

    let said = behind(&ran.shells, "REC");
    assert_eq!(said.len(), 1, "{}", report(&ran.shells));
    assert_eq!(said[0][0], "returned 125");
    assert!(said[0][1].contains("label NOPE is not joined"), "which label: {:?}", said[0][1]);
    assert!(said[0][1].contains(&format!("{ENTRY}:2")), "and where: {:?}", said[0][1]);
}

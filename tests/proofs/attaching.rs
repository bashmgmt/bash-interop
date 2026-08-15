//! A shell's pipe: made by the shell, announced with its account, opened by
//! the run, held for the shell's life. Every process that sources the prelude
//! attaches; a fork attaches on its first word.

use std::ffi::OsString;
use std::sync::Arc;

use mb_resolver::bash::rig::{
    Driving, ExitStatus, Failure, Layout, Message, Reaching, Rig, Setup, Shell,
};

use crate::support::{bash, Scripts};
use crate::{behind, report, running, script, Keeping, ENTRY};

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

/// Two labels in one shell are two sessions of one process: a second join
/// gives it a second pipe, and Rust hears two shells with the same pid. The
/// second join states its coordinate — the address names the workspace, so
/// `${BC_SESSION%/*}` is how a script spells it.
struct Twice;

impl Rig for Twice {
    type Reaction = Vec<Message>;

    fn setup(&self) -> Setup {
        Setup { label: "ONE".to_string(), bash: String::new() }
    }

    async fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Vec<Message>, Failure> {
        Ok(Vec::new())
    }
}

impl Driving for Twice {
    fn environment(&self, at: &Layout) -> Vec<(OsString, OsString)> {
        Reaching::BashEnv.environment(at)
    }
}

#[tokio::test]
async fn two_labels_in_one_process_are_two_shells() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        BC_JOIN TWO "${BC_SESSION%/*}"
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

/// An account crosses the control fifo in frames of at most `PIPE_BUF` bytes,
/// cut wherever the byte count falls — inside a character, here — and is put
/// back together whole: what bash was given as its command reads back byte
/// for byte, however long.
#[tokio::test]
async fn an_account_of_any_size_arrives_whole() {
    let euros = "€".repeat(7000);
    let scripts = Scripts::of(&[(
        ENTRY,
        &format!(
            r#"
            bash -c ": {euros}; BC_INSTR KEEP say REC done"
            "#
        ),
    )]);

    let ran = Keeping::bash_env().run(&bash(scripts.at(ENTRY))).await.unwrap().whole().unwrap();

    assert_eq!(behind(&ran.shells, "REC"), [["done"]], "{}", report(&ran.shells));
    let child = ran.shells.iter().find(|at| at.shell.bash.invocation.command.is_some()).unwrap();
    assert_eq!(
        child.shell.bash.invocation.command.as_deref(),
        Some(format!(": {euros}; BC_INSTR KEEP say REC done").as_str()),
        "21 KB, across six frames"
    );
}

/// Many shells announcing at once interleave their frames on the one fifo,
/// and every account comes back whole and its own.
#[tokio::test]
async fn many_shells_announce_at_once() {
    let pad = "x".repeat(6000);
    let scripts = Scripts::of(&[(
        ENTRY,
        &format!(
            r#"
            for i in $(seq 16); do
                bash -c ": $i {pad}; BC_INSTR KEEP say REC $i" &
            done
            wait
            "#
        ),
    )]);

    let ran = Keeping::bash_env().run(&bash(scripts.at(ENTRY))).await.unwrap().whole().unwrap();

    assert_eq!(ran.shells.len(), 17, "the script and sixteen children{}", report(&ran.shells));
    for at in ran.shells.iter().filter(|at| at.shell.bash.invocation.command.is_some()) {
        let said = &at.kept[0].words;
        let (i, command) = (&said[1], at.shell.bash.invocation.command.as_deref().unwrap());

        assert_eq!(command, format!(": {i} {pad}; BC_INSTR KEEP say REC {i}"), "shell {i}'s own");
    }
}

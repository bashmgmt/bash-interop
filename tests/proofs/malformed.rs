//! What the reader does with a line the protocol did not write.
//!
//! These are the one place a proof reaches past the client surface: the
//! subject writes raw lines to its own pipe, `${__BC__FD[KEEP]}`, because
//! producing a cut or unreadable one is the whole point. Everything else about
//! the run is ordinary.

use mb_resolver::bash::rig::{Driving, ExitStatus};

use crate::support::{bash, Scripts};
use crate::{behind, report, script, Keeping, ENTRY};

/// A shell that goes leaving a line without its newline is a fault, and it is
/// reported where the run stands: while the run is being served, it ends it.
#[tokio::test]
async fn a_line_cut_short_by_a_shell_that_left_ends_the_run() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        BC_INSTR KEEP say REC first
        ( BC_INSTR KEEP say REC fork; printf '(never finished' >&"${__BC__FD[KEEP]}" )
        sleep 5
        "#,
    )]);

    let failure = Keeping::default()
        .run(&bash(scripts.at(ENTRY)))
        .await
        .err()
        .expect("the half-read line must be reported");

    assert!(failure.to_string().contains("never finished"), "{failure}");
}

/// The same line, left by a shell the session outlived, is reported when the
/// run closes up — beside the subject's own status, which is news of its own.
#[tokio::test]
async fn a_line_cut_short_at_the_end_is_reported_beside_the_subjects_status() {
    let ran = Keeping::default()
        .run(&bash(
            Scripts::of(&[(
                ENTRY,
                r#"
                setsid bash -c 'BC_INSTR KEEP say REC outsider $BASHPID; printf "(never finished" >&"${__BC__FD[KEEP]}"; sleep 30' &
                sleep 0.3
                exit 3
                "#,
            )])
            .at(ENTRY),
        ))
        .await
        .unwrap();

    let outsider: i32 = behind(&ran.shells, "REC")[0][1].parse().unwrap();
    let _ = unsafe { libc::kill(outsider, libc::SIGKILL) };

    let failed = ran.failed.as_ref().expect("the half-read line must be reported");
    assert!(failed.to_string().contains("never finished"), "{failed}");
    assert_eq!(ran.subject, ExitStatus::Code(3), "and the subject's own status survives it");
}

/// A line that will not read as a message ends the run — nothing was seen out,
/// so there is no status to report — and the run says what it could not read.
#[tokio::test]
async fn a_line_that_will_not_read_ends_the_run() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        BC_INSTR KEEP say REC first
        printf '(junk\n' >&"${__BC__FD[KEEP]}"
        BC_INSTR KEEP say REC second
        sleep 5
        "#,
    )]);

    let failure = Keeping::default()
        .run(&bash(scripts.at(ENTRY)))
        .await
        .err()
        .expect("a line that will not read must end the run");

    assert!(failure.to_string().contains("(junk"), "it quotes what it could not read: {failure}");
}

/// A second account on a pipe is not a message.
#[tokio::test]
async fn an_account_out_of_place_ends_the_run() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        BC_INSTR KEEP say REC first
        printf "('JOIN' 'at=1.000000' 'pid' '1')\n" >&"${__BC__FD[KEEP]}"
        sleep 5
        "#,
    )]);

    let failure = Keeping::default()
        .run(&bash(scripts.at(ENTRY)))
        .await
        .err()
        .expect("a second account must end the run");
    assert!(failure.to_string().contains("JOIN is not a message"), "{failure}");
}

/// The lines around a fault arrive untouched: a shell's pipe is its own.
#[tokio::test]
async fn a_fault_on_one_pipe_touches_no_other() {
    let ran = script(
        r#"
        BC_INSTR KEEP say REC first
        BC_INSTR KEEP say REC second
        "#,
    )
    .await;

    assert_eq!(behind(&ran.shells, "REC"), [["first"], ["second"]], "{}", report(&ran.shells));
}

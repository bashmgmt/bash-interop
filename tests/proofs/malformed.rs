//! What the reader does with a line the protocol did not write.
//!
//! These are the one place a proof reaches past the client surface: the
//! subject writes raw lines to its own pipe, `${__BC__FD[KEEP]}`, or to the
//! control fifo under `${__BC__DIR[KEEP]}`, because producing a cut or
//! unreadable one is the whole point. Everything else about the run is
//! ordinary.

use bash_interop::rig::{Driving, ExitStatus};

use crate::{ENTRY, Keeping, behind, provisioned, report, script};
use bash_interop::scratch::{Scripts, bash};

/// A shell that goes leaving a line without its newline is a fault, and it is
/// reported where the run stands: while the run is being served, it ends it.
#[tokio::test]
async fn a_line_cut_short_by_a_shell_that_left_ends_the_run() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        REC first
        ( REC fork; printf '(never finished' >&"${__BC__FD[KEEP]}" )
        sleep 5
        "#,
    )]);

    let failure = Keeping
        .run(
            &bash(scripts.at(ENTRY)),
            provisioned(&Keeping),
        )
        .await
        .err()
        .expect("the half-read line must be reported");

    assert!(
        failure.to_string().contains("never finished"),
        "{failure}"
    );
}

/// The same line, left by a shell the session outlived, is reported when the
/// run closes up — beside the subject's own status, which is news of its own.
#[tokio::test]
async fn a_line_cut_short_at_the_end_is_reported_beside_the_subjects_status() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        setsid bash -c 'REC outsider $BASHPID; printf "(never finished" >&"${__BC__FD[KEEP]}"; sleep 30' &
        sleep 0.3
        exit 3
        "#,
    )]);
    let ran = Keeping
        .run(
            &bash(scripts.at(ENTRY)),
            provisioned(&Keeping),
        )
        .await
        .unwrap();

    let outsider: i32 = behind(&ran.shells, "REC")[0][1].parse().unwrap();
    let _ = unsafe { libc::kill(outsider, libc::SIGKILL) };

    let failed = ran
        .failed
        .as_ref()
        .expect("the half-read line must be reported");
    assert!(
        failed.to_string().contains("never finished"),
        "{failed}"
    );
    assert_eq!(
        ran.subject,
        ExitStatus::Code(3),
        "and the subject's own status survives it"
    );
}

/// A line that will not read as a message ends the run — nothing was seen out,
/// so there is no status to report — and the run says what it could not read.
#[tokio::test]
async fn a_line_that_will_not_read_ends_the_run() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        REC first
        printf '(junk\n' >&"${__BC__FD[KEEP]}"
        REC second
        sleep 5
        "#,
    )]);

    let failure = Keeping
        .run(
            &bash(scripts.at(ENTRY)),
            provisioned(&Keeping),
        )
        .await
        .err()
        .expect("a line that will not read must end the run");

    assert!(
        failure.to_string().contains("(junk"),
        "it quotes what it could not read: {failure}"
    );
}

/// A line on the control fifo that is not a frame ends the run, naming what
/// it could not read: the fifo is the session's, made by the run, and a shell
/// writes it only through the protocol.
#[tokio::test]
async fn a_frame_the_protocol_did_not_write_ends_the_run() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        REC first
        printf 'nonsense\n' >"${__BC__DIR[KEEP]}/join"
        sleep 5
        "#,
    )]);

    let failure = Keeping
        .run(
            &bash(scripts.at(ENTRY)),
            provisioned(&Keeping),
        )
        .await
        .err()
        .expect("a line that is not a frame must end the run");
    assert!(
        failure.to_string().contains("\"nonsense\" is not a frame"),
        "{failure}"
    );
}

/// The lines around a fault arrive untouched: a shell's pipe is its own.
#[tokio::test]
async fn a_fault_on_one_pipe_touches_no_other() {
    let ran = script(
        r#"
        REC first
        REC second
        "#,
    )
    .await;

    assert_eq!(
        behind(&ran.shells, "REC"),
        [["first"], ["second"]],
        "{}",
        report(&ran.shells)
    );
}

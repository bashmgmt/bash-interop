//! What the reader does with a stream that is not what the protocol writes.
//!
//! These are the one place a proof reaches past the client surface: the
//! subject writes raw frames to `$__BC__up` itself, because producing a
//! truncated or colliding one is the whole point. Everything else about the
//! run is ordinary.

use mb_resolver::bash::rig::{ExitStatus, Master};

use crate::support::{bash, Scripts};
use crate::{behind, report, Keeping, ENTRY};

/// A message whose last chunk never comes is reported when the run closes up —
/// beside the subject's own status, which is news of its own. Its key is its
/// own, so the messages around it arrive untouched.
#[test]
fn an_unfinished_message_is_reported_beside_the_subjects_status() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        BC_INSTR say REC first
        printf '+ %s 99 (never finished\n' "$BASHPID" >&"$__BC__up"
        BC_INSTR say REC second
        exit 3
        "#,
    )]);

    let ran = Keeping::default().run(&bash(scripts.at(ENTRY))).unwrap();
    let failed = ran.failed.as_ref().expect("the half-read message must be reported");

    assert!(failed.to_string().contains("never finished"), "{failed}");
    assert_eq!(ran.subject, ExitStatus::Code(3), "and the subject's own status survives it");
    assert_eq!(
        behind(&ran.session, "REC"),
        [["first"], ["second"]],
        "a stalled key holds up nothing else{}",
        report(&ran.session)
    );
}

/// A chunk claiming the key the next real message will use corrupts it. The
/// reader refuses what it cannot read rather than handing on nonsense, and
/// that ends the run — nothing was seen out, so there is no status to report.
#[test]
fn a_message_that_will_not_read_ends_the_run() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        BC_INSTR say REC first
        printf '+ %s %s (junk\n' "$BASHPID" "$__BC__seq" >&"$__BC__up"
        BC_INSTR say REC second
        "#,
    )]);

    let failure = Keeping::default().run(&bash(scripts.at(ENTRY)))
        .err()
        .expect("a message that will not read must end the run");

    assert!(failure.to_string().contains("(junk"), "it quotes what it could not read: {failure}");
}

//! What the run starts, and what a rig puts in the shells it reaches.

use std::sync::Arc;

use bash_interop::rig::{Driving, ExitStatus, Failure, Layout, Message, Provision, Rig, Shell};

use crate::{ENTRY, Joins, Keeping, behind, provisioned, report};
use bash_interop::scratch::{Scripts, bash};

/// Hands the subject a word of its own, and a variable of its own.
struct Deploying;

impl Rig for Deploying {
    type Reaction = Vec<Message>;

    fn bash(&self, _at: &Layout) -> String {
        r#"
        TELL() { BC_INSTR TELL say TELL "$@"; }
        "#
        .to_string()
    }

    async fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Vec<Message>, Failure> {
        Ok(Vec::new())
    }
}

impl Driving for Deploying {}

/// The closure's return is the subject's whole environment: its word and its
/// variable reach the subject and a child it starts, because `BASH_ENV`
/// reaches both; one the command line carries arrives too — it names its own
/// program, so `env` puts one there. And a variable the closure did not
/// return — `DEPLOY_SESSION` here — is absent: the core adds nothing.
#[tokio::test]
async fn the_closures_return_is_the_subjects_whole_environment() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            TELL subject "$DEPLOY_TARGET" "$DEPLOY_STAGE" "$#"
            [[ -z ${DEPLOY_SESSION-} ]] && TELL no-handle
            bash "${BASH_SOURCE[0]%/*}/child.bash"
            "#,
        ),
        (
            "child.bash",
            r#"
            TELL child "$DEPLOY_TARGET" "$DEPLOY_STAGE"
            [[ -z ${DEPLOY_SESSION-} ]] && TELL no-handle
            "#,
        ),
    ]);

    let mut argv = vec!["env".to_string(), "DEPLOY_TARGET=staging".to_string()];
    argv.extend(
        bash(scripts.at(ENTRY))
            .iter()
            .map(|word| word.to_string_lossy().to_string()),
    );

    let ran = Deploying
        .run(&argv, |at| {
            Ok(vec![
                at.bash_env(Provision::Joining(
                    &Deploying.joining(at),
                ))?,
                ("DEPLOY_STAGE".into(), "canary".into()),
            ])
        })
        .await
        .unwrap()
        .whole()
        .unwrap();

    assert_eq!(
        ran.subject,
        ExitStatus::Code(0),
        "{}",
        report(&ran.shells)
    );
    assert_eq!(
        behind(&ran.shells, "TELL"),
        [
            ["subject", "staging", "canary", "0"].as_slice(),
            ["no-handle"].as_slice(),
            ["child", "staging", "canary"].as_slice(),
            ["no-handle"].as_slice(),
        ],
        "{}",
        report(&ran.shells)
    );
}

/// The run starts exactly the command line it was handed — no launcher in
/// front of it, and no argument the caller did not write.
#[tokio::test]
async fn the_command_line_is_run_as_asked() {
    let scripts = Scripts::of(&[(
        ENTRY,
        "BC_INSTR KEEP say REC \"$0\" \"$#\"",
    )]);
    let ran = Keeping
        .run(
            &bash(scripts.at(ENTRY)),
            provisioned(&Keeping),
        )
        .await
        .unwrap()
        .whole()
        .unwrap();

    assert_eq!(ran.subject, ExitStatus::Code(0));
    assert_eq!(
        behind(&ran.shells, "REC"),
        [[
            scripts.at(ENTRY).to_string_lossy().to_string(),
            "0".to_string()
        ]],
        "the program it names, and nothing appended{}",
        report(&ran.shells)
    );
}

/// An environment of the handle alone leaves joining to the scripts: the
/// subject sources the address where it chooses, and a child it starts, which
/// sourced nothing, is not a shell.
#[tokio::test]
async fn a_subject_may_join_by_hand_where_it_chooses() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            declare -- workspace="${DEPLOY_SESSION:?the workspace, from the run closure}"

            bash "${BASH_SOURCE[0]%/*}/other.bash"
            source "$workspace/prelude.bash"
            source "$workspace/rig.bash"
            BC_JOIN KEEP "$workspace"
            BC_INSTR KEEP say REC by-hand
            bash "${BASH_SOURCE[0]%/*}/other.bash"
            "#,
        ),
        (
            "other.bash",
            "type BC_INSTR >/dev/null 2>&1 && BC_INSTR KEEP say REC never\n",
        ),
    ]);

    let ran = Keeping
        .run(&bash(scripts.at(ENTRY)), |at| {
            Ok(vec![crate::deploy_session(at)])
        })
        .await
        .unwrap()
        .whole()
        .unwrap();

    assert_eq!(
        behind(&ran.shells, "REC"),
        [["by-hand"]],
        "{}",
        report(&ran.shells)
    );
    assert_eq!(
        ran.shells.len(),
        1,
        "the children never joined{}",
        report(&ran.shells)
    );
}

/// `Provision::Definitions` — the words exist in every shell, and nothing is
/// joined until a script says so itself; the file carries no coordinate, so
/// the closure states one beside it. The word said before the join went
/// nowhere and complained on stderr; only what follows the script's own
/// initiation is heard.
#[tokio::test]
async fn a_definitions_file_leaves_initiation_to_the_script() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        declare -- workspace="${DEPLOY_SESSION:?the workspace, from the run closure}"

        TELL defined-but-quiet 2>/dev/null
        BC_JOIN TELL "$workspace"
        TELL joined-by-choice
        "#,
    )]);

    let ran = Deploying
        .run(&bash(scripts.at(ENTRY)), |at| {
            Ok(vec![
                at.bash_env(Provision::Definitions)?,
                crate::deploy_session(at),
            ])
        })
        .await
        .unwrap()
        .whole()
        .unwrap();

    assert_eq!(
        behind(&ran.shells, "TELL"),
        [["joined-by-choice"]],
        "the word before the join went nowhere: {}",
        report(&ran.shells)
    );
}

impl crate::Joins for Deploying {
    fn joining(&self, at: &Layout) -> String {
        format!(
            "BC_JOIN TELL {}\n",
            bash_strings::emit_scalar(at.text())
        )
    }
}

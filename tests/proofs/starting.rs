//! What the run starts, and what a rig puts in the shells it reaches.

use std::ffi::OsString;
use std::sync::Arc;

use mb_resolver::bash::rig::{
    Driving, ExitStatus, Failure, Layout, Message, Rig, Setup, Shell, Workspace,
};

use crate::support::{bash, Scripts};
use crate::{behind, report, Keeping, ENTRY};

/// Hands the subject a word of its own, and a variable of its own.
struct Deploying;

impl Rig for Deploying {
    type Reaction = Vec<Message>;

    fn setup(&self) -> Setup {
        Setup {
            bash: "TELL() { BC_INSTR TELL say TELL \"$@\"; }\nBC_JOIN TELL\n".to_string(),
            workspace: Workspace::Temporary,
        }
    }

    async fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Vec<Message>, Failure> {
        Ok(Vec::new())
    }
}

impl Driving for Deploying {
    /// `BASH_ENV`, and a variable of the rig's own beside it: the environment
    /// is whatever the rig says, and the core adds the address.
    fn environment(&self, at: &Layout) -> Vec<(OsString, OsString)> {
        vec![at.bash_env(), ("DEPLOY_STAGE".into(), "canary".into())]
    }
}

/// A rig's word reaches the subject and a child it starts, because `BASH_ENV`
/// reaches both; so does the rig's variable, and so does one the command line
/// carries — it names its own program, so `env` puts one there. `BC_SESSION`
/// is in every shell either way, and is a file a shell could source.
#[tokio::test]
async fn the_rigs_word_and_environment_reach_every_shell_and_the_address_is_always_there() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            TELL subject "$DEPLOY_TARGET" "$DEPLOY_STAGE" "$#"
            [[ -r $BC_SESSION ]] && TELL address readable
            bash "${BASH_SOURCE[0]%/*}/child.bash"
            "#,
        ),
        (
            "child.bash",
            r#"
            TELL child "$DEPLOY_TARGET" "$DEPLOY_STAGE"
            [[ -r $BC_SESSION ]] && TELL address readable
            "#,
        ),
    ]);

    let mut argv = vec!["env".to_string(), "DEPLOY_TARGET=staging".to_string()];
    argv.extend(bash(scripts.at(ENTRY)).iter().map(|word| word.to_string_lossy().to_string()));

    let ran = Deploying.run(&argv).await.unwrap().whole().unwrap();

    assert_eq!(ran.subject, ExitStatus::Code(0), "{}", report(&ran.shells));
    assert_eq!(
        behind(&ran.shells, "TELL"),
        [
            ["subject", "staging", "canary", "0"].as_slice(),
            ["address", "readable"].as_slice(),
            ["child", "staging", "canary"].as_slice(),
            ["address", "readable"].as_slice(),
        ],
        "{}",
        report(&ran.shells)
    );
}

/// The run starts exactly the command line it was handed — no launcher in
/// front of it, and no argument the caller did not write.
#[tokio::test]
async fn the_command_line_is_run_as_asked() {
    let scripts = Scripts::of(&[(ENTRY, "BC_INSTR KEEP say REC \"$0\" \"$#\"")]);
    let ran = Keeping::bash_env().run(&bash(scripts.at(ENTRY))).await.unwrap().whole().unwrap();

    assert_eq!(ran.subject, ExitStatus::Code(0));
    assert_eq!(
        behind(&ran.shells, "REC"),
        [[scripts.at(ENTRY).to_string_lossy().to_string(), "0".to_string()]],
        "the program it names, and nothing appended{}",
        report(&ran.shells)
    );
}

/// A rig whose environment adds nothing leaves joining to the scripts: the
/// subject sources the address where it chooses, and a child it starts, which
/// sourced nothing, is not a shell.
#[tokio::test]
async fn a_subject_may_join_by_hand_where_it_chooses() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            bash "${BASH_SOURCE[0]%/*}/other.bash"
            source "$BC_SESSION"
            BC_INSTR KEEP say REC by-hand
            bash "${BASH_SOURCE[0]%/*}/other.bash"
            "#,
        ),
        ("other.bash", "type BC_INSTR >/dev/null 2>&1 && BC_INSTR KEEP say REC never\n"),
    ]);

    let ran = Keeping::by_hand().run(&bash(scripts.at(ENTRY))).await.unwrap().whole().unwrap();

    assert_eq!(behind(&ran.shells, "REC"), [["by-hand"]], "{}", report(&ran.shells));
    assert_eq!(ran.shells.len(), 1, "the children never joined{}", report(&ran.shells));
}

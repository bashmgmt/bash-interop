//! What the run starts, and what a rig puts in the shells it reaches.

use std::sync::Arc;

use mb_resolver::bash::rig::{
    Driving, ExitStatus, Failure, Layout, Message, Rig, Setup, Shell, Workspace,
};

use crate::support::{bash, Scripts};
use crate::{behind, report, Keeping, ENTRY};

/// Hands the subject a word of its own.
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

impl Driving for Deploying {}

/// A rig's word reaches the subject and a child it starts, because `BASH_ENV`
/// reaches both. A variable is not the run's business: the command line
/// carries its own program, so `env` puts one there, and it is inherited the
/// same way.
#[tokio::test]
async fn the_rigs_word_reaches_every_shell_and_so_does_the_callers_environment() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            TELL subject "$DEPLOY_TARGET" "$#"
            bash "${BASH_SOURCE[0]%/*}/child.bash"
            "#,
        ),
        (
            "child.bash",
            r#"
            TELL child "$DEPLOY_TARGET"
            "#,
        ),
    ]);

    let mut argv = vec!["env".to_string(), "DEPLOY_TARGET=staging".to_string()];
    argv.extend(bash(scripts.at(ENTRY)).iter().map(|word| word.to_string_lossy().to_string()));

    let ran = Deploying.run(&argv).await.unwrap().whole().unwrap();

    assert_eq!(ran.subject, ExitStatus::Code(0), "{}", report(&ran.shells));
    assert_eq!(
        behind(&ran.shells, "TELL"),
        [["subject", "staging", "0"].as_slice(), ["child", "staging"].as_slice()],
        "{}",
        report(&ran.shells)
    );
}

/// The run starts exactly the command line it was handed — no launcher in
/// front of it, and no argument the caller did not write.
#[tokio::test]
async fn the_command_line_is_run_as_asked() {
    let scripts = Scripts::of(&[(ENTRY, "BC_INSTR KEEP say REC \"$0\" \"$#\"")]);
    let ran = Keeping::default().run(&bash(scripts.at(ENTRY))).await.unwrap().whole().unwrap();

    assert_eq!(ran.subject, ExitStatus::Code(0));
    assert_eq!(
        behind(&ran.shells, "REC"),
        [[scripts.at(ENTRY).to_string_lossy().to_string(), "0".to_string()]],
        "the program it names, and nothing appended{}",
        report(&ran.shells)
    );
}

/// `BASH_ENV` is one way in. `BC_SESSION` is exported either way, so a subject
/// started with `env -u BASH_ENV` sources the prelude where it chooses — and a
/// child it starts, which sourced nothing, is not a shell.
#[tokio::test]
async fn a_subject_may_join_by_hand_where_it_chooses() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            bash "${BASH_SOURCE[0]%/*}/other.bash"
            source "$BC_SESSION/prelude.bash"
            BC_INSTR KEEP say REC by-hand
            bash "${BASH_SOURCE[0]%/*}/other.bash"
            "#,
        ),
        ("other.bash", "type BC_INSTR >/dev/null 2>&1 && BC_INSTR KEEP say REC never\n"),
    ]);
    let mut argv = vec!["env".to_string(), "-u".to_string(), "BASH_ENV".to_string()];
    argv.extend(bash(scripts.at(ENTRY)).iter().map(|word| word.to_string_lossy().to_string()));

    let ran = Keeping::default().run(&argv).await.unwrap().whole().unwrap();

    assert_eq!(behind(&ran.shells, "REC"), [["by-hand"]], "{}", report(&ran.shells));
    assert_eq!(ran.shells.len(), 1, "the children never joined{}", report(&ran.shells));
}

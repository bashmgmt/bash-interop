//! What the run starts, and what a rig puts in the shells it reaches.

use mb_resolver::bash::rig::{ExitStatus, Failure, Halt, Line, Master, Rig};

use crate::support::{bash, Scripts};
use crate::{behind, report, ENTRY};

/// Hands the subject a word of its own.
struct Deploying;

impl Rig for Deploying {
    type Session = Vec<Line>;

    fn bash(&self) -> String {
        "TELL() { BC_INSTR say TELL \"$@\"; }".to_string()
    }

    fn open(&self) -> Result<Vec<Line>, Failure> {
        Ok(Vec::new())
    }

    fn hear(&self, heard: &mut Vec<Line>, said: Line) -> Result<(), Halt> {
        heard.push(said);

        Ok(())
    }
}

impl Master for Deploying {}

/// A rig's word reaches the subject and a child it starts, because `BASH_ENV`
/// reaches both. A variable is not the run's business: the command line
/// carries its own program, so `env` puts one there, and it is inherited the
/// same way.
#[test]
fn the_rigs_word_reaches_every_shell_and_so_does_the_callers_environment() {
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

    let (seen, status) = Deploying.run(&argv).unwrap().whole().unwrap();

    assert_eq!(status, ExitStatus::Code(0), "{}", report(&seen));
    assert_eq!(
        behind(&seen, "TELL"),
        [["subject", "staging", "0"].as_slice(), ["child", "staging"].as_slice()],
        "{}",
        report(&seen)
    );
}

/// The run starts exactly the command line it was handed — no launcher in
/// front of it, and no argument the caller did not write.
#[test]
fn the_command_line_is_run_as_asked() {
    let scripts = Scripts::of(&[(ENTRY, "BC_INSTR say REC \"$0\" \"$#\"")]);
    let (seen, status) = crate::Keeping::default().run(&bash(scripts.at(ENTRY))).unwrap().whole().unwrap();

    assert_eq!(status, ExitStatus::Code(0));
    assert_eq!(
        behind(&seen, "REC"),
        [[scripts.at(ENTRY).to_string_lossy().to_string(), "0".to_string()]],
        "the program it names, and nothing appended{}",
        report(&seen)
    );
}

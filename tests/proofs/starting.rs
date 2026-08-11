//! What a rig tells the run about the process it is about to start.

use mb_resolver::bash::rig::{run, ExitStatus, Failure, Line, Rig, Startup};

use crate::support::{bash, Scripts};
use crate::{behind, report, ENTRY};

/// Hands the subject a word of its own and a variable of its own.
struct Deploying;

impl Rig for Deploying {
    type Session = Vec<Line>;

    fn startup(&self) -> Startup {
        Startup {
            bash: "TELL() { BC_INSTR say TELL \"$@\"; }".to_string(),
            env: vec![("DEPLOY_TARGET".into(), "staging".into())],
        }
    }

    fn open(&self) -> Result<Vec<Line>, Failure> {
        Ok(Vec::new())
    }

    fn hear(&self, heard: &mut Vec<Line>, said: Line) -> Result<(), Failure> {
        heard.push(said);

        Ok(())
    }
}

/// `Startup::env` reaches the subject, and so does a child it starts —
/// the environment is inherited, and `BASH_ENV` puts the rig's own word in
/// both shells.
#[test]
fn a_rig_may_add_to_the_environment_and_reach_every_shell() {
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

    let (seen, status) = run(&Deploying, &bash(scripts.at(ENTRY))).unwrap().whole().unwrap();

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
    let (seen, status) = run(&crate::Keeping, &bash(scripts.at(ENTRY))).unwrap().whole().unwrap();

    assert_eq!(status, ExitStatus::Code(0));
    assert_eq!(
        behind(&seen, "REC"),
        [[scripts.at(ENTRY).to_string_lossy().to_string(), "0".to_string()]],
        "the program it names, and nothing appended{}",
        report(&seen)
    );
}

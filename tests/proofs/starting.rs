//! What a rig tells the run about the process it is about to start: the
//! environment it is given, and the command line actually run.

use std::ffi::OsString;
use std::path::PathBuf;

use mb_resolver::bash::rig::{run, ExitStatus, Failure, Line, Rig, Startup};

use crate::support::{bash, Scripts};
use crate::{behind, report, ENTRY};

const WRAPPER: &str = "wrapper.bash";

/// Puts a launcher in front of the command line and hands the subject a
/// variable of its own.
struct Launching {
    wrapper: PathBuf,
}

impl Rig for Launching {
    type Session = Vec<Line>;

    fn startup(&self) -> Startup {
        Startup {
            bash: "TELL() { BC_INSTR say TELL \"$@\"; }".to_string(),
            env: vec![("DEPLOY_TARGET".into(), "staging".into())],
        }
    }

    /// The caller asked for `bash main.bash`; what runs is the wrapper, with
    /// that command line behind it as its own arguments.
    fn transform_command(&self, argv: Vec<OsString>) -> Vec<OsString> {
        let mut wrapped = bash(self.wrapper.clone());
        wrapped.extend(argv);

        wrapped
    }

    fn open(&self) -> Result<Vec<Line>, Failure> {
        Ok(Vec::new())
    }

    fn hear(&self, heard: &mut Vec<Line>, said: Line) -> Result<(), Failure> {
        heard.push(said);

        Ok(())
    }
}

/// The launcher runs, the command line it was given arrives intact, and the
/// environment reaches the subject the launcher started rather than only the
/// process the run spawned.
#[test]
fn a_rig_may_wrap_the_command_line_and_add_to_the_environment() {
    let scripts = Scripts::of(&[
        (WRAPPER, "TELL wrapper \"$#\" \"${@: -1}\"\n\"$@\"\n"),
        (ENTRY, "TELL subject \"$DEPLOY_TARGET\" \"$BASH_SOURCE\"\n"),
    ]);

    let launching = Launching { wrapper: scripts.at(WRAPPER) };
    let (seen, status) =
        run(&launching, &bash(scripts.at(ENTRY))).unwrap().whole().unwrap();

    assert_eq!(status, ExitStatus::Code(0), "{}", report(&seen));

    let told = behind(&seen, "TELL");
    let said = |lead: &str| {
        told.iter().find(|words| words[0] == lead).unwrap_or_else(|| panic!("no {lead} message"))
    };

    let wrapper = said("wrapper");
    assert_eq!(wrapper[1], "2", "the caller's whole command line, as two words");
    assert_eq!(wrapper[2], scripts.at(ENTRY).to_string_lossy(), "and unaltered");

    let subject = said("subject");
    assert_eq!(subject[1], "staging", "the rig's variable reached the wrapped command");
    assert_eq!(subject[2], scripts.at(ENTRY).to_string_lossy(), "which is the one asked for");
}

/// The default is identity, and a run that transforms nothing starts exactly
/// what it was handed.
#[test]
fn the_command_line_is_run_as_asked_by_default() {
    let scripts = Scripts::of(&[(ENTRY, "BC_INSTR say REC \"$0\" \"$#\"")]);
    let (seen, status) =
        run(&crate::Keeping, &bash(scripts.at(ENTRY))).unwrap().whole().unwrap();

    assert_eq!(status, ExitStatus::Code(0));
    assert_eq!(
        behind(&seen, "REC"),
        [[scripts.at(ENTRY).to_string_lossy().to_string(), "0".to_string()]],
        "no launcher, no extra arguments{}",
        report(&seen)
    );
}

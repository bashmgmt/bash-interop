//! The run owns its subject: the workspace it lays down, and the process
//! group it takes with it however it ends.

use std::time::{Duration, Instant};

use mb_resolver::bash::rig::{Answer, ExitStatus, Failure, Kind, Line, Master, Rig};

use crate::support::{bash, Scripts};
use crate::{behind, gone, report, script, Keeping, ENTRY};

/// A workspace the rig named is left where it was told to, so what the session
/// laid down is there to read afterwards.
#[test]
fn a_named_workspace_is_left_behind() {
    let temp = tempfile::tempdir().unwrap();
    let at = temp.path().join("under").join("here");
    let scripts = Scripts::of(&[(ENTRY, "BC_INSTR say REC one")]);

    let (seen, status) = Keeping::at(&at).run(&bash(scripts.at(ENTRY))).unwrap().whole().unwrap();

    assert_eq!(status, ExitStatus::Code(0));
    assert_eq!(behind(&seen, "REC"), [["one"]]);

    assert!(at.join("prelude.bash").is_file(), "the protocol's bash");
    assert!(at.join("rig.bash").is_file(), "the rig's own, beside it");
    assert!(at.join("up").exists(), "the pipe every shell joined");
}

/// A reply pipe belongs to one question and goes with its answer, so a run
/// that asks many times from several shells ends holding none of them — and
/// the name a later question uses is one nothing else can be sitting on.
#[test]
fn every_reply_pipe_goes_with_its_answer() {
    let temp = tempfile::tempdir().unwrap();
    let at = temp.path().join("workspace");
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            for i in $(seq 1 40); do BC_INSTR ask step "$i"; done
            bash "${BASH_SOURCE[0]%/*}/other.bash"
            exit 0
            "#,
        ),
        (
            "other.bash",
            r#"
            for i in 1 2 3; do BC_INSTR ask step "$i"; done
            exit 0
            "#,
        ),
    ]);

    let (seen, status) = Keeping::at(&at).run(&bash(scripts.at(ENTRY))).unwrap().whole().unwrap();

    assert_eq!(status, ExitStatus::Code(0));
    assert_eq!(seen.iter().filter(|line| line.kind == Kind::Ask).count(), 43, "two shells asked");

    let left: Vec<String> = std::fs::read_dir(&at)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("rep."))
        .collect();

    assert!(left.is_empty(), "reply pipes left behind: {left:?}");
}

/// A workspace belongs to one run. The pipe every shell joins is made there,
/// and making it is what claims the directory — so whatever a run leaves
/// behind, a later one never meets it.
#[test]
fn a_workspace_belongs_to_one_run() {
    let temp = tempfile::tempdir().unwrap();
    let at = temp.path().join("workspace");
    let scripts = Scripts::of(&[(ENTRY, "BC_INSTR say REC one")]);

    let first = Keeping::at(&at).run(&bash(scripts.at(ENTRY))).unwrap().whole().unwrap();
    assert_eq!(first.1, ExitStatus::Code(0));

    let again = Keeping::at(&at).run(&bash(scripts.at(ENTRY)))
        .err()
        .expect("a second run must not reuse it");

    assert!(again.to_string().contains("EEXIST"), "{again}");
}

/// A shell asking after the subject exited can never be answered, so the run
/// takes the whole process group with it.
#[test]
fn a_shell_left_asking_does_not_outlive_the_run() {
    let started = Instant::now();
    let (seen, _) = script(
        r#"
        bash -c 'BC_INSTR say REC lingering $BASHPID; sleep 30; BC_INSTR ask never' &
        sleep 0.2
        exit 0
        "#,
    );
    assert!(started.elapsed() < Duration::from_secs(5), "the run must not wait for a straggler");

    let lingering: i32 = behind(&seen, "REC")
        .iter()
        .find_map(|words| match words {
            [first, pid] if first == "lingering" => pid.parse().ok(),
            _ => None,
        })
        .expect("the straggler reported itself");

    assert!(gone(lingering), "{lingering} outlived the run\n{}", report(&seen));
}

/// Blows up instead of answering. Its session is nothing at all, which is a
/// session type like any other.
struct Exploding;

impl Rig for Exploding {
    type Session = ();

    fn open(&self) -> Result<(), Failure> {
        Ok(())
    }

    fn answer(&self, _: &mut (), _: Line) -> Result<Answer, Failure> {
        panic!("answer blew up")
    }
}

impl Master for Exploding {}

/// The panic propagates and the subject is still killed. Nothing comes back
/// from the run, so the subject writes its own pid to a file before asking.
#[test]
fn a_panicking_answer_kills_the_subject() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        echo $BASHPID > "${BASH_SOURCE[0]%/*}/pid"
        BC_INSTR ask anything
        "#,
    )]);
    let argv = bash(scripts.at(ENTRY));

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Exploding.run(&argv)));
    std::panic::set_hook(previous);

    assert!(outcome.is_err(), "the panic must propagate rather than be swallowed");

    let blocked: i32 = std::fs::read_to_string(scripts.at("pid"))
        .expect("the subject reported its pid before asking")
        .trim()
        .parse()
        .unwrap();
    assert!(gone(blocked), "{blocked} was left waiting for an answer that will never come");
}

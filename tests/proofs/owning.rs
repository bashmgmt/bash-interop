//! The run owns its subject: the workspace it lays down, and the process
//! group it takes with it however it ends.

use std::time::{Duration, Instant};

use mb_resolver::bash::rig::{run, run_in, Answer, ExitStatus, Failure, Line, Rig};

use crate::support::{bash, Scripts};
use crate::{behind, report, script, Keeping, ENTRY};

/// `run_in` leaves its workspace where it was told to, so what the run set up
/// is there to read afterwards.
#[test]
fn a_named_workspace_is_left_behind() {
    let temp = tempfile::tempdir().unwrap();
    let at = temp.path().join("under").join("here");
    let scripts = Scripts::of(&[(ENTRY, "BC_INSTR say REC one")]);

    let (seen, status) = run_in(&Keeping, &at, &bash(scripts.at(ENTRY))).unwrap().whole().unwrap();

    assert_eq!(status, ExitStatus::Code(0));
    assert_eq!(behind(&seen, "REC"), [["one"]]);

    assert!(at.join("prelude.bash").is_file(), "the protocol's bash");
    assert!(at.join("rig.bash").is_file(), "the rig's own, beside it");
    assert!(at.join("up").exists(), "the pipe every shell joined");
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

/// The panic propagates and the subject is still killed. Nothing comes back
/// from the run, so the subject writes its own pid to a file before asking.
#[test]
fn a_panicking_answer_kills_the_subject() {
    let scripts = Scripts::of(&[(
        ENTRY,
        "echo $BASHPID > \"${BASH_SOURCE[0]%/*}/pid\"\nBC_INSTR ask anything\n",
    )]);
    let argv = bash(scripts.at(ENTRY));

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&Exploding, &argv)));
    std::panic::set_hook(previous);

    assert!(outcome.is_err(), "the panic must propagate rather than be swallowed");

    let blocked: i32 = std::fs::read_to_string(scripts.at("pid"))
        .expect("the subject reported its pid before asking")
        .trim()
        .parse()
        .unwrap();
    assert!(gone(blocked), "{blocked} was left waiting for an answer that will never come");
}

/// The kill is immediate; the reaping is init's and takes a moment.
fn gone(pid: i32) -> bool {
    for _ in 0..100 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

//! The run owns its subject: the workspace it lays down, and the process
//! group it takes with it however it ends. Nothing outside that group is
//! signalled.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bash_interop::rig::{
    Answer, Driving, Failure, Layout, Message, Provision, Reacting, Rig, Shell, Verb,
};

use bash_interop::scratch::{bash, Scripts};
use crate::{behind, gone, lines, provisioned, report, script, Keeping, ENTRY};

/// A workspace the caller named and made is left where it was told to,
/// holding the session's three bash files, the lock file, and none of the
/// fifos: the control fifo goes when the session closes, a shell's two when
/// it parts, and even the fifo of an announcement that never finished —
/// staged here as the protocol would have left it — is removed, since its
/// token names it.
#[tokio::test]
async fn a_named_workspace_is_left_behind_without_its_fifos() {
    let temp = tempfile::tempdir().unwrap();
    let at = temp.path().join("under").join("here");
    std::fs::create_dir_all(&at).unwrap();
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            for i in 1 2 3; do BC_INSTR KEEP ask step "$i"; done
            bash "${BASH_SOURCE[0]%/*}/other.bash"
            ( BC_INSTR KEEP say REC fork )
            declare -- workspace="${DEPLOY_SESSION:?the workspace, from the run closure}"
            mkfifo "$workspace/up.GHOST"
            printf 'GHOST + half\n' >"$workspace/join"
            exit 0
            "#,
        ),
        (
            "other.bash",
            r#"
            for i in 1 2 3; do BC_INSTR KEEP ask step "$i"; done
            exit 0
            "#,
        ),
    ]);

    let ran = Keeping
        .run_at(&at, &bash(scripts.at(ENTRY)), |at| {
            Ok(vec![
                crate::deploy_session(at),
                at.bash_env(Provision::Joining(&crate::join(at)))?,
            ])
        })
        .await
        .unwrap()
        .whole()
        .unwrap();

    assert_eq!(behind(&ran.shells, "REC"), [["fork"]]);
    assert_eq!(
        lines(&ran.shells).iter().filter(|message| message.verb == Verb::Ask).count(),
        6,
        "both shells asked, so both reply fifos existed"
    );

    let mut left: Vec<String> = std::fs::read_dir(&at)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        ["bash_env.bash", "lock", "prelude.bash", "rig.bash"],
        "the bash, and nothing that was a pipe"
    );
}

/// A shell asking after the subject exited can never be answered, so the run
/// takes the whole process group with it.
#[tokio::test]
async fn a_shell_left_asking_does_not_outlive_the_run() {
    let started = Instant::now();
    let ran = script(
        r#"
        bash -c 'BC_INSTR KEEP say REC lingering $BASHPID; sleep 30; BC_INSTR KEEP ask never' &
        sleep 0.2
        exit 0
        "#,
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(5), "the run must not wait for a straggler");

    let lingering: i32 = behind(&ran.shells, "REC")
        .iter()
        .find_map(|words| match words {
            [first, pid] if first == "lingering" => pid.parse().ok(),
            _ => None,
        })
        .expect("the straggler reported itself");

    assert!(gone(lingering), "{lingering} outlived the run\n{}", report(&ran.shells));
}

/// A shell that joined from outside the run's process group is heard for as
/// long as the run lasts, and never signalled: it is not the run's to end.
#[tokio::test]
async fn a_shell_outside_the_group_is_heard_and_never_signalled() {
    let ran = script(
        r#"
        setsid bash -c 'BC_INSTR KEEP say REC outsider $BASHPID; sleep 30' &
        sleep 0.3
        BC_INSTR KEEP say REC done
        "#,
    )
    .await;

    let said = behind(&ran.shells, "REC");
    let outsider: i32 = said
        .iter()
        .find_map(|words| match words {
            [first, pid] if first == "outsider" => pid.parse().ok(),
            _ => None,
        })
        .expect("the outsider was heard");
    assert_eq!(said.len(), 2, "{}", report(&ran.shells));

    let alive = unsafe { libc::kill(outsider, 0) } == 0;
    let _ = unsafe { libc::kill(outsider, libc::SIGKILL) };
    assert!(alive, "{outsider} was signalled by a run that did not start it");

    let its = ran.shells.iter().find(|at| at.shell.pid.0 == outsider as u32).expect("its shell");
    assert!(its.parted.is_none(), "the session outlived it, and says so");
}

/// Blows up instead of answering, and keeps nothing.
struct Exploding;

struct Boom;

impl Rig for Exploding {
    type Reaction = Boom;

    fn bash(&self, _at: &Layout) -> String {
        String::new()
    }

    fn joining(&self, at: &Layout) -> String {
        crate::join(at)
    }

    async fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Boom, Failure> {
        Ok(Boom)
    }
}

impl Reacting for Boom {
    type Kept = ();

    async fn hear(&mut self, _said: Message) -> Result<(), Failure> {
        Ok(())
    }

    async fn answer(&mut self, _: Message) -> Result<Answer, Failure> {
        panic!("answer blew up")
    }

    async fn finish(self) -> Result<(), Failure> {
        Ok(())
    }
}

impl Driving for Exploding {}

/// The panic propagates out of the run and the subject is still killed.
/// Nothing comes back from the run, so the subject writes its own pid to a
/// file before asking.
#[test]
fn a_panicking_answer_kills_the_subject() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        echo $BASHPID > "${BASH_SOURCE[0]%/*}/pid"
        BC_INSTR KEEP ask anything
        "#,
    )]);
    let argv = bash(scripts.at(ENTRY));
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().build().unwrap();

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(Exploding.run(&argv, provisioned(&Exploding)))
    }));
    std::panic::set_hook(previous);

    assert!(outcome.is_err(), "the panic must propagate rather than be swallowed");

    let blocked: i32 = std::fs::read_to_string(scripts.at("pid"))
        .expect("the subject reported its pid before asking")
        .trim()
        .parse()
        .unwrap();
    assert!(gone(blocked), "{blocked} was left waiting for an answer that will never come");
}

//! A session a bash script joins for its own purposes.
//!
//! Nothing here starts the shell and nothing here ends it. The session lasts
//! as long as someone holds its handle, which is the same rule a driven run
//! applies to its process group — with the difference that this side owns
//! nothing it could kill.

use std::io::{pipe, Write};
use std::os::fd::OwnedFd;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use mb_resolver::bash::rig::{
    Attended, Failure, Layout, Message, Rig, Serving, Shell,
};

use crate::support::Scripts;
use crate::{behind, report, ENTRY};

/// Keeps what it hears, and has no say in when the session ends.
struct Attaching;

impl Rig for Attaching {
    type Reaction = Vec<Message>;

    fn bash(&self) -> String {
        "TELL() { BC_INSTR TELL say TELL \"$@\"; }\nBC_JOIN TELL \"$1\"\n".to_string()
    }

    async fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Vec<Message>, Failure> {
        Ok(Vec::new())
    }
}

impl Serving for Attaching {}

/// The client's side of joining, as `BC_START` does it: the address into
/// `BC_SESSION`, exported for the script's children, sourced; then on with its
/// own script.
const JOINING: &str = r#"export BC_SESSION="$1"; source "$BC_SESSION"; source "$2""#;

/// A shell of the initiator's own, holding the session's handle on its
/// standard output — what a client holds when it started the server as a
/// coprocess. It hangs up when the last shell holding it is gone, or when the
/// client closes it deliberately.
fn joining(address: &str, script: &Path, handle: OwnedFd) -> Child {
    Command::new("bash")
        .args(["-c", JOINING, "--"])
        .arg(address)
        .arg(script)
        .stdout(Stdio::from(handle))
        .spawn()
        .expect("a shell to join with")
}

/// Serve `scripts`' entry in a shell started for it — the workspace is the
/// initiator's to name, so this side makes one, and the address is the file
/// under it this side could already spell — and hand back the shells that
/// joined beside how that shell ended.
async fn joined(scripts: &Scripts) -> (Vec<Attended<Vec<Message>>>, std::process::ExitStatus) {
    let workspace = tempfile::tempdir().expect("a workspace to prescribe");
    let (held, handle) = pipe().expect("a handle");

    let mut child = None;
    let served = Attaching
        .serve(workspace.path(), held.into(), |address| {
            let expected = workspace.path().canonicalize().unwrap().join("session.bash");
            assert_eq!(Path::new(address), expected, "the prescribed dir, as prescribed");

            child = Some(joining(address, &scripts.at(ENTRY), handle.into()));
            Ok(())
        })
        .await
        .expect("the session");

    assert!(served.failed.is_none(), "the session closed up cleanly");

    let status = child.expect("the shell").wait().expect("reaping the shell");

    (served.shells, status)
}

/// Everything the joined shell says arrives, subshells included, and the
/// session lasts exactly as long as the handle does. Nothing of that shell's
/// life is the session's: it is neither started nor stopped here, and its
/// status is the initiator's to collect.
#[tokio::test]
async fn a_shell_that_joined_is_heard_until_it_lets_go() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        TELL first
        ( TELL from-a-subshell )
        TELL last
        exit 3
        "#,
    )]);

    let (shells, status) = joined(&scripts).await;

    assert_eq!(
        behind(&shells, "TELL"),
        [["first"].as_slice(), ["from-a-subshell"].as_slice(), ["last"].as_slice()],
        "{}",
        report(&shells)
    );
    assert_eq!(status.code(), Some(3), "the initiator's own status, which is not ours to hold");
    assert!(shells[1].parted.is_some(), "the subshell parted long before the handle went");
}

/// A client that lets go while still running is a shell the session outlived,
/// and says so. Its next word finds nobody at the other end of its pipe.
#[tokio::test]
async fn a_shell_the_session_outlived_is_left_to_its_own_devices() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        TELL before
        exec 1>&-
        sleep 0.3
        TELL after
        echo unreachable >&2
        "#,
    )]);

    let (shells, status) = joined(&scripts).await;

    assert_eq!(behind(&shells, "TELL"), [["before"]], "{}", report(&shells));
    assert!(shells[0].parted.is_none(), "still running when the handle went");
    assert_eq!(status.signal(), Some(libc::SIGPIPE), "the word after the session took SIGPIPE");
}

/// How far a joined session reaches is the client's decision. Exporting
/// `BASH_ENV` to the address puts the session in every process the script
/// starts, which is what a driven run does for the tree it creates.
#[tokio::test]
async fn a_joined_shell_may_publish_the_address_to_its_children() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            TELL parent "$BASHPID"
            export BASH_ENV="$BC_SESSION"
            bash "${BASH_SOURCE[0]%/*}/child.bash"
            "#,
        ),
        ("child.bash", "TELL child \"$BASHPID\"\n"),
    ]);

    let (shells, status) = joined(&scripts).await;
    let said = behind(&shells, "TELL");

    assert_eq!(status.code(), Some(0), "{}", report(&shells));
    assert_eq!(said.len(), 2, "the script and the bash it started: {}", report(&shells));
    assert_eq!(said[0][0], "parent");
    assert_eq!(said[1][0], "child", "{}", report(&shells));
    assert_ne!(said[0][1], said[1][1], "a process of its own");
}

/// A shell nothing started, joining because it wants to.
///
/// Interactive is the case that can only happen this way: bash reads
/// `BASH_ENV` for non-interactive shells alone, so an interactive one can join
/// a session only by sourcing the address itself. Its code arrives on standard
/// input, and standard output is the handle it holds.
fn interactively(address: &str, handle: OwnedFd) -> Child {
    let mut shell = Command::new("bash")
        .args(["--norc", "--noprofile", "-i"])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(handle))
        .stderr(Stdio::null())
        .spawn()
        .expect("an interactive shell");

    let typed = format!("source \"{address}\"\nTELL at-the-prompt\n");
    shell.stdin.take().expect("its input").write_all(typed.as_bytes()).expect("typing at it");

    shell
}

/// What a shell is is what it said, and nothing is read off the shape of what
/// it went on to say.
///
/// An interactive shell writes `main` into `BASH_SOURCE` for whatever is
/// defined at its prompt and pushes no frame for the prompt itself — both of
/// which a script can also produce, and neither of which says *interactive*.
/// The shell says so itself, once, when it joins.
#[tokio::test]
async fn a_shell_says_what_it_is_rather_than_being_guessed_at() {
    let workspace = tempfile::tempdir().expect("a workspace to prescribe");
    let (held, handle) = pipe().expect("a handle");

    let mut child = None;
    let served = Attaching
        .serve(workspace.path(), held.into(), |address| {
            child = Some(interactively(address, handle.into()));
            Ok(())
        })
        .await
        .expect("the session");

    child.expect("the shell").wait().expect("reaping the shell");
    assert!(served.failed.is_none(), "the session closed up cleanly");

    assert_eq!(behind(&served.shells, "TELL"), [["at-the-prompt"]], "{}", report(&served.shells));
    assert_eq!(served.shells.len(), 1);

    let shell = &served.shells[0].shell;
    let started = &shell.bash.invocation;

    assert!(started.interactive, "it said so: {started:?}");
    assert!(started.standard_input, "and where its code came from");
    assert!(started.command.is_none(), "which was not a command line");
    assert!(shell.bash.version.at_least(5, 0, 0), "$EPOCHREALTIME is bash 5");
    assert_eq!(shell.subshell, 0);

    // Interactive is not something the options can be turned into: `set`
    // refuses `-i`, so this is settled at startup and true of the whole shell.
    assert!(shell.options.flags.has('i'));
}

/// A `Failure` before anything was served — the announcement itself — still
/// sees the session out: the control fifo is unlinked, the three bash files
/// remain, and the error is the announcer's own.
#[tokio::test]
async fn a_failed_announcement_still_sees_the_session_out() {
    let temp = tempfile::tempdir().unwrap();
    let at = temp.path().join("here");
    let (held, _handle) = pipe().expect("a handle");

    let refused = Attaching
        .serve(&at, held.into(), |_| Err(Failure::new("announcing", "the client is gone")))
        .await
        .err()
        .expect("the announcer's failure must come back");
    assert!(refused.to_string().contains("the client is gone"), "{refused}");

    let mut left: Vec<String> = std::fs::read_dir(&at)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, ["prelude.bash", "rig.bash", "session.bash"], "no fifo left behind");
}

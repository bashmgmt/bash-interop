//! A session a bash script joins for its own purposes.
//!
//! Nothing here starts the shell and nothing here ends it. The session lasts
//! as long as someone holds its handle, which is the same rule a driven run
//! applies to its process group — with the difference that this side owns
//! nothing it could kill.

use std::io::{Write, pipe};
use std::os::fd::OwnedFd;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use bash_interop::rig::{Attended, Failure, Layout, Message, Rig, Serving, Shell};

use crate::{ENTRY, behind, report};
use bash_interop::scratch::Scripts;

/// Keeps what it hears, and has no say in when the session ends.
struct Attaching;

impl Rig for Attaching {
    type Reaction = Vec<Message>;

    fn bash(&self, _at: &Layout) -> String {
        crate::saying("TELL", "TELL")
    }

    async fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Vec<Message>, Failure> {
        Ok(Vec::new())
    }
}

impl Serving for Attaching {}

/// The client's side: probe the directory it named until the session is
/// up, load the laid definitions, initiate its
/// own channel, then on with its own script — which is handed the same
/// coordinate as its `$1`, explicitly, the way everything else receives it.
const JOINING: &str = r#"
    declare -- workspace="${1:?the session workspace}"
    declare -- entry="${2:?the client script}"

    until [[ -p "$workspace/join" ]]; do sleep 0.01; done
    source "$workspace/prelude.bash"
    source "$workspace/rig.bash"
    BC_JOIN TELL "$workspace"
    source "$entry" "$workspace"
"#;

/// A shell of the initiator's own, holding the session's handle on its
/// standard output — what a client holds when it started the server as a
/// coprocess. It hangs up when the last shell holding it is gone, or when the
/// client closes it deliberately. It is started before the server: nothing is
/// read back, so the probe is what says the session is up.
fn joining(dir: &Path, script: &Path, handle: OwnedFd) -> Child {
    Command::new("bash")
        .args(["-c", JOINING, "--"])
        .arg(dir)
        .arg(script)
        .stdout(Stdio::from(handle))
        .spawn()
        .expect("a shell to join with")
}

/// Serve `scripts`' entry in a shell started for it — the workspace is the
/// initiator's to name and make, and the same directory is all the client
/// holds — and hand back the shells that joined beside how that shell ended.
/// The join fifo brackets the session: absent before, present exactly while
/// it serves, gone when it is over.
async fn joined(
    scripts: &Scripts,
) -> (
    Vec<Attended<Vec<Message>>>,
    std::process::ExitStatus,
) {
    let workspace = tempfile::tempdir().expect("a workspace to prescribe");
    let (held, handle) = pipe().expect("a handle");
    assert!(
        !workspace.path().join("join").exists(),
        "nothing serves yet"
    );

    let mut child = joining(
        workspace.path(),
        &scripts.at(ENTRY),
        handle.into(),
    );
    let served = Attaching
        .serve(workspace.path(), held.into())
        .await
        .expect("the session");

    assert!(
        served.failed.is_none(),
        "the session closed up cleanly"
    );
    assert!(
        !workspace.path().join("join").exists(),
        "the liveness signal went with it"
    );

    let status = child.wait().expect("reaping the shell");

    (served.shells, status)
}

/// Everything the joined shell says arrives, subshells included, and the
/// session lasts exactly as long as the handle does. The session does not
/// manage that shell's life: it is neither started nor stopped here, and
/// whoever started it collects its status.
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
        [
            ["first"].as_slice(),
            ["from-a-subshell"].as_slice(),
            ["last"].as_slice()
        ],
        "{}",
        report(&shells)
    );
    assert_eq!(
        status.code(),
        Some(3),
        "the initiator's own status, which is not ours to hold"
    );
    assert!(
        shells[1].parted.is_some(),
        "the subshell parted long before the handle went"
    );
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

    assert_eq!(
        behind(&shells, "TELL"),
        [["before"]],
        "{}",
        report(&shells)
    );
    assert!(
        shells[0].parted.is_none(),
        "still running when the handle went"
    );
    assert_eq!(
        status.signal(),
        Some(libc::SIGPIPE),
        "the word after the session took SIGPIPE"
    );
}

/// How far a session reaches is the client's decision — the startup file
/// included. Nothing laid in the workspace initiates, so a client that
/// wants its child processes joined writes its own `BASH_ENV` file — the
/// two sources and its initiation, `%q`-spelled, in its own directory —
/// and bash does the rest at every child's startup.
#[tokio::test]
async fn a_joined_shell_may_publish_to_its_children() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            declare -- workspace="${1:?the session workspace}"
            TELL parent "$BASHPID"

            declare -- own="${BASH_SOURCE[0]%/*}/own.bash"
            printf 'source %q\nsource %q\nBC_JOIN TELL %q\n' \
                "$workspace/prelude.bash" "$workspace/rig.bash" "$workspace" > "$own"
            export BASH_ENV="$own"
            bash "${BASH_SOURCE[0]%/*}/child.bash"
            "#,
        ),
        (
            "child.bash",
            r#"
            TELL child "$BASHPID"
            "#,
        ),
    ]);

    let (shells, status) = joined(&scripts).await;
    let said = behind(&shells, "TELL");

    assert_eq!(
        status.code(),
        Some(0),
        "{}",
        report(&shells)
    );
    assert_eq!(
        said.len(),
        2,
        "the script and the bash it started: {}",
        report(&shells)
    );
    assert_eq!(said[0][0], "parent");
    assert_eq!(
        said[1][0],
        "child",
        "{}",
        report(&shells)
    );
    assert_ne!(
        said[0][1], said[1][1],
        "a process of its own"
    );
}

/// The coordinate travels only as an argument, and initiation is the
/// child's own: handed the workspace on its command line, it loads the
/// definitions and says the join itself. It names the `BASH_ENV` it does
/// not have.
#[tokio::test]
async fn a_child_may_be_told_the_workspace_as_an_argument() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            declare -- workspace="${1:?the session workspace}"
            TELL parent "$BASHPID"
            bash "${BASH_SOURCE[0]%/*}/child.bash" "$workspace"
            "#,
        ),
        (
            "child.bash",
            r#"
            declare -- workspace="${1:?the session workspace}"
            source "$workspace/prelude.bash"
            source "$workspace/rig.bash"
            BC_JOIN TELL "$workspace"
            TELL child "$BASHPID" "${BASH_ENV-unset}"
            "#,
        ),
    ]);

    let (shells, status) = joined(&scripts).await;
    let said = behind(&shells, "TELL");

    assert_eq!(
        status.code(),
        Some(0),
        "{}",
        report(&shells)
    );
    assert_eq!(said.len(), 2, "{}", report(&shells));
    assert_eq!(
        said[1][0],
        "child",
        "{}",
        report(&shells)
    );
    assert_eq!(
        said[1][2], "unset",
        "no environment carried anything"
    );
    assert_ne!(
        said[0][1], said[1][1],
        "a process of its own"
    );
}

/// A shell nothing started, joining because it wants to.
///
/// Interactive is the case that can only happen this way: bash reads
/// `BASH_ENV` for non-interactive shells alone, so an interactive one can join
/// a session only by sourcing the address itself. Its code arrives on standard
/// input, and standard output is the handle it holds.
fn interactively(dir: &Path, handle: OwnedFd) -> Child {
    let mut shell = Command::new("bash")
        .args(["--norc", "--noprofile", "-i"])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(handle))
        .stderr(Stdio::null())
        .spawn()
        .expect("an interactive shell");

    let dir = dir.display();
    let typed = format!(
        r#"
        until [[ -p "{dir}/join" ]]; do sleep 0.01; done
        source "{dir}/prelude.bash"
        source "{dir}/rig.bash"
        BC_JOIN TELL "{dir}"
        TELL at-the-prompt
        "#
    );
    shell
        .stdin
        .take()
        .expect("its input")
        .write_all(typed.as_bytes())
        .expect("typing at it");

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

    let mut child = interactively(workspace.path(), handle.into());
    let served = Attaching
        .serve(workspace.path(), held.into())
        .await
        .expect("the session");

    child.wait().expect("reaping the shell");
    assert!(
        served.failed.is_none(),
        "the session closed up cleanly"
    );

    assert_eq!(
        behind(&served.shells, "TELL"),
        [["at-the-prompt"]],
        "{}",
        report(&served.shells)
    );
    assert_eq!(served.shells.len(), 1);

    let shell = &served.shells[0].shell;
    let started = &shell.bash.invocation;

    assert!(
        started.interactive,
        "it said so: {started:?}"
    );
    assert!(
        started.standard_input,
        "and where its code came from"
    );
    assert!(
        started.command.is_none(),
        "which was not a command line"
    );
    assert!(
        shell.bash.version.at_least(5, 0, 0),
        "$EPOCHREALTIME is bash 5"
    );
    assert_eq!(shell.subshell, 0);

    // Interactive is not something the options can be turned into: `set`
    // refuses `-i`, so this is settled at startup and true of the whole shell.
    assert!(shell.options.flags.has('i'));
}

/// A workspace is one session's at a time: the lock is taken before
/// anything in it is touched, so a second server on the same directory is
/// refused whole — no files rewritten, no fifo disturbed — while the first
/// serves on.
#[tokio::test]
async fn an_occupied_workspace_is_refused() {
    let workspace = tempfile::tempdir().unwrap();
    let (held, handle) = pipe().expect("a handle");
    let (second_held, _second_handle) = pipe().expect("a second handle");

    let first = Attaching.serve(workspace.path(), held.into());
    let second = async {
        while !workspace.path().join("join").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let refused = Attaching
            .serve(workspace.path(), second_held.into())
            .await
            .err()
            .expect("the second server must be refused");
        assert!(
            refused
                .to_string()
                .contains("already held by a live session"),
            "{refused}"
        );
        assert!(
            workspace.path().join("join").exists(),
            "the first session is undisturbed"
        );

        drop(handle);
    };

    let (served, ()) = tokio::join!(first, second);
    assert!(served.expect("the first session").failed.is_none());
}

/// A predecessor that could not clean up — killed outright — leaves its
/// fifos behind. The next session on that directory owns it the moment it
/// holds the lock, sweeps them, and serves; when it is over, nothing of
/// either is left but the bash files and the lock.
#[tokio::test]
async fn a_killed_predecessors_leavings_are_swept() {
    let temp = tempfile::tempdir().unwrap();
    let at = temp.path();
    for stale in ["join", "up.GHOST", "rep.GHOST"] {
        nix::unistd::mkfifo(
            &at.join(stale),
            nix::sys::stat::Mode::S_IRWXU,
        )
        .unwrap();
    }

    let scripts = Scripts::of(&[(ENTRY, "TELL revived\n")]);
    let (held, handle) = pipe().expect("a handle");
    let mut child = joining(at, &scripts.at(ENTRY), handle.into());
    let served = Attaching.serve(at, held.into()).await.expect("the session");
    child.wait().expect("reaping the shell");

    assert_eq!(
        behind(&served.shells, "TELL"),
        [["revived"]],
        "{}",
        report(&served.shells)
    );

    let mut left: Vec<String> = std::fs::read_dir(at)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        ["lock", "prelude.bash", "rig.bash"],
        "swept and closed"
    );
}

/// A workspace nobody made is nobody's to invent: the prescribed directory
/// must exist, and a missing one is a refusal that touches nothing.
#[tokio::test]
async fn a_missing_workspace_is_a_refusal() {
    let temp = tempfile::tempdir().unwrap();
    let at = temp.path().join("never-made");
    let (held, _handle) = pipe().expect("a handle");

    let refused = Attaching
        .serve(&at, held.into())
        .await
        .err()
        .expect("a missing workspace must be refused");
    assert!(
        refused
            .to_string()
            .contains("opening the prescribed workspace"),
        "{refused}"
    );
    assert!(!at.exists(), "and it was not invented");
}

//! A session a bash script joins for its own purposes.
//!
//! Nothing here starts the shell and nothing here ends it. The session lasts
//! as long as someone holds its handle, which is the same rule a driven run
//! applies to its process group — with the difference that this side owns
//! nothing it could kill.

use std::io::pipe;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use mb_resolver::bash::rig::{Answer, Failure, Line, Rig, Slave};

use crate::support::Scripts;
use crate::{behind, report, ENTRY};

/// Keeps what it hears, and has no say in when the session ends.
struct Attaching;

impl Rig for Attaching {
    type Session = Vec<Line>;

    fn bash(&self) -> String {
        "TELL() { BC_INSTR say TELL \"$@\"; }".to_string()
    }

    fn open(&self) -> Result<Vec<Line>, Failure> {
        Ok(Vec::new())
    }

    fn hear(&self, heard: &mut Vec<Line>, said: Line) -> Result<(), Failure> {
        heard.push(said);

        Ok(())
    }
}

impl Slave for Attaching {}

/// The client's side of joining: run the one command it was handed, then carry
/// on with its own script. `declare -a` reads the address exactly as the
/// prelude reads an answer, because it is the same kind of thing.
const JOINING: &str = r#"declare -a __join="$1"; "${__join[@]}"; source "$2""#;

/// A shell of the initiator's own, holding the session's handle — what a
/// client holds when it started the server as a coprocess. Nothing reads that
/// pipe; it hangs up when the last shell holding it is gone, which is what a
/// client does deliberately with `exec {fd}>&-`.
fn joining(address: &Answer, script: &Path, handle: OwnedFd) -> Child {
    Command::new("bash")
        .args(["-c", JOINING, "--"])
        .arg(address.to_string())
        .arg(script)
        .stdout(Stdio::from(handle))
        .spawn()
        .expect("a shell to join with")
}

/// Serve `scripts`' entry in a shell started for it, and hand back what the
/// session heard beside how that shell ended.
fn joined(scripts: &Scripts) -> (Vec<Line>, Option<i32>) {
    let (held, handle) = pipe().expect("a handle");

    let mut child = None;
    let served = Attaching
        .serve(held.into(), |address| {
            child = Some(joining(address, &scripts.at(ENTRY), handle.into()));
            Ok(())
        })
        .expect("the session");

    assert!(served.failed.is_none(), "the session closed up cleanly");

    let status = child.expect("the shell").wait().expect("reaping the shell");

    (served.session, status.code())
}

/// Everything the joined shell says arrives, subshells included, and the
/// session lasts exactly as long as the handle does. Nothing of that shell's
/// life is the session's: it is neither started nor stopped here, and its
/// status is the initiator's to collect.
#[test]
fn a_shell_that_joined_is_heard_until_it_lets_go() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        TELL first
        ( TELL from-a-subshell )
        TELL last
        exit 3
        "#,
    )]);

    let (heard, status) = joined(&scripts);

    assert_eq!(
        behind(&heard, "TELL"),
        [["first"].as_slice(), ["from-a-subshell"].as_slice(), ["last"].as_slice()],
        "{}",
        report(&heard)
    );
    assert_eq!(status, Some(3), "the initiator's own status, which is not ours to hold");
}

/// How far a joined session reaches is the client's decision, and `$__BC__DIR`
/// is where it finds the address again. Exporting `BASH_ENV` to it puts the
/// session in every process the script starts, which is what a driven run does
/// for the tree it creates.
#[test]
fn a_joined_shell_may_publish_the_address_to_its_children() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            TELL parent "$BASHPID"
            export BASH_ENV="$__BC__DIR/prelude.bash"
            bash "${BASH_SOURCE[0]%/*}/child.bash"
            "#,
        ),
        ("child.bash", "TELL child \"$BASHPID\"\n"),
    ]);

    let (heard, status) = joined(&scripts);
    let said = behind(&heard, "TELL");

    assert_eq!(status, Some(0), "{}", report(&heard));
    assert_eq!(said.len(), 2, "the script and the bash it started: {}", report(&heard));
    assert_eq!(said[0][0], "parent");
    assert_eq!(said[1][0], "child", "{}", report(&heard));
    assert_ne!(
        said[0][1], said[1][1],
        "a process of its own, reached because the client published the address"
    );
}

//! A session a bash script joins for its own purposes.
//!
//! Nothing here starts the shell and nothing here ends it: the initiator does
//! both, which is why the only thing the session watches is the handle, and
//! why a client that closes properly closes with a word of its own.

use std::io::pipe;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use mb_resolver::bash::rig::{Answer, Closed, Failure, Halt, Held, Line, Rig, Slave};

use crate::support::Scripts;
use crate::{behind, report, ENTRY};

/// Keeps what it hears, and reads one word of its own as the end of the
/// session. Which word that is, is the client's and the rig's business; the
/// protocol reserves none.
struct Attaching;

impl Rig for Attaching {
    type Session = Vec<Line>;

    fn bash(&self) -> String {
        "TELL() { BC_INSTR say TELL \"$@\"; }".to_string()
    }

    fn open(&self) -> Result<Vec<Line>, Failure> {
        Ok(Vec::new())
    }

    fn hear(&self, heard: &mut Vec<Line>, said: Line) -> Result<(), Halt> {
        if said.behind("TELL").is_some_and(|rest| rest == ["done"]) {
            return Err(Halt::Done);
        }
        heard.push(said);

        Ok(())
    }
}

impl Slave for Attaching {}

/// The client's side of joining: run the one command it was handed, then carry
/// on with its own script. `declare -a` reads the address exactly as the
/// prelude reads an answer, because it is the same kind of thing.
const JOINING: &str = r#"declare -a __join="$1"; "${__join[@]}"; source "$2""#;

/// A shell of the initiator's own, holding the write end of the session's
/// handle — which is what a client holds when it started the server as a
/// coprocess. Nothing reads that pipe; it hangs up when the shell is gone.
fn joining(address: &Answer, script: &Path, handle: OwnedFd) -> Child {
    Command::new("bash")
        .args(["-c", JOINING, "--"])
        .arg(address.to_string())
        .arg(script)
        .stdout(Stdio::from(handle))
        .spawn()
        .expect("a shell to join with")
}

/// Everything the joined shell says arrives, subshells included, and the word
/// the rig reads as the end is what ends it. Nothing of the shell's own life
/// is the session's: it is neither started nor stopped here, and its status is
/// the initiator's to collect.
#[test]
fn a_shell_that_joined_is_heard_and_closes_the_session_itself() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        TELL first
        ( TELL from-a-subshell )
        TELL done
        "#,
    )]);
    let (held, handle) = pipe().expect("a handle");

    let mut child = None;
    let served = Attaching
        .serve(Held::of(held.into()), |address| {
            child = Some(joining(address, &scripts.at(ENTRY), handle.into()));
            Ok(())
        })
        .expect("the session");

    assert_eq!(served.closed, Closed::Said, "the client closed it, not the handle");
    assert_eq!(
        behind(&served.session, "TELL"),
        [["first"].as_slice(), ["from-a-subshell"].as_slice()],
        "{}",
        report(&served.session)
    );

    let status = child.expect("the shell").wait().expect("reaping the shell");
    assert_eq!(status.code(), Some(0), "the initiator's own status, which is not ours to hold");
}

/// A client that leaves without closing takes its handle with it, and that is
/// the only thing left to end the session.
#[test]
fn a_session_whose_initiator_vanished_ends_on_its_handle() {
    let scripts = Scripts::of(&[(ENTRY, "TELL only\nexit 3\n")]);
    let (held, handle) = pipe().expect("a handle");

    let mut child = None;
    let served = Attaching
        .serve(Held::of(held.into()), |address| {
            child = Some(joining(address, &scripts.at(ENTRY), handle.into()));
            Ok(())
        })
        .expect("the session");

    assert_eq!(served.closed, Closed::Gone, "nobody was left to say it");
    assert_eq!(behind(&served.session, "TELL"), [["only"].as_slice()], "{}", report(&served.session));

    let status = child.expect("the shell").wait().expect("reaping the shell");
    assert_eq!(status.code(), Some(3));
}

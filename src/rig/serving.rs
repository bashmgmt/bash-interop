//! bash orchestrates: a script that is already running started the server,
//! takes the address it is handed, and lets go when it is done.
//!
//! Nothing here starts a process or ends one. What the client started, the
//! client cleans up, which is why the only thing this side watches is the
//! handle.

use std::io;
use std::os::fd::{AsFd, OwnedFd};

use super::session::Session;
use super::watch::Watch;
use super::{Answer, Attended, Kept, Rig};
use crate::failure::{Doing, Failure};

/// What a served session produced.
///
/// Reaching one of these means the conversation ran and was seen out. A
/// `Failure` instead means it never got that far: the workspace could not be
/// laid, or a reaction could not do its work.
pub struct Served<K> {
    /// Every shell that joined, in the order they did, with what its reaction
    /// left behind.
    pub shells: Vec<Attended<K>>,

    /// What went wrong closing up, if anything: a message left half-read, or a
    /// reaction that would not let go. Both happen after the conversation
    /// reached its own end.
    pub failed: Option<Failure>,
}

/// A rig a running bash may attach to.
///
/// | | |
/// |---|---|
/// | what ends it | the handle the initiator holds, watched and never closed here |
/// | what comes back | [`Served`] — every [`Attended`] shell, and what went wrong closing up |
///
/// `held` is a descriptor the initiator holds open for as long as it wants the
/// session: serving ends when the last holder has let go, whether the client
/// released it or died with it. Releasing it and waiting for the process it
/// started is how a client closes and learns that the reading is written.
///
/// `announce` is handed the session's address — one command, which the client
/// runs to join. It is called once, before anything is served, so a client
/// waiting for it is unblocked before the first message can arrive.
///
/// What the address reaches is the client's decision. Sourcing it instruments
/// that shell, its functions, its subshells and what it sources; exporting
/// `BASH_ENV` to the same path instruments the processes it starts.
pub trait Serving: Rig {
    fn serve<A>(&self, held: OwnedFd, announce: A) -> Result<Served<Kept<Self>>, Failure>
    where
        A: FnOnce(&Answer) -> Result<(), Failure>,
        Self: Sized,
    {
        let mut session = Session::open(self)?;

        let address = &session.layout.prelude;
        let path = address.to_str().ok_or_else(|| {
            Failure::new("announcing the session", format!("{} is not text", address.display()))
        })?;
        announce(&Answer::of("source", [path]))?;

        session.drive(&Watch::held(held))?;

        let (shells, failed) = session.finish();

        Ok(Served { shells, failed })
    }

    /// Serve the client that started this process as a coprocess: it holds this
    /// process's standard input, and reads the address from its standard
    /// output.
    ///
    /// That convention has a second half, and `BC_JOIN` in `assets/joining.bash`
    /// is it — one word doing the `coproc`, the one `read` and the `declare -a`.
    /// A server that wants a channel of its own calls [`serve`](Serving::serve)
    /// instead.
    fn serve_coprocess(&self) -> Result<Served<Kept<Self>>, Failure>
    where
        Self: Sized,
    {
        let held = io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .doing(|| "taking hold of the handle the client kept".into())?;

        // `println!` writes through a line buffer, so the newline that ends the
        // address is also what puts it on the pipe the client is blocked on.
        self.serve(held, |address| {
            println!("{address}");

            Ok(())
        })
    }
}

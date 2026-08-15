//! bash orchestrates: a script that is already running started the server,
//! takes the address it is handed, and lets go when it is done.
//!
//! Nothing here starts a process or ends one. What the client started, the
//! client cleans up, which is why the only thing this side watches is the
//! handle.

use std::io;
use std::os::fd::{AsFd, OwnedFd};

use tokio::task::LocalSet;

use super::session::Session;
use super::watch::Watch;
use super::{Answer, Attended, Kept, Rig};
use crate::failure::{Doing, Failure};

/// What a served session produced.
///
/// Reaching one means the conversation ran and was seen out. A `Failure`
/// instead means it never got that far.
pub struct Served<K> {
    /// Every shell that joined, in the order they did.
    pub shells: Vec<Attended<K>>,

    /// What went wrong closing up, if anything: a line left half-read, or a
    /// reaction that would not let go.
    pub failed: Option<Failure>,
}

/// A rig a running bash may attach to.
///
/// | | |
/// |---|---|
/// | what ends it | the handle the initiator holds, watched and never closed here |
/// | what comes back | [`Served`] |
///
/// `held` is a descriptor the initiator holds open for as long as it wants the
/// session: serving ends when the last holder has let go. `announce` is handed
/// the session's address — one command, which the client runs to join — once,
/// before anything is served.
///
/// What the address reaches is the client's decision. Sourcing it instruments
/// that shell, its functions, its subshells and what it sources; exporting
/// `BASH_ENV` to the same path instruments the processes it starts.
#[expect(async_fn_in_trait, reason = "single-threaded by design: no Send bound")]
pub trait Serving: Rig {
    async fn serve<A>(&self, held: OwnedFd, announce: A) -> Result<Served<Kept<Self>>, Failure>
    where
        A: FnOnce(&Answer) -> Result<(), Failure>,
        Self: Sized,
    {
        LocalSet::new()
            .run_until(async {
                let mut session = Session::open(self)?;

                let address = &session.layout.prelude;
                let path = address.to_str().ok_or_else(|| {
                    Failure::new("announcing the session", format!("{} is not text", address.display()))
                })?;
                announce(&Answer::of("source", [path]))?;

                session.serve(&Watch::held(held)?).await?;
                let (shells, failed) = session.close().await;

                Ok(Served { shells, failed })
            })
            .await
    }

    /// Serve the client that started this process as a coprocess: it holds this
    /// process's standard input, and reads the address from its standard
    /// output. `BC_START` in `assets/joining.bash` is the other half.
    async fn serve_coprocess(&self) -> Result<Served<Kept<Self>>, Failure>
    where
        Self: Sized,
    {
        let held = io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .doing(|| "taking hold of the handle the client kept".into())?;

        // `println!` is line-buffered: the newline that ends the address is
        // what puts it on the pipe the client is blocked on.
        self.serve(held, |address| {
            println!("{address}");

            Ok(())
        })
        .await
    }
}

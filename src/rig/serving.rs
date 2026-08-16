//! bash orchestrates: a script that is already running names the workspace,
//! starts the server, joins at the address its own choice fixed, and lets go
//! when it is done.
//!
//! Nothing here starts a process or ends one. What the client started, the
//! client cleans up, which is why the only thing this side watches is the
//! handle.

use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::path::Path;

use tokio::task::LocalSet;

use super::session::Session;
use super::watch::Watch;
use super::{Attended, Kept, Rig};
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
/// | who chose the workspace | the client: `at` is required, so it knows the address before the server runs |
/// | what the client is handed | the address — `<at>/session.bash`, one line, after the files are down |
/// | what ends it | the handle the initiator holds, watched and never closed here |
/// | what comes back | [`Served`] |
///
/// `at` is the workspace the client prescribed — created if missing, left
/// behind: a reading taken later may follow source paths into it. `held` is a
/// descriptor the initiator holds open for as long as it wants the session:
/// serving ends when the last holder has let go. `announce` is handed the
/// address once, after the session is laid and before anything is served — the
/// read on the client's side is what says the session is ready; the client
/// puts the line in `BC_SESSION` and runs `source "$BC_SESSION"` — see
/// [`JOINING`](super::JOINING).
///
/// What the address reaches is the client's decision. Sourcing it instruments
/// that shell, its functions, its subshells and what it sources; exporting
/// `BASH_ENV` to the same path instruments the processes it starts.
///
/// A `Failure` while serving still sees the session out: every shell released
/// or finished, the workspace's fifos gone.
#[expect(async_fn_in_trait, reason = "single-threaded by design: no Send bound")]
pub trait Serving: Rig {
    async fn serve<A>(
        &self,
        at: &Path,
        held: OwnedFd,
        announce: A,
    ) -> Result<Served<Kept<Self>>, Failure>
    where
        A: FnOnce(&str) -> Result<(), Failure>,
        Self: Sized,
    {
        LocalSet::new()
            .run_until(async {
                let mut session = Session::open(self, Some(at))?;

                let served = async {
                    let watch = Watch::held(held)?;
                    announce(&session.layout.address)?;
                    session.serve(&watch).await
                }
                .await;
                let (shells, failed) = session.close().await;
                served?;

                Ok(Served { shells, failed })
            })
            .await
    }

    /// Serve the client that started this process as a coprocess: it holds this
    /// process's standard input, and reads the address from its standard
    /// output. `BC_START` in `assets/joining.bash` is the other half.
    async fn serve_coprocess(&self, at: &Path) -> Result<Served<Kept<Self>>, Failure>
    where
        Self: Sized,
    {
        let held = io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .doing(|| "taking hold of the handle the client kept".into())?;

        // `println!` is line-buffered: the newline that ends the address is
        // what puts it on the pipe the client is blocked on.
        self.serve(at, held, |address| {
            println!("{address}");

            Ok(())
        })
        .await
    }
}

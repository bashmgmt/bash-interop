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
use super::{Attended, Kept, Layout, Rig};
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
/// | what the client is handed | the address — the file it sources to join, one line |
/// | what ends it | the handle the initiator holds, watched and never closed here |
/// | what comes back | [`Served`] |
///
/// `held` is a descriptor the initiator holds open for as long as it wants the
/// session: serving ends when the last holder has let go. `announce` is handed
/// the address once, before anything is served; the client puts it in
/// `BC_SESSION` and runs `source "$BC_SESSION"` — see [`JOINING`](super::JOINING).
///
/// What the address reaches is the client's decision. Sourcing it instruments
/// that shell, its functions, its subshells and what it sources; exporting
/// `BASH_ENV` to the same path instruments the processes it starts.
///
/// A `Failure` while serving still sees the session out: every shell released
/// or finished, the workspace's fifos gone.
#[expect(async_fn_in_trait, reason = "single-threaded by design: no Send bound")]
pub trait Serving: Rig {
    async fn serve<A>(&self, held: OwnedFd, announce: A) -> Result<Served<Kept<Self>>, Failure>
    where
        A: FnOnce(&str) -> Result<(), Failure>,
        Self: Sized,
    {
        LocalSet::new()
            .run_until(async {
                let mut session = Session::open(self)?;
                announce(address(&session.layout)?)?;

                let served = session.serve(&Watch::held(held)?).await;
                let (shells, failed) = session.close().await;
                served?;

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

/// The address as one line of text: what `read -r` on the client's side takes.
fn address(layout: &Layout) -> Result<&str, Failure> {
    let path = layout.prelude.to_str().filter(|path| !path.contains('\n')).ok_or_else(|| {
        Failure::new("announcing the session", format!("{} is not one line", layout.prelude.display()))
    })?;

    Ok(path)
}

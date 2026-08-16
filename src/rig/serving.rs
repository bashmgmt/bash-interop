//! bash orchestrates: a script that is already running names and makes the
//! workspace, starts the server, joins at the coordinate its own choice
//! fixed, and lets go when it is done.
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
/// | who chose the workspace | the client: `at` is required, exists, and is the address |
/// | what the client is handed | nothing — liveness is the workspace's to show: the join fifo is present exactly while the session serves |
/// | what ends it | the handle the initiator holds, watched and never closed here |
/// | what comes back | [`Served`] |
///
/// `at` is the workspace the client prescribed and made — left behind: a
/// reading taken later may follow source paths into it. `held` is a
/// descriptor the initiator holds open for as long as it wants the session:
/// serving ends when the last holder has let go.
///
/// A serving application is a complete standalone program: it owes nobody a
/// byte on any channel. A client that wants to know the session is up asks
/// the workspace — `BC_UP` in [`JOINING_BASH`](super::JOINING_BASH) — loads
/// the laid definitions (`BC_LOAD`) and initiates its own channel, feeding
/// every step the same coordinate it gave the server. What the session
/// reaches is the client's decision: joining instruments that shell, its
/// functions, its subshells and what it sources; a client that wants its
/// child processes reached writes its own startup file and exports
/// `BASH_ENV` to it.
///
/// A `Failure` while serving still sees the session out: every shell
/// released or finished, the workspace's fifos gone.
#[expect(async_fn_in_trait, reason = "single-threaded by design: no Send bound")]
pub trait Serving: Rig {
    async fn serve(&self, at: &Path, held: OwnedFd) -> Result<Served<Kept<Self>>, Failure>
    where
        Self: Sized,
    {
        LocalSet::new()
            .run_until(async {
                let mut session = Session::open(self, Some(at))?;

                let served = async {
                    let watch = Watch::held(held)?;
                    session.serve(&watch).await
                }
                .await;
                let (shells, failed) = session.close().await;
                served?;

                Ok(Served { shells, failed })
            })
            .await
    }

    /// Serve the client that started this process as a coprocess: it holds
    /// this process's standard input, and lets go to end the session.
    /// `BC_START` in `assets/joining.bash` is the other half.
    async fn serve_coprocess(&self, at: &Path) -> Result<Served<Kept<Self>>, Failure>
    where
        Self: Sized,
    {
        let held = io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .doing(|| "taking hold of the handle the client kept".into())?;

        self.serve(at, held).await
    }
}

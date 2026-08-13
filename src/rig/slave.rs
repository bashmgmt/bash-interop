//! bash orchestrates: a script that is already running started the server,
//! takes the address it is handed, and says when the session is over.
//!
//! Nothing here starts a process or ends one. What the client started, the
//! client cleans up — which is why the only thing this side can watch is the
//! handle, and why a session that is closed properly is closed by a message.

use std::io;
use std::os::fd::{FromRawFd, OwnedFd};

use super::serving::{Served, Serving, Until};
use super::{Answer, Rig};
use crate::failure::{Doing, Failure};

/// The handle an initiator holds open for as long as its session lasts.
///
/// It hangs up when the last holder has let go, which is the only thing that
/// can end a session nobody is left to close. An initiator that closes
/// properly never reaches it.
pub struct Held(OwnedFd);

impl Held {
    pub fn of(handle: OwnedFd) -> Self {
        Self(handle)
    }

    /// This process's own standard input — what a client holds the other end
    /// of when it started the server as a coprocess.
    pub fn stdin() -> Result<Self, Failure> {
        let raw = unsafe { libc::dup(libc::STDIN_FILENO) };
        if raw < 0 {
            return Err(io::Error::last_os_error()).doing(|| "taking the session handle".into());
        }

        Ok(Self(unsafe { OwnedFd::from_raw_fd(raw) }))
    }
}

/// A rig a running bash may attach to.
///
/// `announce` is handed the session's address: one command, which the client
/// runs to join. It is called once, before anything is served, so a client
/// that is waiting for it is unblocked before the first message can arrive.
///
/// What reaches the address is the client's decision. Sourcing it instruments
/// that shell, its functions, its subshells and what it sources; exporting
/// `BASH_ENV` to it as well instruments the processes it starts.
pub trait Slave: Rig {
    fn serve<A>(&self, held: Held, announce: A) -> Result<Served<Self::Session>, Failure>
    where
        A: FnOnce(&Answer) -> Result<(), Failure>,
        Self: Sized,
    {
        let mut serving = Serving::lay(self)?;

        let address = {
            let prelude = serving.prelude();
            let path = prelude.to_str().ok_or_else(|| {
                Failure::new("announcing the session", format!("{} is not text", prelude.display()))
            })?;

            Answer::of("source", [path])
        };
        announce(&address)?;

        let closed = serving.drive(&Until::held(held.0))?;

        Ok(serving.finish(closed))
    }
}

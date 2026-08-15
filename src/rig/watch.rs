//! What a session ends on.
//!
//! A [`Watch`] is only ever observed. Signalling and reaping belong to whoever
//! started the thing being watched, which is never this — and that is what
//! lets one loop serve both orchestrations.

use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

use tokio::io::unix::AsyncFd;

use crate::failure::{Doing, Failure};

/// A descriptor that becomes readable when nobody who could speak is left.
pub(super) struct Watch(AsyncFd<OwnedFd>);

impl Watch {
    /// A process whose end is the end. `pidfd_open` needs no ownership of it,
    /// which is why the same watch serves a child and a stranger.
    pub(super) fn process(pid: libc::pid_t) -> Result<Self, Failure> {
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if raw < 0 {
            return Err(io::Error::last_os_error()).doing(|| format!("watching bash {pid}"));
        }

        Self::over(unsafe { OwnedFd::from_raw_fd(raw as RawFd) })
    }

    /// A handle an initiator holds: readable once the last holder has let go.
    pub(super) fn held(handle: OwnedFd) -> Result<Self, Failure> {
        Self::over(handle)
    }

    fn over(fd: OwnedFd) -> Result<Self, Failure> {
        AsyncFd::new(fd).map(Self).doing(|| "registering the watch".into())
    }

    /// Resolves once, when the end has come. A pidfd reports readable when its
    /// process exits; a handle reports hangup, which readiness includes.
    pub(super) async fn fired(&self) -> Result<(), Failure> {
        self.0.readable().await.map(|_| ()).doing(|| "waiting for the end".into())
    }
}

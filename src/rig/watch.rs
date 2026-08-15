//! What a session ends on, and the one poll that waits for it.
//!
//! A [`Watch`] is only ever observed. Signalling and reaping belong to whoever
//! started the thing being watched, which is never this — and that is what lets
//! one loop serve both orchestrations.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use super::wire::Wire;
use crate::failure::{Doing, Failure};

/// A descriptor that becomes ready when nobody who could speak is left.
pub(super) struct Watch(OwnedFd);

impl Watch {
    /// A process whose end is the end. `pidfd_open` needs no ownership of it,
    /// which is why the same watch serves a child and a stranger.
    pub(super) fn process(pid: libc::pid_t) -> Result<Self, Failure> {
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if raw < 0 {
            return Err(io::Error::last_os_error()).doing(|| format!("watching bash {pid}"));
        }

        Ok(Self(unsafe { OwnedFd::from_raw_fd(raw as RawFd) }))
    }

    /// A handle an initiator holds: ready once the last holder has let go.
    pub(super) fn held(handle: OwnedFd) -> Self {
        Self(handle)
    }
}

pub(super) enum Ready {
    Spoke,
    Gone,
}

/// One `poll` over the pipe and the end condition at once. A ready descriptor
/// does not imply an empty pipe, so the pipe is checked first.
///
/// `events` asks for `POLLIN` on both, which is what a pidfd reports when its
/// process exits. A handle reports `POLLHUP` or `POLLERR` instead, and `poll`
/// delivers those whether or not they were asked for — so the second descriptor
/// is read as ready on anything at all.
pub(super) fn wait_for(wire: &Wire, watch: &Watch) -> Result<Ready, Failure> {
    loop {
        let mut watching = [
            libc::pollfd { fd: wire.reader(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: watch.0.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];

        if unsafe { libc::poll(watching.as_mut_ptr(), 2, -1) } < 0 {
            let cause = io::Error::last_os_error();
            if cause.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(cause).doing(|| "waiting for the conversation".into());
        }

        if watching[0].revents & libc::POLLIN != 0 {
            return Ok(Ready::Spoke);
        }
        if watching[1].revents != 0 {
            return Ok(Ready::Gone);
        }
    }
}

//! The conversation itself: a workspace with a wire in it, a session beside
//! it, and the loop that serves until nobody can speak any more.
//!
//! What that means is an [`Until`] — a descriptor the role built. Which shells
//! it stands for, and whether anything is owed to them afterwards, belongs to
//! the role and not here.

use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::wire::{prelude, Kind, Wire};
use super::{Rig, Workspace};
use crate::failure::{Doing, Failure};

/// A laid conversation: the workspace is written, the pipe is open, the
/// session is up. What is missing is who speaks and what ends it.
pub(super) struct Serving<'r, R: Rig> {
    rig: &'r R,
    session: R::Session,
    wire: Wire,
    prelude: PathBuf,

    /// Held only to be dropped: it takes the workspace with it, and it goes
    /// last so nothing is reading the files when it does.
    _temporary: Option<TempDir>,
}

impl<'r, R: Rig> Serving<'r, R> {
    /// The workspace is canonicalised: every shell reads its own location from
    /// the path it was sourced from, so a relative one would move with the
    /// subject.
    pub(super) fn lay(rig: &'r R) -> Result<Self, Failure> {
        let (at, temporary) = match rig.workspace() {
            Workspace::At(at) => (at, None),
            Workspace::Temporary => {
                let temp =
                    tempfile::tempdir().doing(|| "opening a workspace for the run".into())?;

                (temp.path().to_path_buf(), Some(temp))
            }
        };
        let opening = || format!("opening the workspace {}", at.display());

        fs::create_dir_all(&at).doing(opening)?;
        let dir = fs::canonicalize(&at).doing(opening)?;

        let wire = Wire::create(&dir)?;
        let prelude = prelude(&dir, &rig.bash())?;
        let session = rig.open()?;

        Ok(Self { rig, session, wire, prelude, _temporary: temporary })
    }

    /// The file a shell has to source to join. It is the session's only
    /// address.
    pub(super) fn prelude(&self) -> &Path {
        &self.prelude
    }

    /// Every message the pipe holds, handed to the rig one at a time. A shell
    /// that asked is blocked until its answer is written, so writing it is
    /// part of delivering rather than something a caller does afterwards.
    ///
    /// A shell's account of itself is delivered like anything nobody waits on:
    /// it belongs to the session, where a decoder that has to read a walk
    /// against the shell it was taken in will look for it.
    fn deliver(&mut self) -> Result<(), Failure> {
        for line in self.wire.drain()? {
            // The rig consumes the message, and the reply pipe is named after
            // the shell that sent it.
            let asking = line.sent.pid;

            match line.kind {
                Kind::Say | Kind::Join => self.rig.hear(&mut self.session, line)?,
                Kind::Ask => {
                    let answer = self.rig.answer(&mut self.session, line)?;

                    self.wire.answer(asking, answer)?;
                }
            }
        }
        Ok(())
    }

    /// Serve until nobody can speak any more. There is no interval and no
    /// timer.
    ///
    /// The pipe is polled first, so a message already waiting is read before
    /// the end is noticed, and the delivery behind the loop takes what arrived
    /// with it.
    pub(super) fn drive(&mut self, until: &Until) -> Result<(), Failure> {
        while let Ready::Spoke = wait_for(&self.wire, until)? {
            self.deliver()?;
        }

        self.deliver()
    }

    /// Release what the session holds. A message left half-read is reported
    /// before the rig is asked to end, since it is the earlier fault.
    pub(super) fn finish(self) -> (R::Session, Option<Failure>) {
        let Self { rig, mut session, wire, .. } = self;

        let failed = wire.finish().err().or_else(|| rig.end(&mut session).err());

        (session, failed)
    }
}

/// What a serving ends on: a descriptor that becomes ready when nobody who
/// could speak is left.
///
/// It is only ever watched. Signalling and reaping belong to whoever started
/// the thing being watched, which is never this.
pub(super) struct Until(OwnedFd);

impl Until {
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

enum Ready {
    Spoke,
    Gone,
}

/// One `poll` over the pipe and the end condition at once. A ready descriptor
/// does not imply an empty pipe, so the pipe is checked first.
///
/// `events` asks for `POLLIN` on both, which is what a pidfd reports when its
/// process exits. A handle reports `POLLHUP` or `POLLERR` instead, and `poll`
/// delivers those whether or not they were asked for — so the second
/// descriptor is read as ready on anything at all.
fn wait_for(wire: &Wire, until: &Until) -> Result<Ready, Failure> {
    loop {
        let mut watching = [
            libc::pollfd { fd: wire.reader(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: until.0.as_raw_fd(), events: libc::POLLIN, revents: 0 },
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

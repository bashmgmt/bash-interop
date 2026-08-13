//! The conversation itself: a workspace with a wire in it, a session beside
//! it, and the loop that serves until something says stop.
//!
//! A serving has exactly two exits, and both exist however it was started. An
//! [`Until`] is one of them — a descriptor that becomes ready when nobody can
//! speak any more; [`Halt::Done`] from the rig is the other. Which of the two
//! is the ordinary one is a fact about who started what, so neither the loop
//! nor this module asks.

use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::wire::{prelude, Kind, Wire};
use super::{Halt, Rig, Workspace};
use crate::failure::{Doing, Failure};

/// What a serving produced.
///
/// Reaching one of these means the conversation ran and was seen out. A
/// `Failure` instead means it never got that far: the workspace could not be
/// laid, or the rig could not do its work.
pub struct Served<S> {
    /// The client's own state, whatever it made of what it heard.
    pub session: S,

    /// Which of the two exits was taken.
    pub closed: Closed,

    /// What went wrong closing up, if anything: a message left half-read, or a
    /// session that would not let go. Both happen after the conversation
    /// reached its own end.
    pub failed: Option<Failure>,
}

/// How a serving came to an end.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Closed {
    /// The rig said so.
    Said,

    /// What else it ends on became ready: nobody is left who could say it.
    Gone,
}

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
    fn deliver(&mut self) -> Result<(), Halt> {
        for line in self.wire.drain()? {
            // The rig consumes the message, and the reply pipe is named after
            // the shell that sent it.
            let asking = line.sent.pid;

            match line.kind {
                Kind::Say => self.rig.hear(&mut self.session, line)?,
                Kind::Ask => {
                    let answer = self.rig.answer(&mut self.session, line)?;

                    self.wire.answer(asking, answer)?;
                }
            }
        }
        Ok(())
    }

    /// One step: deliver what is there, then wait. `Gone` is reported only
    /// after a last delivery, because a ready descriptor does not mean the
    /// pipe is empty.
    fn step(&mut self, until: &Until) -> Result<Ready, Halt> {
        self.deliver()?;

        match wait_for(&self.wire, until)? {
            Ready::Spoke => Ok(Ready::Spoke),
            Ready::Gone => {
                self.deliver()?;

                Ok(Ready::Gone)
            }
        }
    }

    /// Serve until something says stop. There is no interval and no timer.
    ///
    /// This is the one place where the rig's vocabulary becomes the run's:
    /// above it there is no [`Halt`], below it no [`Closed`].
    pub(super) fn drive(&mut self, until: &Until) -> Result<Closed, Failure> {
        loop {
            match self.step(until) {
                Ok(Ready::Spoke) => continue,
                Ok(Ready::Gone) => return Ok(Closed::Gone),
                Err(Halt::Done) => return Ok(Closed::Said),
                Err(Halt::Failed(why)) => return Err(why),
            }
        }
    }

    /// Release what the session holds. A message left half-read is reported
    /// before the rig is asked to end, since it is the earlier fault.
    pub(super) fn finish(self, closed: Closed) -> Served<R::Session> {
        let Self { rig, mut session, wire, .. } = self;

        let failed = wire.finish().err().or_else(|| rig.end(&mut session).err());

        Served { session, closed, failed }
    }
}

/// What ends a serving when the rig does not: a descriptor that becomes ready
/// when nobody can speak any more.
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

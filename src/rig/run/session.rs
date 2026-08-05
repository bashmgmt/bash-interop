//! Driving one run.
//!
//! There is no polling interval and no timer. The subject's exit is a
//! readable file descriptor like any other, so one `poll` waits on both the
//! pipe and the child at once: an ask costs a wakeup and a write.
//!
//! A run owns its subject's process group. Every exit path from here — a
//! clean return, an error, or an unwind — releases it, so no shell is ever
//! left blocked on a pipe nobody will write to again.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use super::{Outcome, Rig, RigError, Setup, Turn, Workspace};
use crate::bash::rig::wire::Wire;

pub(crate) fn run(rig: &mut impl Rig, argv: &[String]) -> Result<Outcome, RigError> {
    let setup = rig.setup();
    let site = Site::open(&setup.workspace)?;
    let dir = site.dir();

    let mut wire = Wire::create(dir).map_err(RigError::Pipe)?;
    let prelude = dir.join("prelude.bash");
    std::fs::write(&prelude, rig.prelude(dir, wire.up_path())?.as_str())
        .map_err(|cause| RigError::Prelude { path: prelude.clone(), cause })?;

    let mut subject = Subject::spawn(argv, &prelude, &setup)?;

    loop {
        serve(rig, &mut wire, dir)?;
        match wait_for(wire.reader(), subject.exit()).map_err(RigError::Read)? {
            Ready::Spoke => continue,
            Ready::Exited => break,
        }
    }
    // Whatever the subject said just before it went is still in the pipe.
    serve(rig, &mut wire, dir)?;

    let status = subject.finish().map_err(RigError::Wait)?;
    let debug = std::fs::read_to_string(dir.join("debug.log"))
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default();

    Ok(Outcome { capture: wire.finish(), status: status.into(), debug })
}

/// Takes everything the pipe holds and answers whatever asked. Every answer
/// in a pass is decided against the same history, then applied.
fn serve(rig: &mut impl Rig, wire: &mut Wire, dir: &Path) -> Result<(), RigError> {
    for ask in wire.drain().map_err(RigError::Read)? {
        let reply = rig.answer(&Turn::new(&ask, wire.seen(), dir));
        wire.answer(ask.stamp.pid, reply).map_err(RigError::Reply)?;
    }
    Ok(())
}

enum Ready {
    Spoke,
    Exited,
}

/// Blocks until the subject says something or ends. Nothing else can wake it,
/// so there is no interval to tune and nothing to miss.
fn wait_for(pipe: RawFd, exit: RawFd) -> io::Result<Ready> {
    loop {
        let mut watching = [
            libc::pollfd { fd: pipe, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: exit, events: libc::POLLIN, revents: 0 },
        ];

        if unsafe { libc::poll(watching.as_mut_ptr(), 2, -1) } < 0 {
            let cause = io::Error::last_os_error();
            match cause.kind() {
                io::ErrorKind::Interrupted => continue,
                _ => return Err(cause),
            }
        }

        // The pipe first: what the subject said before exiting is still in it,
        // and it has to be read before the run is allowed to end.
        if watching[0].revents & libc::POLLIN != 0 {
            return Ok(Ready::Spoke);
        }
        if watching[1].revents != 0 {
            return Ok(Ready::Exited);
        }
    }
}

/// The subject, its process group, and the descriptor that becomes readable
/// when it ends.
struct Subject {
    child: Option<Child>,
    group: libc::pid_t,
    exit: OwnedFd,
}

impl Subject {
    fn spawn(argv: &[String], prelude: &Path, setup: &Setup) -> Result<Self, RigError> {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new("bash");
        command.args(argv).env("BASH_ENV", prelude);
        for (key, value) in &setup.env {
            command.env(key, value);
        }

        // Its own group, so releasing the subject releases everything it
        // started. Killing only the direct child would leave a grandchild
        // that asked blocked on its reply pipe forever.
        command.process_group(0);

        let child = command.spawn().map_err(RigError::Spawn)?;
        let group = child.id() as libc::pid_t;
        let exit = pidfd(group).map_err(RigError::Spawn)?;

        Ok(Self { child: Some(child), group, exit })
    }

    fn exit(&self) -> RawFd {
        self.exit.as_raw_fd()
    }

    /// Releases the group, then reaps. In that order: while the subject is
    /// still unreaped its group cannot have been recycled, so the signal
    /// cannot reach anything else.
    fn finish(&mut self) -> io::Result<std::process::ExitStatus> {
        self.release();
        match self.child.take() {
            Some(mut child) => child.wait(),
            None => Err(io::Error::other("the subject was already finished")),
        }
    }

    fn release(&self) {
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
    }
}

impl Drop for Subject {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.finish();
        }
    }
}

fn pidfd(pid: libc::pid_t) -> io::Result<OwnedFd> {
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(raw as RawFd) })
}

/// Where a run keeps its files, once the description has been acted on.
enum Site {
    Temporary(tempfile::TempDir),
    Kept(PathBuf),
}

impl Site {
    fn open(workspace: &Workspace) -> Result<Self, RigError> {
        match workspace {
            Workspace::Temporary => {
                tempfile::tempdir().map(Self::Temporary).map_err(RigError::Workspace)
            }
            Workspace::At(path) => {
                std::fs::create_dir_all(path).map_err(RigError::Workspace)?;
                Ok(Self::Kept(path.clone()))
            }
        }
    }

    fn dir(&self) -> &Path {
        match self {
            Self::Temporary(temp) => temp.path(),
            Self::Kept(path) => path,
        }
    }
}

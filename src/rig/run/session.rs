//! Driving one run.
//!
//! There is no polling interval and no timer. The subject's exit is a
//! readable file descriptor like any other, so one `poll` waits on both the
//! pipe and the child at once: an ask costs a wakeup and a write.
//!
//! A run owns its subject's process group. Every exit path from here — a
//! clean return, an error, or an unwind — releases it, so no shell is ever
//! left blocked on a pipe nobody will write to again.

use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use super::{Outcome, Rig, Setup, Turn, Workspace};
use crate::bash::rig::capture::Capture;
use crate::bash::rig::error::{Doing, RigError};
use crate::bash::rig::wire::Wire;

pub(crate) fn run(rig: &mut impl Rig, argv: &[String]) -> Result<Outcome, RigError> {
    let setup = rig.setup()?;
    let site = Site::open(&setup.workspace)?;
    let dir = site.dir();

    let mut wire = Wire::create(dir)?;
    let prelude = dir.join("prelude.bash");
    fs::write(&prelude, rig.prelude(dir, wire.up_path())?.as_str())
        .doing(|| format!("writing the prelude to {}", prelude.display()))?;

    let mut subject = Subject::spawn(argv, &prelude, &setup)?;
    let mut capture = Capture::default();

    loop {
        serve(rig, &mut wire, &mut capture, dir)?;
        match wait_for(wire.reader(), subject.exit())? {
            Ready::Spoke => continue,
            Ready::Exited => break,
        }
    }
    // Whatever the subject said just before it went is still in the pipe.
    serve(rig, &mut wire, &mut capture, dir)?;
    wire.finish()?;

    let status = subject.finish().doing(|| "waiting for bash".into())?;
    Ok(Outcome { capture, status: status.into() })
}

/// Takes everything the pipe holds, then answers whatever asked. Reading
/// first is what lets an answer see the question in its own history, and lets
/// every answer in a pass be decided against the same one.
fn serve(
    rig: &mut impl Rig,
    wire: &mut Wire,
    capture: &mut Capture,
    dir: &Path,
) -> Result<(), RigError> {
    let arrived = capture.lines.len();
    capture.lines.extend(wire.drain()?);

    for index in arrived..capture.lines.len() {
        let Some(turn) = Turn::over(&capture.lines[index], capture, dir) else { continue };
        let reply = rig.answer(&turn)?;
        wire.answer(turn.stamp().pid, reply)?;
    }
    Ok(())
}

enum Ready {
    Spoke,
    Exited,
}

/// Blocks until the subject says something or ends. Nothing else can wake it,
/// so there is no interval to tune and nothing to miss.
fn wait_for(pipe: RawFd, exit: RawFd) -> Result<Ready, RigError> {
    loop {
        let mut watching = [
            libc::pollfd { fd: pipe, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: exit, events: libc::POLLIN, revents: 0 },
        ];

        if unsafe { libc::poll(watching.as_mut_ptr(), 2, -1) } < 0 {
            let cause = io::Error::last_os_error();
            if cause.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(cause).doing(|| "waiting for the subject".into());
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

        let child = command.spawn().doing(|| format!("spawning bash {}", argv.join(" ")))?;
        let group = child.id() as libc::pid_t;
        let exit = pidfd(group).doing(|| format!("watching bash {group}"))?;

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
            Workspace::Temporary => tempfile::tempdir()
                .map(Self::Temporary)
                .doing(|| "opening a temporary workspace".into()),

            Workspace::At(path) => {
                fs::create_dir_all(path)
                    .doing(|| format!("opening the workspace {}", path.display()))?;
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

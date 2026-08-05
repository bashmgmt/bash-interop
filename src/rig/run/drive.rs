//! Driving one run: `start`, then every event, then `ended`.
//!
//! The subject's exit is a `pidfd`, so one `poll` waits on both it and the
//! pipe; there is no interval and no timer. Every exit path from here kills
//! the subject's process group.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use super::{Rig, Setup, Turn, Workspace};
use crate::bash::rig::error::{Doing, RigError};
use crate::bash::rig::source;
use crate::bash::rig::wire::Wire;

pub(crate) fn run<R: Rig, S: AsRef<OsStr>>(rig: &R, argv: &[S]) -> Result<R::Output, RigError> {
    let (setup, mut session) = rig.start()?;
    let site = Site::open(&setup.workspace)?;
    let dir = site.dir();

    let mut wire = Wire::create(dir)?;
    let written = dir.join("prelude.bash");
    let bash = source::prelude(&setup.bash, setup.debug, dir, wire.up_path())?;
    fs::write(&written, bash.as_str())
        .doing(|| format!("writing the prelude to {}", written.display()))?;

    let mut subject = Subject::spawn(argv, &written, &setup)?;

    loop {
        serve(rig, &mut session, &mut wire, dir)?;
        match wait_for(wire.reader(), subject.exit())? {
            Ready::Spoke => continue,
            Ready::Exited => break,
        }
    }
    // Whatever the subject said just before it went is still in the pipe.
    serve(rig, &mut session, &mut wire, dir)?;
    wire.finish()?;

    let status = subject.finish().doing(|| "waiting for bash".into())?;
    rig.ended(session, status.into())
}

/// `say` goes one way and `ask` the other, exactly as in bash.
fn serve<R: Rig>(
    rig: &R,
    session: &mut R::Session,
    wire: &mut Wire,
    dir: &Path,
) -> Result<(), RigError> {
    for line in wire.drain()? {
        let Some(args) = line.value.asked() else {
            rig.heard(session, line)?;
            continue;
        };

        let reply = rig.answer(session, &Turn::new(&line, args, dir))?;
        wire.answer(line.stamp.pid, reply)?;
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
    fn spawn<S: AsRef<OsStr>>(argv: &[S], prelude: &Path, setup: &Setup) -> Result<Self, RigError> {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new("bash");
        command.args(argv).env("BASH_ENV", prelude);
        for (key, value) in &setup.env {
            command.env(key, value);
        }

        // Its own group: killing only the direct child would leave a
        // grandchild that asked blocked on its reply pipe forever.
        command.process_group(0);

        let child = command.spawn().doing(|| {
            let words: Vec<_> = argv.iter().map(|word| word.as_ref().to_string_lossy()).collect();
            format!("spawning bash {}", words.join(" "))
        })?;
        let group = child.id() as libc::pid_t;
        let exit = pidfd(group).doing(|| format!("watching bash {group}"))?;

        Ok(Self { child: Some(child), group, exit })
    }

    fn exit(&self) -> RawFd {
        self.exit.as_raw_fd()
    }

    /// Kills the group, then reaps. In that order: an unreaped subject's
    /// group cannot have been recycled, so the signal reaches nothing else.
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

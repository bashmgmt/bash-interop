//! Performing a run: everything the driver needs, and nothing a rig sees.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::process::{Child, Command};

use crate::bash::rig::wire::{prelude, Kind, Wire};
use crate::bash::rig::{ExitStatus, Rig};
use crate::failure::{Doing, Failure};

/// Run `argv` under `rig`, and hand back the session it drove and how bash
/// ended. The workspace is the run's, made and discarded with it — it drops
/// after `run_in` has reaped the subject that was reading it.
pub fn run<R: Rig, S: AsRef<OsStr>>(
    rig: &R,
    argv: &[S],
) -> Result<(R::Session, ExitStatus), Failure> {
    let workspace = tempfile::tempdir().doing(|| "opening a workspace for the run".into())?;

    run_in(rig, workspace.path(), argv)
}

/// The same, in a directory of your choosing, left behind to read. The
/// workspace is canonicalised: every shell reads its own location from the
/// path `BASH_ENV` names, so a relative one would move with the subject.
pub fn run_in<R: Rig, S: AsRef<OsStr>>(
    rig: &R,
    at: &Path,
    argv: &[S],
) -> Result<(R::Session, ExitStatus), Failure> {
    let opening = || format!("opening the workspace {}", at.display());

    fs::create_dir_all(at).doing(opening)?;
    let dir = fs::canonicalize(at).doing(opening)?;

    let mut running = Running::open(rig, &dir, argv)?;

    running.drive()?;
    running.finish()
}

/// The run's own machinery. The session sits alongside it and is the only
/// part the rig ever touches.
struct Running<'r, R: Rig> {
    rig: &'r R,
    session: R::Session,
    subject: Subject,
    wire: Wire,
}

impl<'r, R: Rig> Running<'r, R> {
    fn open<S: AsRef<OsStr>>(rig: &'r R, dir: &Path, argv: &[S]) -> Result<Self, Failure> {
        let session = rig.open()?;
        let wire = Wire::create(dir)?;
        let entry = prelude(dir, &rig.bash())?;
        let subject = Subject::spawn(argv, &entry)?;

        Ok(Self { rig, session, subject, wire })
    }

    /// Serve until bash is gone, then once more for what it said on the way
    /// out. There is no interval and no timer.
    fn drive(&mut self) -> Result<(), Failure> {
        loop {
            self.serve()?;
            match wait_for(&self.wire, &self.subject)? {
                Ready::Spoke => continue,
                Ready::Exited => break,
            }
        }
        self.serve()
    }

    fn serve(&mut self) -> Result<(), Failure> {
        for line in self.wire.drain()? {
            match line.kind {
                Kind::Say => self.rig.hear(&mut self.session, line)?,
                Kind::Ask => {
                    let waiting = line.pid;
                    let answer = self.rig.answer(&mut self.session, line)?;

                    self.wire.answer(waiting, answer)?;
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<(R::Session, ExitStatus), Failure> {
        let Self { rig, mut session, mut subject, wire } = self;

        wire.finish()?;
        let status = ExitStatus::from(subject.finish().doing(|| "waiting for bash".into())?);
        rig.end(&mut session, status)?;

        Ok((session, status))
    }
}

/// The bash the run owns: its process group, and the descriptor that becomes
/// readable when it is gone.
struct Subject {
    child: Child,
    group: libc::pid_t,
    exit: OwnedFd,
}

impl Subject {
    fn spawn<S: AsRef<OsStr>>(argv: &[S], prelude: &Path) -> Result<Self, Failure> {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new("bash");
        command.args(argv).env("BASH_ENV", prelude).process_group(0);

        let child = command.spawn().doing(|| {
            let words: Vec<_> = argv.iter().map(|word| word.as_ref().to_string_lossy()).collect();
            format!("spawning bash {}", words.join(" "))
        })?;
        let group = child.id() as libc::pid_t;
        let exit = pidfd(group).doing(|| format!("watching bash {group}"))?;

        Ok(Self { child, group, exit })
    }

    /// Kill the group, then reap — in that order, because while the subject
    /// is unreaped its group cannot have been recycled.
    fn finish(&mut self) -> io::Result<std::process::ExitStatus> {
        self.release();
        self.child.wait()
    }

    fn release(&self) {
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
    }
}

impl Drop for Subject {
    fn drop(&mut self) {
        self.release();
        // `wait`, not `try_wait`: an unreaped child still answers
        // `kill(pid, 0)`. `Child::wait` caches, so a second call is free.
        let _ = self.child.wait();
    }
}

enum Ready {
    Spoke,
    Exited,
}

/// One `poll` over the pipe and the subject at once. A readable `pidfd` does
/// not imply an empty pipe, so the pipe is checked first.
fn wait_for(wire: &Wire, subject: &Subject) -> Result<Ready, Failure> {
    loop {
        let mut watching = [
            libc::pollfd { fd: wire.reader(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: subject.exit.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];

        if unsafe { libc::poll(watching.as_mut_ptr(), 2, -1) } < 0 {
            let cause = io::Error::last_os_error();
            if cause.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(cause).doing(|| "waiting for the subject".into());
        }

        if watching[0].revents & libc::POLLIN != 0 {
            return Ok(Ready::Spoke);
        }
        if watching[1].revents != 0 {
            return Ok(Ready::Exited);
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

//! Rust orchestrates: the run starts the subject, reaches every shell in the
//! tree it creates, and owns its life.

use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::path::Path;
use std::process::{Child, Command};

use super::session::Session;
use super::watch::Watch;
use super::{Attended, Kept, Rig};
use crate::failure::{Doing, Failure};

/// What a driven run produced.
///
/// Reaching one of these means bash was started and seen out. A `Failure`
/// instead means the run never got that far: it could not be set up, or a
/// reaction could not do its work and the subject was killed — and then how the
/// subject would have ended is not something anyone can say.
pub struct Run<K> {
    /// Every shell that joined, in the order they did, with what its reaction
    /// left behind.
    pub shells: Vec<Attended<K>>,

    /// How bash ended. Always its own: the run serves until the subject leaves
    /// of its own accord, whether or not anything went wrong.
    pub subject: ExitStatus,

    /// What went wrong closing up, if anything. It happens after the subject
    /// reached its own end, so `subject` is news of its own either way.
    pub failed: Option<Failure>,
}

impl<K> Run<K> {
    /// The run with its closing-up discharged. Reaching a [`Whole`] means
    /// nothing is left to report.
    pub fn whole(self) -> Result<Whole<K>, Failure> {
        match self.failed {
            Some(why) => Err(why),
            None => Ok(Whole { shells: self.shells, subject: self.subject }),
        }
    }
}

/// A run that closed cleanly.
pub struct Whole<K> {
    pub shells: Vec<Attended<K>>,
    pub subject: ExitStatus,
}

/// A rig whose run Rust orchestrates.
///
/// The command line is run as it is given and carries its own program, so a run
/// is not bound to bash at the top, and a caller wanting a launcher or an
/// environment puts one there — `env VAR=v -- cmd` is the whole story.
/// Instrumentation travels by `BASH_ENV` instead, which is what reaches the
/// shells a command line never could.
///
/// | | |
/// |---|---|
/// | what ends it | a pidfd on the subject, watched and never signalled |
/// | what comes back | [`Run`] — every [`Attended`] shell, the subject's [`ExitStatus`], and what went wrong closing up |
/// | with that discharged | [`Run::whole`] → [`Whole`] |
pub trait Driving: Rig {
    fn run<A: AsRef<OsStr>>(&self, argv: &[A]) -> Result<Run<Kept<Self>>, Failure>
    where
        Self: Sized,
    {
        let mut session = Session::open(self)?;

        // Declared after the session, so it drops before it: leaving through
        // `?` below stops the subject before releasing what it was feeding.
        let mut subject = Subject::spawn(argv, &session.layout.prelude)?;

        session.drive(&Watch::process(subject.pid())?)?;
        let subject = ExitStatus::from(subject.finish().doing(|| "waiting for bash".into())?);
        let (shells, failed) = session.finish();

        Ok(Run { shells, subject, failed })
    }
}

/// The bash the run owns: its process group, and the right to end it.
struct Subject {
    child: Child,
    group: libc::pid_t,
}

impl Subject {
    /// Instrumentation travels by `BASH_ENV`, which any bash the subject starts
    /// will read, whether or not the subject is one itself.
    fn spawn<A: AsRef<OsStr>>(argv: &[A], prelude: &Path) -> Result<Self, Failure> {
        use std::os::unix::process::CommandExt;

        let said =
            || argv.iter().map(|word| word.as_ref().to_string_lossy()).collect::<Vec<_>>().join(" ");
        let (program, rest) = argv
            .split_first()
            .ok_or_else(|| Failure::new("starting the subject", "the command line is empty"))?;

        let mut command = Command::new(program);
        command.args(rest).env("BASH_ENV", prelude).process_group(0);

        let child = command.spawn().doing(|| format!("spawning {}", said()))?;
        let group = child.id() as libc::pid_t;

        Ok(Self { child, group })
    }

    fn pid(&self) -> libc::pid_t {
        self.group
    }

    /// Kill the group, then reap — in that order, because while the subject is
    /// unreaped its group cannot have been recycled. The kill is what collects
    /// anything that outlived the leader, and what ends the subject where a
    /// reaction cut the run short.
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

/// How bash ended. `wait(2)` yields exactly one of these, and both fields are
/// the width the kernel gives them.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ExitStatus {
    Code(u8),
    Signal(u8),
}

impl ExitStatus {
    /// What a shell would report for it: `128 + n` for a signal.
    pub fn shell_code(self) -> i32 {
        match self {
            Self::Code(code) => i32::from(code),
            Self::Signal(signal) => 128 + i32::from(signal),
        }
    }
}

impl fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code(code) => write!(f, "exit {code}"),
            Self::Signal(signal) => write!(f, "killed by signal {signal}"),
        }
    }
}

impl From<std::process::ExitStatus> for ExitStatus {
    /// After `wait(2)` a process has either exited or been signalled, so
    /// reading the two fields out of the raw status is total: `WTERMSIG` is the
    /// low seven bits, `WEXITSTATUS` the second byte, and there is no third
    /// outcome to default to.
    fn from(status: std::process::ExitStatus) -> Self {
        use std::os::unix::process::ExitStatusExt;

        let raw = status.into_raw();
        match status.signal() {
            Some(_) => Self::Signal((raw & 0x7f) as u8),
            None => Self::Code(((raw >> 8) & 0xff) as u8),
        }
    }
}

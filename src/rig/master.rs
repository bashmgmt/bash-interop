//! Rust orchestrates: the run starts the subject, reaches every shell in the
//! tree it creates, and owns its life.

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Child, Command};

use super::serving::{Serving, Until};
use super::{ExitStatus, Rig};
use crate::failure::{Doing, Failure};

/// What a driven run produced.
///
/// Reaching one of these means bash was started and seen out. A `Failure`
/// instead means the run never got that far: it could not be set up, or the
/// rig could not do its work and the subject was killed — and then how the
/// subject would have ended is not something anyone can say.
pub struct Run<S> {
    /// The client's own state, whatever it made of what it heard.
    pub session: S,

    /// How bash ended. Always its own: the run serves until the subject leaves
    /// of its own accord, whether or not anything went wrong.
    pub subject: ExitStatus,

    /// What went wrong closing up, if anything. It happens after the subject
    /// reached its own end, so `subject` is news of its own either way.
    pub failed: Option<Failure>,
}

impl<S> Run<S> {
    /// The session, if nothing went wrong — the shape a caller wants when a
    /// partial reading is no use to it.
    pub fn whole(self) -> Result<(S, ExitStatus), Failure> {
        match self.failed {
            Some(why) => Err(why),
            None => Ok((self.session, self.subject)),
        }
    }
}

/// A rig whose run Rust orchestrates.
///
/// The command line is run as it is given and carries its own program, so a
/// run is not bound to bash at the top, and a caller wanting a launcher or an
/// environment puts one there — `env VAR=v -- cmd` is the whole story.
/// Instrumentation travels by `BASH_ENV` instead, which is what reaches the
/// shells a command line never could.
pub trait Master: Rig {
    fn run<A: AsRef<OsStr>>(&self, argv: &[A]) -> Result<Run<Self::Session>, Failure>
    where
        Self: Sized,
    {
        let mut serving = Serving::lay(self)?;

        // Declared after the serving, so it drops before it: leaving through
        // `?` below stops the subject before releasing what it was feeding.
        let mut subject = Subject::spawn(argv, serving.prelude())?;

        serving.drive(&Until::process(subject.pid())?)?;
        let subject = ExitStatus::from(subject.finish().doing(|| "waiting for bash".into())?);
        let (session, failed) = serving.finish();

        Ok(Run { session, subject, failed })
    }
}

/// The bash the run owns: its process group, and the right to end it.
struct Subject {
    child: Child,
    group: libc::pid_t,
}

impl Subject {
    /// Instrumentation travels by `BASH_ENV`, which any bash the subject
    /// starts will read, whether or not the subject is one itself.
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
    /// anything that outlived the leader, and what ends the subject where the
    /// rig cut the run short.
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

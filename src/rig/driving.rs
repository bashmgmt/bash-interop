//! Rust orchestrates: the run starts the subject, exports the session's
//! address into it, and owns its life.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::Path;
use std::process::{Child, Command};

use tokio::task::LocalSet;

use super::session::Session;
use super::watch::Watch;
use super::{Attended, Kept, Layout, Rig};
use crate::failure::{Doing, Failure};

/// What a driven run produced.
///
/// Reaching one means bash was started and seen out. A `Failure` instead means
/// the run never got that far: it could not be set up, or a reaction could not
/// do its work and the subject was killed.
pub struct Run<K> {
    /// Every shell that joined, in the order they did.
    pub shells: Vec<Attended<K>>,

    /// How bash ended — its own, whether or not anything else went wrong.
    pub subject: ExitStatus,

    /// What went wrong closing up, if anything. After the subject reached its
    /// own end.
    pub failed: Option<Failure>,
}

impl<K> Run<K> {
    /// The run with its closing-up discharged.
    pub fn whole(self) -> Result<Whole<K>, Failure> {
        match self.failed {
            Some(why) => Err(why),
            None => Ok(Whole {
                shells: self.shells,
                subject: self.subject,
            }),
        }
    }
}

/// A run that closed cleanly.
pub struct Whole<K> {
    pub shells: Vec<Attended<K>>,
    pub subject: ExitStatus,
}

/// A rig whose run Rust orchestrates. The impl block is empty: the whole
/// contract is the two provided entries.
///
/// The command line is run as it is given and carries its own program, so a
/// caller wanting a launcher puts one there: `env TARGET=staging -- bash
/// x.bash` is the whole story. `environment` is handed the settled [`Layout`]
/// and its return is the subject's **whole** environment delta — the core
/// adds nothing. Fallible, because provisioning writes a file:
/// [`Layout::bash_env`] with a stated [`Provision`](super::Provision) is the
/// usual pair.
///
/// | | |
/// |---|---|
/// | what reaches the shells | exactly what `environment(&Layout)` returned |
/// | where the session is laid | a directory of the run's own ([`run`](Driving::run)), or the caller's ([`run_at`](Driving::run_at)) |
/// | what ends it | a pidfd on the subject, watched and never signalled; then the group is killed |
/// | what comes back | [`Run`], and [`Run::whole`] → [`Whole`] |
///
/// The future is not `Send`: it runs on a `LocalSet` of its own, and is awaited
/// from a current-thread runtime or `block_on`.
#[expect(async_fn_in_trait, reason = "single-threaded by design: no Send bound")]
pub trait Driving: Rig {
    /// A workspace of the run's own, gone when the run ends.
    async fn run<A, E>(&self, argv: &[A], environment: E) -> Result<Run<Kept<Self>>, Failure>
    where
        A: AsRef<OsStr>,
        E: FnOnce(&Layout) -> Result<Vec<(OsString, OsString)>, Failure>,
        Self: Sized,
    {
        driven(self, None, argv, environment).await
    }

    /// The caller's directory instead — it exists, and is the caller's to
    /// have made — left behind: a reading taken later may follow source
    /// paths into it.
    async fn run_at<A, E>(&self, at: &Path, argv: &[A], environment: E) -> Result<Run<Kept<Self>>, Failure>
    where
        A: AsRef<OsStr>,
        E: FnOnce(&Layout) -> Result<Vec<(OsString, OsString)>, Failure>,
        Self: Sized,
    {
        driven(self, Some(at), argv, environment).await
    }
}

/// The one driven orchestration behind both entries.
async fn driven<R, A, E>(rig: &R, at: Option<&Path>, argv: &[A], environment: E) -> Result<Run<Kept<R>>, Failure>
where
    R: Rig,
    A: AsRef<OsStr>,
    E: FnOnce(&Layout) -> Result<Vec<(OsString, OsString)>, Failure>,
{
    LocalSet::new()
        .run_until(async {
            let mut session = Session::open(rig, at)?;

            // The subject lives inside the block: however it leaves, the
            // group is killed and reaped before the session releases files.
            let subject = async {
                let environment = environment(&session.layout)?;
                let mut subject = Subject::spawn(argv, environment)?;

                session.serve(&Watch::process(subject.pid())?).await?;
                subject.finish().doing(|| "waiting for bash".into())
            }
            .await;
            let (shells, failed) = session.close().await;
            let subject = subject?;

            Ok(Run {
                shells,
                subject: ExitStatus::from(subject),
                failed,
            })
        })
        .await
}

/// The bash the run owns: its process group, and the right to end it.
struct Subject {
    child: Child,
    group: libc::pid_t,
}

impl Subject {
    fn spawn<A: AsRef<OsStr>>(argv: &[A], environment: Vec<(OsString, OsString)>) -> Result<Self, Failure> {
        use std::os::unix::process::CommandExt;

        let said = || {
            argv.iter()
                .map(|word| word.as_ref().to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let (program, rest) = argv.split_first().ok_or_else(|| {
            Failure::new(
                "starting the subject",
                "the command line is empty",
            )
        })?;

        let mut command = Command::new(program);
        command.args(rest).envs(environment).process_group(0);

        let child = command.spawn().doing(|| format!("spawning {}", said()))?;
        let group = child.id() as libc::pid_t;

        Ok(Self { child, group })
    }

    fn pid(&self) -> libc::pid_t {
        self.group
    }

    /// Kill the group, then reap — in that order, because while the subject is
    /// unreaped its group cannot have been recycled.
    fn finish(&mut self) -> io::Result<std::process::ExitStatus> {
        self.release();
        self.child.wait()
    }

    fn release(&self) {
        let _ = unsafe { libc::kill(-self.group, libc::SIGKILL) };
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

/// How bash ended. `wait(2)` yields exactly one of these.
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
    /// `WTERMSIG` is the low seven bits, `WEXITSTATUS` the second byte, and
    /// after `wait(2)` there is no third outcome.
    fn from(status: std::process::ExitStatus) -> Self {
        use std::os::unix::process::ExitStatusExt;

        let raw = status.into_raw();
        match status.signal() {
            Some(_) => Self::Signal((raw & 0x7f) as u8),
            None => Self::Code(((raw >> 8) & 0xff) as u8),
        }
    }
}

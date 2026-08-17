//! The conversation: a workspace with the control fifo in it, one task per
//! shell, and the loop that admits shells until the watch fires.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::attend::{Attendance, attend};
use super::watch::Watch;
use super::wire::{self, Announced, Control, Pipe};
use super::{Attended, Kept, Layout, Rig, Shell};
use crate::failure::{Doing, Failure};

pub(super) struct Session<'r, R: Rig> {
    rig: &'r R,
    pub(super) layout: Layout,
    control: Control,
    attending: JoinSet<Result<Attendance<Kept<R>>, Failure>>,
    closing: watch::Sender<bool>,

    /// The next shell's `nth`.
    joined: usize,
    done: Vec<Attended<Kept<R>>>,

    /// Declared below `control`, so the workspace is held until its fifos
    /// are gone; the kernel releases it on any death.
    _lock: Lock,

    /// Held only to be dropped: it takes the workspace with it, last.
    _temporary: Option<TempDir>,
}

impl<'r, R: Rig> Session<'r, R> {
    /// `at` is a directory the caller prescribed — it exists, and is the
    /// caller's to have made — or nothing, for a workspace of the session's
    /// own that goes when it ends. Either is canonicalised before it is
    /// spelled anywhere: the rig's bash states an absolute path. The
    /// workspace is locked before it is touched and swept of any fifos a
    /// predecessor could not remove; a session already holding it is a
    /// refusal. The session's soft descriptor limit is raised to the hard
    /// one, since every attached shell holds one.
    pub(super) fn open(rig: &'r R, at: Option<&Path>) -> Result<Self, Failure> {
        let (dir, temporary) = match at {
            Some(at) => {
                let dir = fs::canonicalize(at).doing(|| {
                    format!(
                        "opening the prescribed workspace {}",
                        at.display()
                    )
                })?;

                (dir, None)
            }
            None => {
                let temp = tempfile::tempdir().doing(|| "opening a workspace for the run".into())?;
                let dir = fs::canonicalize(temp.path()).doing(|| "opening a workspace for the run".into())?;

                (dir, Some(temp))
            }
        };

        raise_descriptor_limit()?;
        let layout = Layout::new(dir)?;
        let lock = Lock::hold(&layout)?;
        sweep(&layout)?;
        let control = Control::open(layout.clone())?;
        wire::lay(&layout, &rig.bash(&layout))?;
        let (closing, _) = watch::channel(false);

        Ok(Self {
            rig,
            layout,
            control,
            attending: JoinSet::new(),
            closing,
            joined: 0,
            done: Vec::new(),
            _lock: lock,
            _temporary: temporary,
        })
    }

    /// Serve until the watch fires. Everything a shell says is heard by its
    /// own task; this loop admits shells and notices a task that failed — or
    /// a pipe left holding a cut line, which is a fault of the same kind.
    pub(super) async fn serve(&mut self, watch: &Watch) -> Result<(), Failure> {
        loop {
            tokio::select! {
                biased;
                announced = self.control.next() => self.announced(announced?).await?,
                Some(done) = self.attending.join_next() => {
                    let Attendance { attended, cut } = finished(done)?;
                    self.done.push(attended);
                    if let Some(why) = cut {
                        return Err(why);
                    }
                }
                fired = watch.fired() => return fired,
            }
        }
    }

    /// A shell announced on the control fifo. The reply pipe is made before
    /// the shell's pipe is opened, so it exists before the shell is released;
    /// nothing here awaits the shell.
    async fn announced(&mut self, Announced { token, account }: Announced) -> Result<(), Failure> {
        let (up, rep) = (
            self.layout.up(&token),
            self.layout.rep(&token),
        );
        wire::mkfifo(Path::new(&rep))?;
        let pipe = Pipe::open(up.into(), rep.into())?;

        let shell = Arc::new(Shell::of(self.joined, account)?);
        self.joined += 1;
        let reaction = self.rig.joined(&self.layout, Arc::clone(&shell)).await?;

        self.attending.spawn_local(attend(
            shell,
            pipe,
            reaction,
            self.closing.subscribe(),
        ));

        Ok(())
    }

    /// The watch has fired. Everything not yet a shell is released and let go;
    /// every shell reads what its pipe already holds and finishes.
    pub(super) async fn close(mut self) -> (Vec<Attended<Kept<R>>>, Option<Failure>) {
        let mut failed = self.control.close().err();
        let _ = self.closing.send(true);

        while let Some(done) = self.attending.join_next().await {
            match finished(done) {
                Ok(Attendance { attended, cut }) => {
                    self.done.push(attended);
                    failed = failed.or(cut);
                }
                Err(why) => failed = failed.or(Some(why)),
            }
        }
        self.done.sort_by_key(|at| at.shell.nth);

        (self.done, failed)
    }
}

/// A task's outcome. A panic in a reaction is a defect and stays one.
fn finished<K>(done: Result<Result<Attendance<K>, Failure>, tokio::task::JoinError>) -> Result<Attendance<K>, Failure> {
    match done {
        Ok(outcome) => outcome,
        Err(join) => std::panic::resume_unwind(join.into_panic()),
    }
}

/// Exclusive hold on a workspace, `flock`ed: taken before the session
/// touches anything in it, held for the session's life, and released by the
/// kernel on any death — which is what makes the join fifo's presence a
/// truthful liveness signal for anyone probing the directory.
struct Lock {
    _file: fs::File,
}

impl Lock {
    fn hold(at: &Layout) -> Result<Self, Failure> {
        use std::os::fd::AsRawFd;

        let holding = || format!("holding the workspace {}", at.text());
        let file = fs::File::create(at.lock()).doing(holding)?;

        if unsafe {
            libc::flock(
                file.as_raw_fd(),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        } < 0
        {
            let error = std::io::Error::last_os_error();
            return match error.kind() {
                std::io::ErrorKind::WouldBlock => Err(Failure::new(
                    holding(),
                    "it is already held by a live session",
                )),
                _ => Err(error).doing(holding),
            };
        }

        Ok(Self { _file: file })
    }
}

/// The lock is held and nothing is live: any fifo in the workspace is a
/// leaving of a predecessor that could not clean up, and goes.
fn sweep(at: &Layout) -> Result<(), Failure> {
    use std::os::unix::fs::FileTypeExt;

    let sweeping = || format!("sweeping the workspace {}", at.text());
    for entry in fs::read_dir(at.path()).doing(sweeping)? {
        let entry = entry.doing(sweeping)?;
        if entry.file_type().doing(sweeping)?.is_fifo() {
            fs::remove_file(entry.path()).doing(|| {
                format!(
                    "removing the stale fifo {}",
                    entry.path().display()
                )
            })?;
        }
    }

    Ok(())
}

fn raise_descriptor_limit() -> Result<(), Failure> {
    let raising = || "raising the descriptor limit".to_string();
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };

    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } < 0 {
        return Err(std::io::Error::last_os_error()).doing(raising);
    }
    limit.rlim_cur = limit.rlim_max;
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } < 0 {
        return Err(std::io::Error::last_os_error()).doing(raising);
    }
    Ok(())
}

//! The conversation: a workspace with the control fifo in it, one task per
//! shell, and the loop that admits shells until the watch fires.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::attend::{attend, Attendance};
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

    /// Held only to be dropped: it takes the workspace with it, last.
    _temporary: Option<TempDir>,
}

impl<'r, R: Rig> Session<'r, R> {
    /// `at` is a directory the caller prescribed — created if missing, left
    /// behind — or nothing, for a workspace of the session's own that goes
    /// when it ends. Either is canonicalised before it is spelled into the
    /// invocation: the join states an absolute path. The session's soft
    /// descriptor limit is raised to the hard one, since every attached shell
    /// holds one.
    pub(super) fn open(rig: &'r R, at: Option<&Path>) -> Result<Self, Failure> {
        let bash = rig.bash();
        let (at, temporary) = match at {
            Some(at) => (at.to_path_buf(), None),
            None => {
                let temp =
                    tempfile::tempdir().doing(|| "opening a workspace for the run".into())?;

                (temp.path().to_path_buf(), Some(temp))
            }
        };
        let opening = || format!("opening the workspace {}", at.display());

        fs::create_dir_all(&at).doing(opening)?;
        let dir = fs::canonicalize(&at).doing(opening)?;

        raise_descriptor_limit()?;
        let control = Control::open(&dir)?;
        let address = wire::lay(&dir, &bash)?;
        let (closing, _) = watch::channel(false);

        Ok(Self {
            rig,
            layout: Layout { dir, address },
            control,
            attending: JoinSet::new(),
            closing,
            joined: 0,
            done: Vec::new(),
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
        let (up, rep) = (wire::up(&self.layout.dir, &token), wire::rep(&self.layout.dir, &token));
        wire::mkfifo(&rep)?;
        let pipe = Pipe::open(up, rep)?;

        let shell = Arc::new(Shell::of(self.joined, account)?);
        self.joined += 1;
        let reaction = self.rig.joined(&self.layout, Arc::clone(&shell)).await?;

        self.attending.spawn_local(attend(shell, pipe, reaction, self.closing.subscribe()));

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
fn finished<K>(
    done: Result<Result<Attendance<K>, Failure>, tokio::task::JoinError>,
) -> Result<Attendance<K>, Failure> {
    match done {
        Ok(outcome) => outcome,
        Err(join) => std::panic::resume_unwind(join.into_panic()),
    }
}

fn raise_descriptor_limit() -> Result<(), Failure> {
    let raising = || "raising the descriptor limit".to_string();
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };

    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } < 0 {
        return Err(std::io::Error::last_os_error()).doing(raising);
    }
    limit.rlim_cur = limit.rlim_max;
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } < 0 {
        return Err(std::io::Error::last_os_error()).doing(raising);
    }
    Ok(())
}

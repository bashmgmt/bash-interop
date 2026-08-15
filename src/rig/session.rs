//! The conversation itself: a workspace with a wire in it, one reaction per
//! shell, and the loop that serves until nobody can speak any more.
//!
//! What "nobody can speak any more" means is a [`Watch`] — a descriptor the
//! role built. Which shells it stands for, and whether anything is owed to them
//! afterwards, belongs to the role and not here.

use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use tempfile::TempDir;

use super::watch::{wait_for, Ready, Watch};
use super::wire::{prelude, Arrived, Pid, Verb, Wire};
use super::{Attended, Kept, Layout, Reacting, Rig, Shell, Workspace};
use crate::failure::{Doing, Failure};

/// One shell and the reaction built for it. Having one is the whole proof that
/// the shell announced itself: there is no other way to make one.
struct Attending<A> {
    shell: Arc<Shell>,
    reacting: A,
}

/// An open conversation: the workspace is written and the pipe is open. What
/// is missing is who speaks and what ends it.
pub(super) struct Session<'r, R: Rig> {
    rig: &'r R,
    pub(super) layout: Layout,
    wire: Wire,

    shells: Vec<Attending<R::Reaction>>,

    /// The newest shell carrying each pid, which is what a later message from
    /// that pid belongs to. A pid reused across a long run opens a new shell
    /// rather than reopening the first.
    newest: HashMap<Pid, usize>,

    /// Held only to be dropped: it takes the workspace with it, and it goes
    /// last so nothing is reading the files when it does.
    _temporary: Option<TempDir>,
}

impl<'r, R: Rig> Session<'r, R> {
    /// The workspace is canonicalised: every shell reads its own location from
    /// the path it was sourced from, so a relative one would move with the
    /// subject.
    pub(super) fn open(rig: &'r R) -> Result<Self, Failure> {
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

        Ok(Self {
            rig,
            layout: Layout { dir, prelude },
            wire,
            shells: Vec::new(),
            newest: HashMap::new(),
            _temporary: temporary,
        })
    }

    /// Every message the pipe holds, handed to the shell that sent it. An
    /// account of itself makes that shell; everything else needs one to already
    /// exist, and a message from a pid that never announced itself is a fault.
    ///
    /// A shell that asked is blocked until its answer is written, so writing it
    /// is part of delivering rather than something a caller does afterwards.
    fn deliver(&mut self) -> Result<(), Failure> {
        for arrived in self.wire.drain()? {
            match arrived {
                Arrived::Account { pid, stamp, words } => {
                    let shell = Arc::new(Shell::of(self.shells.len(), pid, stamp, &words)?);
                    let reacting = self.rig.joined(&self.layout, shell.clone())?;

                    self.newest.insert(pid, self.shells.len());
                    self.shells.push(Attending { shell, reacting });
                }

                Arrived::Message { pid, message } => {
                    let at = *self.newest.get(&pid).ok_or_else(|| {
                        Failure::new(
                            "placing a message",
                            format!("pid {pid} spoke without ever joining"),
                        )
                    })?;
                    let reacting = &mut self.shells[at].reacting;

                    match message.verb {
                        Verb::Say => reacting.hear(message)?,
                        Verb::Ask => {
                            let answer = reacting.answer(message)?;

                            self.wire.answer(pid, answer)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Serve until nobody can speak any more. There is no interval and no
    /// timer.
    ///
    /// The pipe is polled first, so a message already waiting is read before
    /// the end is noticed, and the delivery behind the loop takes what arrived
    /// with it.
    pub(super) fn drive(&mut self, watch: &Watch) -> Result<(), Failure> {
        while let Ready::Spoke = wait_for(&self.wire, watch)? {
            self.deliver()?;
        }

        self.deliver()
    }

    /// Release what the session holds. A message left half-read is reported
    /// before any reaction is asked to finish, since it is the earlier fault.
    pub(super) fn finish(self) -> (Vec<Attended<Kept<R>>>, Option<Failure>) {
        let Self { shells, wire, .. } = self;
        let mut failed = wire.finish().err();
        let mut done = Vec::with_capacity(shells.len());

        for Attending { shell, reacting } in shells {
            match reacting.finish() {
                Ok(kept) => done.push(Attended { shell, kept }),
                Err(why) => failed = failed.or(Some(why)),
            }
        }

        (done, failed)
    }
}

//! Where a session puts its files, and what a run hands back.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use super::{Message, Micros, Reacting, Rig, Shell};
use crate::failure::{Doing, Failure};

/// The session's workspace: the one coordinate, and the model of the files
/// in it. Construction proves what every user needs: the directory exists
/// (canonical), and is one line of text — it crosses into bash. Handed to
/// every reaction at construction, since the instrument's own frames name a
/// file in here.
#[derive(Clone, Debug)]
pub struct Layout {
    dir: String,
}

/// The provisioned startup file: written only by [`Layout::bash_env`].
const BASH_ENV: &str = "bash_env.bash";
/// The protocol's half, laid verbatim.
const PRELUDE: &str = "prelude.bash";
/// The rig's half, laid from [`super::Rig::bash`].
const RIG: &str = "rig.bash";
/// The control fifo: present exactly while a session serves.
const JOIN: &str = "join";
/// Held `flock`ed for the session's life; the kernel releases it on any death.
const LOCK: &str = "lock";

impl Layout {
    /// `dir` is canonical when handed in; what is proven here is that it can
    /// cross: one line of text.
    pub(super) fn new(dir: PathBuf) -> Result<Self, Failure> {
        let display = dir.display().to_string();
        let dir = dir
            .into_os_string()
            .into_string()
            .ok()
            .filter(|dir| !dir.contains('\n'))
            .ok_or_else(|| {
                Failure::new(
                    format!("opening the workspace {display}"),
                    "the path is not one line of text",
                )
            })?;

        Ok(Self { dir })
    }

    /// The workspace: the session's address.
    pub fn path(&self) -> &Path {
        Path::new(&self.dir)
    }

    /// The workspace as text — what a rig splices into its bash, spelled
    /// through [`bash_strings::emit_scalar`].
    pub fn text(&self) -> &str {
        &self.dir
    }

    pub(crate) fn prelude(&self) -> String {
        self.file(PRELUDE)
    }

    pub(crate) fn rig(&self) -> String {
        self.file(RIG)
    }

    pub(crate) fn join(&self) -> String {
        self.file(JOIN)
    }

    pub(crate) fn lock(&self) -> String {
        self.file(LOCK)
    }

    /// One shell's pipe, made by the shell; the token names it.
    pub(crate) fn up(&self, token: &str) -> String {
        self.file(&format!("up.{token}"))
    }

    /// One shell's reply pipe, made by the run before the shell can ask.
    pub(crate) fn rep(&self, token: &str) -> String {
        self.file(&format!("rep.{token}"))
    }

    fn file(&self, name: &str) -> String {
        format!("{}/{name}", self.dir)
    }

    /// The one owner of `<dir>/bash_env.bash`: writes it — the two sources,
    /// then the joining line iff provisioned — and yields the
    /// `("BASH_ENV", <file>)` pair. Every non-interactive bash in the tree
    /// the subject creates sources that file as it starts; whether that
    /// initiates the channel is `provision`, stated by the caller. The core
    /// consults neither this pair nor any other: a run's environment is
    /// whatever its closure returns.
    pub fn bash_env(&self, provision: Provision<'_>) -> Result<(OsString, OsString), Failure> {
        let file = self.file(BASH_ENV);
        let mut content = format!(
            "source {}\nsource {}\n",
            bash_strings::emit_scalar(&self.prelude()),
            bash_strings::emit_scalar(&self.rig()),
        );
        if let Provision::Joining(line) = provision {
            content.push_str(line);
        }
        std::fs::write(&file, content).doing(|| format!("provisioning {file}"))?;

        Ok((OsString::from("BASH_ENV"), file.into()))
    }
}

/// What the provisioned file does about the channel — the first thing a
/// [`Layout::bash_env`] caller states.
#[derive(Copy, Clone, Debug)]
pub enum Provision<'a> {
    /// The file ends with this line, [`super::Rig::joining`]'s
    /// usually: subjects with no prior knowledge join as their shells start.
    Joining(&'a str),

    /// Definitions only: the client code initiates its own channel, and the
    /// file carries no coordinate — the caller states one beside this pair
    /// if its scripts need it.
    Definitions,
}

/// What one shell's reaction leaves behind, for a given rig.
pub type Kept<R> = <<R as Rig>::Reaction as Reacting>::Kept;

/// One shell, what its reaction left behind, and when it went.
#[derive(Debug)]
pub struct Attended<K> {
    pub shell: Arc<Shell>,
    pub kept: K,

    /// When nobody could write on its pipe any more. `None` for a shell the
    /// session outlived — still running when the watch fired.
    pub parted: Option<Micros>,
}

/// One message, and the shell that sent it.
#[derive(Copy, Clone, Debug, Serialize)]
pub struct Said<'a> {
    pub shell: &'a Arc<Shell>,
    pub message: &'a Message,
}

/// Everything the shells said, in the order it was said: by the sending
/// shell's own clock, stably over join order and each shell's own order.
pub fn heard<K: AsRef<[Message]>>(shells: &[Attended<K>]) -> Vec<Said<'_>> {
    let mut said: Vec<Said<'_>> = shells
        .iter()
        .flat_map(|at| at.kept.as_ref().iter().map(|message| Said { shell: &at.shell, message }))
        .collect();

    said.sort_by_key(|said| said.message.stamp.sent_at);
    said
}

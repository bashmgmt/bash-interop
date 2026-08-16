//! Where a session puts its files, and what a run hands back.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

use super::{Message, Micros, Reacting, Rig, Shell};

/// Where the session's files ended up. Handed to every reaction at
/// construction, since the instrument's own frames name a file in here.
#[derive(Clone, Debug)]
pub struct Layout {
    pub dir: PathBuf,

    /// The session's only address: `<dir>/session.bash`, the file a shell
    /// sources to join, and what `BC_SESSION` carries in a driven subject's
    /// environment. Text, because it crosses into bash and onto the announce
    /// line — validated whole at open. Its dirname is the workspace, so any
    /// shell holding the address knows the coordinate: `${BC_SESSION%/*}`.
    pub address: String,
}

impl Layout {
    /// The address, spelled for `BASH_ENV`: reaches every non-interactive bash
    /// in the tree the subject creates.
    pub fn bash_env(&self) -> (OsString, OsString) {
        (OsString::from("BASH_ENV"), self.address.clone().into())
    }

    /// The address, spelled as the conventional handle a script sources —
    /// `source "$BC_SESSION"` — and both tools' spelling. The core consults
    /// neither this pair nor any other: a run's environment is whatever its
    /// closure returns.
    pub fn bc_session(&self) -> (OsString, OsString) {
        (OsString::from("BC_SESSION"), self.address.clone().into())
    }
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

//! Where a session puts its files, and what a run hands back.
//!
//! One entry per shell, in the order they joined. The provenance is the shape
//! rather than a field: there is no second list to cross-reference and nothing
//! that could disagree with one.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

use super::{Message, Reacting, Rig, Shell};

/// Where a session lays its bash and its pipes, and how long that outlives it.
///
/// A frame's source path is only as readable as the file it names, and the
/// instrument's own frames name a file in here — so anything that reads a walk
/// afterwards has to say where the run put it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Workspace {
    /// A directory of the session's own, removed when it ends.
    #[default]
    Temporary,

    /// One of the caller's, created if it is not there and left behind.
    At(PathBuf),
}

/// Where the session's files ended up. Handed to every reaction at
/// construction, so one that resolves paths afterwards knows where the
/// instrument's own frames point without being told twice.
#[derive(Clone, Debug)]
pub struct Layout {
    pub dir: PathBuf,

    /// The file a shell sources to join — the session's only address.
    pub prelude: PathBuf,
}

/// What one shell's reaction leaves behind, for a given rig.
pub type Kept<R> = <<R as Rig>::Reaction as Reacting>::Kept;

/// One shell, and what its reaction left behind.
#[derive(Debug)]
pub struct Attended<K> {
    pub shell: Arc<Shell>,
    pub kept: K,
}

/// One message, and the shell that sent it.
///
/// A reaction has both by construction. Anything reading a run afterwards needs
/// them together too, since a frame walk means nothing without the shell it was
/// taken in.
#[derive(Copy, Clone, Debug, Serialize)]
pub struct Said<'a> {
    pub shell: &'a Arc<Shell>,
    pub message: &'a Message,
}

/// Everything the shells said, in the order the run heard it.
///
/// A run folds per shell, so each shell's own order is kept and nothing else.
/// [`Stamp::nth`](super::Stamp::nth) counts messages over the whole run and is
/// what puts them back together.
pub fn heard<K: AsRef<[Message]>>(shells: &[Attended<K>]) -> Vec<Said<'_>> {
    let mut said: Vec<Said<'_>> = shells
        .iter()
        .flat_map(|at| at.kept.as_ref().iter().map(|message| Said { shell: &at.shell, message }))
        .collect();

    said.sort_by_key(|said| said.message.stamp.nth);
    said
}

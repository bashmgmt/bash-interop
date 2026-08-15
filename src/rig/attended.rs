//! What a rig states about itself, where a session puts its files, and what a
//! run hands back.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

use super::{Message, Micros, Reacting, Rig, Shell};

/// Everything a rig states up front, in one literal.
#[derive(Clone, Debug)]
pub struct Setup {
    /// The rig's own bash, laid beside the protocol's and sourced by it. Ends
    /// with `BC_JOIN <LABEL>`, which is where the label a client's words use
    /// comes from.
    pub bash: String,

    pub workspace: Workspace,
}

/// Where a session lays its bash and its fifos, and how long that outlives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Workspace {
    /// A directory of the session's own, removed when it ends.
    #[default]
    Temporary,

    /// One of the caller's, created if it is not there and left behind.
    At(PathBuf),
}

/// Where the session's files ended up. Handed to every reaction at
/// construction, since the instrument's own frames name a file in here.
#[derive(Clone, Debug)]
pub struct Layout {
    pub dir: PathBuf,

    /// The file a shell sources to join — the session's only address, and what
    /// `BC_SESSION` carries in a driven subject's environment.
    pub prelude: PathBuf,
}

impl Layout {
    /// The address, spelled for `BASH_ENV`: reaches every non-interactive bash
    /// in the tree the subject creates.
    pub fn bash_env(&self) -> (OsString, OsString) {
        (OsString::from("BASH_ENV"), self.prelude.clone().into_os_string())
    }
}

/// The two usual answers a driving rig gives to [`Driving::environment`](super::Driving::environment).
/// The core consults neither.
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Reaching {
    /// `BASH_ENV` names the address: every non-interactive bash in the
    /// subject's tree joins as it starts.
    BashEnv,

    /// Nothing beyond the address: a shell joins where its script says
    /// `source "$BC_SESSION"`.
    ByHand,
}

impl Reaching {
    pub fn environment(self, at: &Layout) -> Vec<(OsString, OsString)> {
        match self {
            Self::BashEnv => vec![at.bash_env()],
            Self::ByHand => Vec::new(),
        }
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

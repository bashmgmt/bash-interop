//! What a rig is, and what running one means.

pub mod drive;
pub mod turn;

pub use turn::Turn;

use std::fmt;
use std::path::{Path, PathBuf};

use crate::bash::rig::error::RigError;
use crate::bash::rig::source::{Asset, BashSrc};
use crate::bash::rig::wire::{Line, Reply, FRAME_LIMIT};

const WIRE_SRC: Asset = Asset::new("rig/wire.bash");

/// How a run is performed: the description, and the hooks that drive it.
///
/// A rig is never mutated by running — `&self` throughout. Everything that
/// changes belongs to `Session`, a plain struct the rig allocates in
/// [`start`](Rig::start), borrows through the run, and consumes in
/// [`ended`](Rig::ended). Because both are in scope at every hook, a session
/// never has to borrow its description.
pub trait Rig {
    /// What one run needs while it runs.
    type Session;

    /// What a finished run comes to.
    type Output;

    /// How a run is configured, and what it starts with.
    fn start(&self) -> Result<(Setup, Self::Session), RigError>;

    /// `BC_INSTR say`: nothing is waiting, and whether to keep it is yours.
    fn heard(&self, session: &mut Self::Session, said: Line) -> Result<(), RigError>;

    /// `BC_INSTR ask`: what the blocked shell runs next.
    fn answer(&self, session: &mut Self::Session, asked: &Turn) -> Result<Reply, RigError>;

    /// The subject is gone. Release what needs releasing: a failure here is a
    /// failure of the run rather than something dropped on the floor.
    fn ended(&self, session: Self::Session, status: ExitStatus) -> Result<Self::Output, RigError>;

    /// Runs `bash <argv>` until the subject is gone, and does not let it
    /// outlive this call by any route.
    fn run(&self, argv: &[String]) -> Result<Self::Output, RigError>
    where
        Self: Sized,
    {
        drive::run(self, argv)
    }
}

/// The bash every participating shell sources, folded in this order.
pub fn prelude(setup: &Setup, dir: &Path, up: &Path) -> Result<BashSrc, RigError> {
    let quote = |path: &Path| crate::bash::value::emit_scalar(&path.to_string_lossy());

    Ok(BashSrc::seq([
        BashSrc::raw(format!("__BC__UP={}", quote(up))),
        BashSrc::raw(format!("__BC__DIR={}", quote(dir))),
        BashSrc::raw(format!("__BC__limit={FRAME_LIMIT}")),
        BashSrc::raw(format!("__BC__DEBUG={}", if setup.debug { "1" } else { "" })),
        WIRE_SRC.read()?,
        setup.bash.clone(),
    ]))
}

/// How a run starts. A pure value: no I/O happens until the runtime uses it.
#[derive(Clone, Default)]
pub struct Setup {
    pub bash: BashSrc,
    pub env: Vec<(String, String)>,
    pub workspace: Workspace,
    pub debug: bool,
}

impl Setup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bash(mut self, bash: BashSrc) -> Self {
        self.bash = bash;
        self
    }

    /// For the subject's code only; the prelude carries its own config.
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
        self
    }

    pub fn workspace(mut self, workspace: Workspace) -> Self {
        self.workspace = workspace;
        self
    }

    /// Traces the bash side into `debug.log`; pair with [`Workspace::At`].
    pub fn debug(mut self, on: bool) -> Self {
        self.debug = on;
        self
    }
}

/// Where a run keeps its pipe, its prelude, and anything a step wrote.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub enum Workspace {
    #[default]
    Temporary,

    At(PathBuf),
}

/// How the wrapped bash process ended.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ExitStatus {
    Code(i32),
    Signal(i32),
}

impl ExitStatus {
    /// Shell convention: a signalled process reports `128 + signal`.
    pub fn code(self) -> i32 {
        match self {
            Self::Code(code) => code,
            Self::Signal(signal) => 128 + signal,
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
    fn from(status: std::process::ExitStatus) -> Self {
        use std::os::unix::process::ExitStatusExt;

        match (status.code(), status.signal()) {
            (Some(code), _) => Self::Code(code),
            (None, Some(signal)) => Self::Signal(signal),
            (None, None) => Self::Signal(0),
        }
    }
}

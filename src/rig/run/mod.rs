//! What a rig is, and what running one means.
//!
//! [`Rig`] is the whole interface: two functions a tool writes, and three
//! methods derived from them that nothing outside has to reimplement. The
//! rest of this directory is what those methods need — a [`Turn`] to answer
//! against, a [`Report`] to hand back, and the session that drives a run.

pub mod report;
pub mod rigging;
pub mod session;
pub mod turn;

pub use report::Report;
pub use rigging::Rigging;
pub use turn::Turn;

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::bash::rig::capture::Capture;
use crate::bash::rig::error::RigError;
use crate::bash::rig::source::{Asset, BashSrc};
use crate::bash::rig::wire::{FromRecord, Reply, FRAME_LIMIT};

const WIRE_SRC: Asset = Asset::new("rig/wire.bash");

/// What a rig is.
///
/// Two functions: how a run starts, and what a shell that asked runs next.
/// Everything a rig *does* is derived from those and lives here as a method,
/// so nothing outside takes a rig as an argument.
pub trait Rig {
    /// How a run starts.
    fn setup(&self) -> Result<Setup, RigError>;

    /// What the shell runs next. Always an answer; saying needs none, so
    /// there is only this one.
    fn answer(&mut self, turn: &Turn) -> Result<Reply, RigError>;

    /// The bash a subject will source, without running anything.
    fn prelude(&self, dir: &Path, up: &Path) -> Result<BashSrc, RigError> {
        let setup = self.setup()?;
        let quote = |path: &Path| crate::bash::value::emit_scalar(&path.to_string_lossy());

        Ok(BashSrc::seq([
            BashSrc::raw(format!("__BC__UP={}", quote(up))),
            BashSrc::raw(format!("__BC__DIR={}", quote(dir))),
            BashSrc::raw(format!("__BC__limit={FRAME_LIMIT}")),
            BashSrc::raw(format!("__BC__DEBUG={}", if setup.debug { "1" } else { "" })),
            WIRE_SRC.read()?,
            setup.bash,
        ]))
    }

    /// Runs `bash <argv>` under this rig, answering until the subject is
    /// gone. The subject does not outlive this call by either route.
    fn run(&mut self, argv: &[String]) -> Result<Outcome, RigError>
    where
        Self: Sized,
    {
        session::run(self, argv)
    }

    /// Runs `argv` and writes every decoded `T`, with its provenance, as one
    /// JSON object per line. The destination is always given: a wrapper must
    /// not compete for the wrapped program's own output.
    fn capture_into<T>(&mut self, argv: &[String], into: &Path) -> Result<Report, RigError>
    where
        T: FromRecord + Serialize,
        Self: Sized,
    {
        report::capture_into::<T, Self>(self, argv, into)
    }
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

    /// Environment for the subject's own code. The rig's own mechanisms use
    /// none: the prelude carries its configuration baked in.
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
        self
    }

    pub fn workspace(mut self, workspace: Workspace) -> Self {
        self.workspace = workspace;
        self
    }

    /// Traces the bash side into `debug.log` in the workspace, which is worth
    /// pairing with [`Workspace::At`] — a temporary one takes it along.
    pub fn debug(mut self, on: bool) -> Self {
        self.debug = on;
        self
    }
}

/// Where a run keeps its pipe, its prelude, and anything a step wrote.
/// `Temporary` discards them when the run ends; `At` keeps them, which is
/// what you want when something went wrong.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub enum Workspace {
    #[default]
    Temporary,

    At(PathBuf),
}

/// How the wrapped bash process ended. No `Option`: a process that did not
/// exit normally was signalled, and both are reportable.
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

/// Everything a finished run has to say.
#[must_use]
#[derive(Debug)]
pub struct Outcome {
    pub capture: Capture,
    pub status: ExitStatus,
}

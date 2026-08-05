//! What a rig is, and what running one means.

mod drive;
mod plain;
mod turn;

pub use plain::{converse, listen};
pub use turn::Turn;

use std::ffi::OsStr;
use std::fmt;
use std::path::PathBuf;

use crate::bash::rig::error::RigError;
use crate::bash::rig::source::BashSrc;
use crate::bash::rig::wire::{Line, Reply};

pub trait Rig {
    type Session;

    type Output;

    fn start(&self) -> Result<(Setup, Self::Session), RigError>;

    fn heard(&self, session: &mut Self::Session, said: Line) -> Result<(), RigError>;

    fn answer(&self, session: &mut Self::Session, asked: &Turn) -> Result<Reply, RigError>;

    fn ended(&self, session: Self::Session, status: ExitStatus) -> Result<Self::Output, RigError>;

    fn run<S: AsRef<OsStr>>(&self, argv: &[S]) -> Result<Self::Output, RigError>
    where
        Self: Sized,
    {
        drive::run(self, argv)
    }
}

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

    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
        self
    }

    pub fn workspace(mut self, workspace: Workspace) -> Self {
        self.workspace = workspace;
        self
    }

    pub fn debug(mut self, on: bool) -> Self {
        self.debug = on;
        self
    }
}

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub enum Workspace {
    #[default]
    Temporary,

    At(PathBuf),
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ExitStatus {
    Code(i32),
    Signal(i32),
}

impl ExitStatus {
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

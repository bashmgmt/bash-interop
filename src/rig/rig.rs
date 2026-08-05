//! The needle's eye: instruments folded into one bash prelude, evaluated
//! before user code in every participating shell.
//!
//! Three moments and no others. **Setup** writes the prelude. **Speak** is a
//! function the subject calls to ship a message. **Ask** is `BC_INSTR`, where
//! the subject blocks for a continuation. Nothing is injected behind the
//! subject's back, so the rig installs no traps, shadows no builtin, exports
//! nothing, and mutates no global shell state.
//!
//! The prelude is self-reliant: every path it needs is baked into it, so a
//! shell that sources it needs nothing else.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use indexmap::IndexMap;

use super::asset::{Asset, AssetError};
use super::capture::Capture;
use super::control::{self, Reply, Verb};
use super::instrument::{Codegen, Instrument};
use super::src::BashSrc;
use super::wire::{FRAME_LIMIT, Wire};

const WIRE_SRC: Asset = Asset::new("rig/wire.bash");
const CONTROL_SRC: Asset = Asset::new("rig/control.bash");
const SERVICE_INTERVAL: Duration = Duration::from_micros(200);

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

#[must_use]
#[derive(Debug)]
pub struct Outcome {
    pub capture: Capture,
    pub status: ExitStatus,
    /// The bash side's own trace, when the rig was built with `debug(true)`.
    pub debug: Vec<String>,
}

#[derive(Default)]
pub struct Rig {
    instruments: Vec<Box<dyn Instrument>>,
    env: IndexMap<String, String>,
    debug: bool,
}

impl Rig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, instrument: impl Instrument + 'static) -> Self {
        self.instruments.push(Box::new(instrument));
        self
    }

    /// Environment for the client's own code. The rig's own mechanisms use
    /// none: the prelude carries its configuration itself.
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    /// Traces the bash side into `debug.log`, surfaced as `Outcome::debug`.
    pub fn debug(mut self, on: bool) -> Self {
        self.debug = on;
        self
    }

    fn codegen(&self) -> Codegen {
        Codegen::debugging(self.debug)
    }

    pub fn prelude(&self, dir: &Path, up: &Path) -> Result<BashSrc, RigError> {
        let codegen = self.codegen();
        let quote = |path: &Path| crate::bash::value::emit_scalar(&path.to_string_lossy());

        let mut parts = vec![
            BashSrc::raw(format!("__BC__UP={}", quote(up))),
            BashSrc::raw(format!("__BC__DIR={}", quote(dir))),
            BashSrc::raw(format!("__BC__limit={FRAME_LIMIT}")),
            BashSrc::raw(WIRE_SRC.fill(&[("POST", codegen.post_unguarded("__bc_origin").as_str())])?),
            BashSrc::raw(CONTROL_SRC.fill(&[("ASK", codegen.ask("__bc_ask").as_str())])?),
        ];
        parts.extend(self.instruments.iter().map(|one| one.bash(&codegen)));
        Ok(BashSrc::seq(parts))
    }

    /// Runs `bash <argv>` with the prelude injected through `BASH_ENV`, so it
    /// re-runs in every descendant bash process, draining the wire and
    /// answering questions until the child exits.
    ///
    /// Reading stops once the direct child is gone and the pipe holds nothing
    /// more. A writer that outlives the run is not waited for; the
    /// alternative is waiting on an orphan forever.
    pub fn run(&self, argv: &[String]) -> Result<Outcome, RigError> {
        let workspace = tempfile::tempdir().map_err(RigError::Workspace)?;
        let dir = workspace.path();

        let mut wire = Wire::create(dir).map_err(RigError::Pipe)?;
        let prelude_path = dir.join("prelude.bash");
        std::fs::write(&prelude_path, self.prelude(dir, wire.up_path())?.as_str())
            .map_err(|cause| RigError::Prelude { path: prelude_path.clone(), cause })?;

        let verbs: Vec<Verb> = self.instruments.iter().flat_map(|one| one.verbs()).collect();

        let mut command = Command::new("bash");
        command.args(argv).env("BASH_ENV", &prelude_path);
        for (key, value) in &self.env {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(RigError::Spawn)?;
        let status = loop {
            self.serve(&mut wire, &verbs)?;
            if let Some(status) = child.try_wait().map_err(RigError::Wait)? {
                break status;
            }
            std::thread::sleep(SERVICE_INTERVAL);
        };
        self.serve(&mut wire, &verbs)?;

        let debug = std::fs::read_to_string(dir.join("debug.log"))
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default();

        let (lines, damage) = wire.finish();
        Ok(Outcome { capture: Capture::new(lines, damage), status: status.into(), debug })
    }

    fn serve(&self, wire: &mut Wire, verbs: &[Verb]) -> Result<(), RigError> {
        wire.drain().map_err(RigError::Read)?;
        for ask in wire.take_asks() {
            let name = control::verb_of(&ask);
            let reply = match verbs.iter().find(|verb| verb.name == name) {
                Some(verb) => verb.answer(&ask),
                None => Reply::unknown_verb(name),
            };
            wire.answer(ask.stamp.pid, reply).map_err(RigError::Reply)?;
        }
        wire.flush().map_err(RigError::Reply)
    }

}

#[derive(Debug)]
pub enum RigError {
    Workspace(std::io::Error),
    Asset(AssetError),
    Prelude { path: PathBuf, cause: std::io::Error },
    Pipe(std::io::Error),
    Spawn(std::io::Error),
    Wait(std::io::Error),
    Read(std::io::Error),
    Reply(std::io::Error),
}

impl fmt::Display for RigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(cause) => write!(f, "run workspace: {cause}"),
            Self::Asset(cause) => write!(f, "{cause}"),
            Self::Prelude { path, cause } => write!(f, "prelude {}: {cause}", path.display()),
            Self::Pipe(cause) => write!(f, "instrumentation pipe: {cause}"),
            Self::Spawn(cause) => write!(f, "spawn bash: {cause}"),
            Self::Wait(cause) => write!(f, "wait for bash: {cause}"),
            Self::Read(cause) => write!(f, "read the pipe: {cause}"),
            Self::Reply(cause) => write!(f, "answer a question: {cause}"),
        }
    }
}

impl std::error::Error for RigError {}

impl From<AssetError> for RigError {
    fn from(cause: AssetError) -> Self {
        Self::Asset(cause)
    }
}

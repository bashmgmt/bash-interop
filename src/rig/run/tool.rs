//! The shape a capture tool takes.
//!
//! Everything that is the same for all of them — where the output goes, what
//! the exit code is, what happens to records that would not decode — is here
//! once, so a tool is its bash, its record family, and one call.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{ExitStatus, Rig, RigError};
use crate::bash::rig::wire::{Damage, FromRecord};

pub struct Report {
    pub written: usize,
    pub damage: Vec<Damage>,
    pub status: ExitStatus,
}

#[derive(Debug)]
pub enum ToolError {
    Run(RigError),
    Output { path: PathBuf, cause: std::io::Error },
    Encode(serde_json::Error),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Run(cause) => write!(f, "{cause}"),
            Self::Output { path, cause } => write!(f, "{}: {cause}", path.display()),
            Self::Encode(cause) => write!(f, "encode: {cause}"),
        }
    }
}

impl std::error::Error for ToolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Run(cause) => Some(cause),
            Self::Output { cause, .. } => Some(cause),
            Self::Encode(cause) => Some(cause),
        }
    }
}

pub(crate) fn capture_into<T, R>(
    rig: &mut R,
    argv: &[String],
    into: &Path,
) -> Result<Report, ToolError>
where
    T: FromRecord + Serialize,
    R: Rig,
{
    let outcome = rig.run(argv).map_err(ToolError::Run)?;
    let fail = |cause| ToolError::Output { path: into.to_path_buf(), cause };

    let mut sink = fs::File::create(into).map_err(fail)?;
    let mut written = 0;
    for entry in outcome.capture.decoded::<T>() {
        let json = serde_json::to_string(&entry).map_err(ToolError::Encode)?;
        writeln!(sink, "{json}").map_err(fail)?;
        written += 1;
    }

    Ok(Report { written, damage: outcome.capture.damage, status: outcome.status })
}

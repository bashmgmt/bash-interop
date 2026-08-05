//! What a run comes to, once it has been written down.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::Serialize;

use super::{ExitStatus, Rig};
use crate::bash::rig::error::{Doing, RigError};
use crate::bash::rig::wire::FromRecord;

/// What a wrapper has to tell its caller: how much it wrote, and how the
/// wrapped program ended.
pub struct Report {
    pub written: usize,
    pub status: ExitStatus,
}

pub(crate) fn capture_into<T, R>(
    rig: &mut R,
    argv: &[String],
    into: &Path,
) -> Result<Report, RigError>
where
    T: FromRecord + Serialize,
    R: Rig,
{
    let outcome = rig.run(argv)?;
    let writing = || format!("writing {}", into.display());

    let mut sink = fs::File::create(into).doing(writing)?;
    let mut written = 0;
    for entry in outcome.capture.decoded::<T>() {
        let json = serde_json::to_string(&entry).doing(|| "encoding a record".into())?;
        writeln!(sink, "{json}").doing(writing)?;
        written += 1;
    }

    Ok(Report { written, status: outcome.status })
}

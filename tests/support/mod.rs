//! Scratch bash for the proofs, the examples and the command-line tests.
//!
//! [`Scripts`] and [`bash`] are defined in the source tree beside the crate's
//! own test helpers, so a unit test and an integration test build a script the
//! same way.

use std::fs;
use std::path::Path;

use mb_resolver::bash::rig::{Answer, Failure};

#[path = "../../src/tests/scripts.rs"]
mod scripts;

#[allow(unused_imports)]
pub use scripts::{bash, Scripts};

/// Write bash of your own and answer with a command to source it.
pub fn sourcing(path: &Path, body: &str) -> Result<Answer, Failure> {
    fs::write(path, body)
        .map_err(|cause| Failure::new(format!("writing {}", path.display()), cause))?;

    Ok(Answer::of("source", [path.to_string_lossy()]))
}

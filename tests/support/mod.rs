//! Scratch bash for the proofs and the examples.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use mb_resolver::bash::rig::{Answer, Failure};

/// A directory of bash scripts, removed when this is dropped — so it must be
/// held for as long as the run that reads it.
pub struct Scripts(tempfile::TempDir);

impl Scripts {
    /// A fresh directory holding each `(name, body)`.
    pub fn of(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("a scratch directory");
        for (name, body) in files {
            fs::write(dir.path().join(name), body).expect(name);
        }
        Self(dir)
    }

    pub fn dir(&self) -> &Path {
        self.0.path()
    }

    /// Where `name` is, written or not yet.
    pub fn at(&self, name: &str) -> PathBuf {
        self.dir().join(name)
    }
}

/// `bash <script>` — the command line, program included, since a run starts
/// whatever its argv names.
pub fn bash(script: PathBuf) -> Vec<OsString> {
    vec!["bash".into(), script.into()]
}

/// Write bash of your own and answer with a command to source it.
pub fn sourcing(path: &Path, body: &str) -> Result<Answer, Failure> {
    fs::write(path, body)
        .map_err(|cause| Failure::new(format!("writing {}", path.display()), cause))?;

    Ok(Answer::of(["source", &path.to_string_lossy()]))
}

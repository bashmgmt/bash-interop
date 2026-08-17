//! Scratch material for tests built on the rig: a directory of bash scripts,
//! the command line that runs one, an answer that sources a file, and
//! [`accounts`] — shells built by hand where none need run.
//!
//! Deliberately public: a crate driving its own rigs writes the same tests
//! this one does.

pub mod accounts;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::failure::{Doing, Failure};
use crate::rig::Answer;

/// A directory of bash scripts, removed when this is dropped — so it must be
/// held for as long as the run that reads it.
pub struct Scripts(tempfile::TempDir);

impl Scripts {
    /// A fresh directory holding each `(name, body)`. A `name` with a `/`
    /// gets its directories made.
    pub fn of(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("a scratch directory");
        for (name, body) in files {
            let file = dir.path().join(name);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent).expect(name);
            }
            fs::write(file, body).expect(name);
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
    fs::write(path, body).doing(|| format!("writing {}", path.display()))?;

    Ok(Answer::of(
        "source",
        [path.to_string_lossy()],
    ))
}

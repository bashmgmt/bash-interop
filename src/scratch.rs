//! A directory of bash scripts, and the command line that runs one.
//!
//! Reached from the crate's own tests as `crate::tests::scripts`, and from the
//! integration tests through `tests/support/mod.rs`, which includes this file.
//! It names nothing of the crate's, so both readings are the same text.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

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

/// Test logging, initialised once per binary: `RUST_LOG` filters, `info` by
/// default, captured with each test and shown under `--nocapture`.
#[allow(dead_code)] // shared support: each test binary uses its own subset
pub fn logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .is_test(true)
        .try_init();
}

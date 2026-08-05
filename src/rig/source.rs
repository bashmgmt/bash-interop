//! The bash a rig injects: fragments that compose, and the files they come
//! from.
//!
//! Bash lives in `bash/` as real files, read when a prelude is built rather
//! than baked in, so an edit takes effect on the next run. `BC_BASH_DIR`
//! relocates the tree.

use std::fmt;
use std::path::PathBuf;

use crate::bash::rig::error::{Doing, RigError};

const DEFAULT_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/bash");
const ROOT_OVERRIDE: &str = "BC_BASH_DIR";

/// Bash source that composes without quoting hazards.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BashSrc(String);

impl BashSrc {
    pub fn empty() -> Self {
        Self(String::new())
    }

    pub fn raw(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The only way bash contributions combine. Empty parts are dropped.
    pub fn seq(parts: impl IntoIterator<Item = BashSrc>) -> Self {
        Self(
            parts
                .into_iter()
                .filter(|part| !part.is_empty())
                .map(|part| part.0)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Display for BashSrc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A bash file, named relative to the tree root.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Asset(&'static str);

impl Asset {
    pub const fn new(relative: &'static str) -> Self {
        Self(relative)
    }

    pub fn path(self) -> PathBuf {
        root().join(self.0)
    }

    pub fn read(self) -> Result<BashSrc, RigError> {
        let path = self.path();

        std::fs::read_to_string(&path)
            .map(BashSrc::raw)
            .doing(|| format!("reading the bash asset {}", path.display()))
    }
}

fn root() -> PathBuf {
    std::env::var_os(ROOT_OVERRIDE).map_or_else(|| PathBuf::from(DEFAULT_ROOT), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequencing_drops_empties() {
        let joined = BashSrc::seq([BashSrc::raw("one"), BashSrc::empty(), BashSrc::raw("two")]);
        assert_eq!(joined.as_str(), "one\ntwo");
        assert!(BashSrc::seq([BashSrc::empty()]).is_empty());
    }

    /// No asset may export a variable: the prelude configures itself and
    /// leaves the client's environment alone.
    #[test]
    fn assets_read_and_export_nothing() {
        for name in ["rig/wire.bash", "mb/mb.bash", "bashcap/bashcap.bash"] {
            let body = Asset::new(name).read().unwrap_or_else(|error| panic!("{error}"));
            assert!(!body.is_empty(), "{name} is empty");
            for line in body.as_str().lines().filter(|line| !line.trim_start().starts_with('#')) {
                assert!(!line.contains("export "), "{name}: {line}");
            }
        }
    }
}

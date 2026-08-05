//! Bash lives in `bash/` as real files, read when a prelude is built rather
//! than baked in at compile time. Editing one takes effect on the next run.
//!
//! `BC_BASH_DIR` relocates the tree, which is how an installed binary carries
//! its bash somewhere other than the source checkout.

use std::fmt;
use std::path::PathBuf;

use super::BashSrc;

const DEFAULT_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/bash");
const ROOT_OVERRIDE: &str = "BC_BASH_DIR";

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

    pub fn read(self) -> Result<BashSrc, AssetError> {
        std::fs::read_to_string(self.path())
            .map(BashSrc::raw)
            .map_err(|cause| AssetError { asset: self, cause })
    }
}

fn root() -> PathBuf {
    std::env::var_os(ROOT_OVERRIDE).map_or_else(|| PathBuf::from(DEFAULT_ROOT), PathBuf::from)
}

#[derive(Debug)]
pub struct AssetError {
    pub asset: Asset,
    pub cause: std::io::Error,
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bash asset {}: {}", self.asset.path().display(), self.cause)
    }
}

impl std::error::Error for AssetError {}

#[cfg(test)]
mod tests {
    use super::*;

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

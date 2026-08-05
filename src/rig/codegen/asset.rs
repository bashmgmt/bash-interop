//! Bash lives in `bash/` as real files, read when a prelude is built rather
//! than baked in at compile time. Editing one takes effect on the next run.
//!
//! `BC_BASH_DIR` relocates the tree, which is how an installed binary carries
//! its bash somewhere other than the source checkout.

use std::fmt;
use std::path::PathBuf;

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

    pub fn read(self) -> Result<String, AssetError> {
        std::fs::read_to_string(self.path())
            .map_err(|cause| AssetError::Unreadable { asset: self, cause })
    }

    /// Substitutes `@@NAME@@` placeholders. Every placeholder in the file
    /// must be bound and every binding must be used — an asset and its
    /// codegen drifting apart is a mistake, not something to paper over.
    pub fn fill(self, bindings: &[(&str, &str)]) -> Result<String, AssetError> {
        let mut body = self.read()?;
        for (name, value) in bindings {
            let placeholder = format!("@@{name}@@");
            if !body.contains(&placeholder) {
                return Err(AssetError::UnusedBinding { asset: self, name: name.to_string() });
            }
            body = body.replace(&placeholder, value);
        }
        if let Some(rest) = body.split_once("@@") {
            let unbound: String =
                rest.1.chars().take_while(|c| c.is_ascii_uppercase() || *c == '_').collect();
            return Err(AssetError::UnboundPlaceholder { asset: self, name: unbound });
        }
        Ok(body)
    }
}

fn root() -> PathBuf {
    std::env::var_os(ROOT_OVERRIDE).map_or_else(|| PathBuf::from(DEFAULT_ROOT), PathBuf::from)
}

#[derive(Debug)]
pub enum AssetError {
    Unreadable { asset: Asset, cause: std::io::Error },
    UnboundPlaceholder { asset: Asset, name: String },
    UnusedBinding { asset: Asset, name: String },
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { asset, cause } => {
                write!(f, "bash asset {}: {cause}", asset.path().display())
            }
            Self::UnboundPlaceholder { asset, name } => {
                write!(f, "bash asset {}: @@{name}@@ was never bound", asset.0)
            }
            Self::UnusedBinding { asset, name } => {
                write!(f, "bash asset {}: nothing to bind for @@{name}@@", asset.0)
            }
        }
    }
}

impl std::error::Error for AssetError {}

#[cfg(test)]
mod tests {
    use super::*;

    const WIRE: Asset = Asset::new("rig/wire.bash");

    /// No asset may export a variable: the prelude configures itself and
    /// leaves the client's environment alone.
    #[test]
    fn assets_read_and_export_nothing() {
        for name in ["rig/wire.bash", "rig/control.bash", "mb/mb.bash"] {
            let body = Asset::new(name).read().unwrap_or_else(|error| panic!("{error}"));
            assert!(!body.trim().is_empty(), "{name} is empty");
            for line in body.lines().filter(|line| !line.trim_start().starts_with('#')) {
                assert!(!line.contains("export "), "{name}: {line}");
            }
        }
    }

    #[test]
    fn filling_is_exact_in_both_directions() {
        assert!(WIRE.fill(&[("POST", "x")]).is_ok());
        assert!(matches!(WIRE.fill(&[]), Err(AssetError::UnboundPlaceholder { .. })));
        assert!(matches!(
            WIRE.fill(&[("POST", "x"), ("NOPE", "y")]),
            Err(AssetError::UnusedBinding { .. })
        ));
    }
}

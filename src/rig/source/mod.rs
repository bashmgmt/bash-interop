//! The bash a rig injects: fragments that compose, the files they come from,
//! and the prelude they are folded into.

mod asset;
mod prelude;

pub use asset::Asset;
pub use prelude::prelude;

use std::fmt;

/// Bash source that composes without quoting hazards.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BashSrc(String);

impl BashSrc {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequencing_drops_empties() {
        let joined = BashSrc::seq([BashSrc::raw("one"), BashSrc::default(), BashSrc::raw("two")]);
        assert_eq!(joined.as_str(), "one\ntwo");
        assert!(BashSrc::seq([BashSrc::default()]).is_empty());
    }
}

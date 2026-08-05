//! Where a shell came from. An ordinary record — the wire has no special
//! cases — written by the wire itself as the preamble to a shell's first
//! utterance.

use std::fmt;
use std::path::PathBuf;

use crate::bash::rig::wire::{field, FromRecord, Pid, Record};

pub const ORIGIN_TAG: &str = "__ORIGIN__";

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Origin {
    pub parent: Option<Pid>,
    pub shlvl: u32,
    pub source: Option<PathBuf>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MissingField(pub &'static str);

impl fmt::Display for MissingField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing field {:?}", self.0)
    }
}

impl std::error::Error for MissingField {}

impl FromRecord for Origin {
    type Err = MissingField;

    fn from_record(record: &Record) -> Option<Result<Self, Self::Err>> {
        let claimed = record.behind(ORIGIN_TAG)?;
        let set = |key| field(claimed, key).filter(|value| !value.is_empty());
        Some(Ok(Self {
            parent: set("parent").and_then(|value| value.parse().ok()).map(Pid),
            shlvl: match set("shlvl").and_then(|value| value.parse().ok()) {
                Some(shlvl) => shlvl,
                None => return Some(Err(MissingField("shlvl"))),
            },
            source: set("source").map(PathBuf::from),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_decodes_its_own_and_declines_the_rest() {
        let record = Record::new([ORIGIN_TAG, "parent", "", "shlvl", "6", "source", "/x.bash"]);
        let origin = Origin::from_record(&record).unwrap().unwrap();
        assert_eq!(origin.parent, None, "an empty parent is absent, not zero");
        assert_eq!(origin.shlvl, 6);
        assert_eq!(origin.source, Some(PathBuf::from("/x.bash")));

        let short = Record::new([ORIGIN_TAG, "parent", "3"]);
        assert_eq!(Origin::from_record(&short), Some(Err(MissingField("shlvl"))));
        assert!(Origin::from_record(&Record::new(["OTHER"])).is_none());
    }
}

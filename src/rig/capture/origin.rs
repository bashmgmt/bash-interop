//! Where a shell came from. An ordinary record — the wire has no special
//! cases — written by the wire itself as the preamble to a shell's first
//! utterance.

use std::fmt;
use std::path::PathBuf;

use super::super::wire::{FromRecord, Pid, Record};

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
    const TAG: &'static str = ORIGIN_TAG;
    type Err = MissingField;

    fn from_record(record: &Record) -> Result<Self, Self::Err> {
        let optional = |key| record.field(key).filter(|value| !value.is_empty());
        Ok(Self {
            parent: optional("parent").and_then(|value| value.parse().ok()).map(Pid),
            shlvl: optional("shlvl")
                .and_then(|value| value.parse().ok())
                .ok_or(MissingField("shlvl"))?,
            source: optional("source").map(PathBuf::from),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_decodes_and_treats_an_empty_parent_as_absent() {
        let record = Record::new(
            ORIGIN_TAG,
            ["parent".into(), String::new(), "shlvl".into(), "6".into(), "source".into(), "/x.bash".into()],
        );
        let origin = Origin::from_record(&record).unwrap();
        assert_eq!(origin.parent, None);
        assert_eq!(origin.shlvl, 6);
        assert_eq!(origin.source, Some(PathBuf::from("/x.bash")));
    }

    #[test]
    fn origin_requires_shlvl() {
        let record = Record::new(ORIGIN_TAG, ["parent".into(), "3".into()]);
        assert_eq!(Origin::from_record(&record), Err(MissingField("shlvl")));
    }

}

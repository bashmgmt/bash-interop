//! Where a shell came from.

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
pub struct BadField(pub &'static str);

impl fmt::Display for BadField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing or unreadable field {:?}", self.0)
    }
}

impl std::error::Error for BadField {}

impl FromRecord for Origin {
    type Err = BadField;

    fn from_record(record: &Record) -> Option<Result<Self, Self::Err>> {
        Some(Self::decode(record.behind(ORIGIN_TAG)?))
    }
}

impl Origin {
    fn decode(words: &[String]) -> Result<Self, BadField> {
        let set = |key| field(words, key).filter(|value| !value.is_empty());
        let number = |key| match set(key) {
            Some(value) => value.parse().map(Some).map_err(|_| BadField(key)),
            None => Ok(None),
        };

        Ok(Self {
            parent: number("parent")?.map(Pid),
            shlvl: number("shlvl")?.ok_or(BadField("shlvl"))?,
            source: set("source").map(PathBuf::from),
        })
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
        assert_eq!(Origin::from_record(&short), Some(Err(BadField("shlvl"))));

        let wrong = Record::new([ORIGIN_TAG, "parent", "many", "shlvl", "6"]);
        assert_eq!(Origin::from_record(&wrong), Some(Err(BadField("parent"))));

        assert!(Origin::from_record(&Record::new(["OTHER"])).is_none());
    }
}

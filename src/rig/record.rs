//! The message layer.
//!
//! A message is one bash array literal — `( 'tag' 'arg' … )` — which bash
//! reconstructs with `declare -a` and Rust with the `QuotedNest` codec. The
//! same shape travels in both directions, and an element may itself be an
//! array literal, so structure survives the trip without sentinels.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::bash::value::{self, BashCodec, BashVal, QuotedNest, Schema};

/// Microseconds since the Unix epoch, from bash `$EPOCHREALTIME`. Both radix
/// characters are accepted, so no locale has to be forced on the shell.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Micros(pub u64);

impl Micros {
    pub fn parse_epoch(text: &str) -> Option<Self> {
        let (secs, fraction) = text.split_once(['.', ','])?;
        let secs: u64 = secs.parse().ok()?;
        let mut micros = 0u64;
        for position in 0..6 {
            let digit = fraction.as_bytes().get(position).copied().unwrap_or(b'0');
            if !digit.is_ascii_digit() {
                return None;
            }
            micros = micros * 10 + u64::from(digit - b'0');
        }
        Some(Self(secs * 1_000_000 + micros))
    }
}


#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Pid(pub u32);

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Rig-owned provenance, stamped by the sending shell.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Stamp {
    pub at: Micros,
    pub pid: Pid,
    pub seq: u32,
}

impl Stamp {
    pub(crate) fn order(&self) -> (Micros, Pid, u32) {
        (self.at, self.pid, self.seq)
    }
}

/// Tool-owned payload. `args` is opaque to the rig.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Record {
    pub tag: String,
    pub args: Vec<String>,
}

impl Record {
    pub fn new(tag: impl Into<String>, args: impl IntoIterator<Item = String>) -> Self {
        Self { tag: tag.into(), args: args.into_iter().collect() }
    }

    /// Value of the first `key value` pair with this key.
    pub fn field(&self, key: &str) -> Option<&str> {
        self.args.chunks_exact(2).find(|pair| pair[0] == key).map(|pair| pair[1].as_str())
    }

    pub fn parse_message(literal: &str) -> Result<Self, WireError> {
        let value = QuotedNest
            .parse_literal(literal, &Schema::n_d(1))
            .map_err(|cause| WireError::Message(cause.to_string()))?;
        let BashVal::Arr(items) = value else { unreachable!("n_d(1) yields an array") };
        let mut words: Vec<String> = items
            .into_iter()
            .map(|item| match item {
                BashVal::Str(word) => word,
                BashVal::Arr(_) => unreachable!("n_d(1) yields scalars"),
            })
            .collect();
        if words.is_empty() {
            return Err(WireError::Message("empty message".into()));
        }
        let args = words.split_off(1);
        Ok(Self { tag: words.pop().expect("split_off leaves the tag"), args })
    }

    /// Tag first, then the arguments — the message as a flat word list.
    pub fn words(&self) -> Vec<String> {
        std::iter::once(self.tag.clone()).chain(self.args.iter().cloned()).collect()
    }

    pub fn to_message(&self) -> String {
        format!("({})", value::emit_q_words(&self.words()))
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Stamped<T> {
    #[serde(flatten)]
    pub stamp: Stamp,
    #[serde(flatten)]
    pub value: T,
}

pub type Line = Stamped<Record>;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WireError {
    Shape(String),
    Message(String),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shape(what) => write!(f, "malformed frame: {what}"),
            Self::Message(what) => write!(f, "malformed message: {what}"),
        }
    }
}

impl std::error::Error for WireError {}

/// Typed decode of one record family. A tool states its tag once, here,
/// instead of at every filter site.
pub trait FromRecord: Sized {
    const TAG: &'static str;
    type Err;

    fn from_record(record: &Record) -> Result<Self, Self::Err>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A message survives everything bash can put in a word, and nesting
    /// survives because an element may itself be a literal.
    #[test]
    fn messages_round_trip() {
        let record = Record::new(
            "TAG",
            [
                "a space".to_string(),
                "quote'inside".to_string(),
                String::new(),
                "two\nlines".to_string(),
                Record::new("INNER", ["x y".to_string()]).to_message(),
            ],
        );
        let wire = record.to_message();
        assert!(!wire.contains('\n'), "a message is always one line");

        let back = Record::parse_message(&wire).unwrap();
        assert_eq!(back, record);
        assert_eq!(Record::parse_message(&back.args[4]).unwrap().args, ["x y"]);
    }

    #[test]
    fn epochs_and_fields_decode() {
        assert_eq!(Micros::parse_epoch("1785922874.170358"), Some(Micros(1785922874170358)));
        assert_eq!(Micros::parse_epoch("1785922874,170358"), Some(Micros(1785922874170358)));
        assert_eq!(Micros::parse_epoch("nope"), None);

        let record = Record::new("T", ["k".into(), "v".into()]);
        assert_eq!(record.field("k"), Some("v"));
        assert_eq!(record.field("missing"), None);
    }
}

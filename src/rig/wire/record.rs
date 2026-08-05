//! The message layer.
//!
//! A message is one bash array literal — `( 'a' 'b' … )` — which bash
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

/// Rig-owned provenance, stamped by the sending shell. The only thing the rig
/// adds to a message.
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

/// Tool-owned payload: the words the subject passed, in order. The rig reads
/// no position of them and attaches no meaning to any of them.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Record {
    pub words: Vec<String>,
}

impl Record {
    pub fn new(words: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self { words: words.into_iter().map(Into::into).collect() }
    }

    /// The words after `lead`, if that is how they begin.
    ///
    /// A leading discriminator is a common convention and no more than that:
    /// this is the one line a decoder writes to opt into it.
    pub fn behind(&self, lead: &str) -> Option<&[String]> {
        match self.words.split_first() {
            Some((first, rest)) if first == lead => Some(rest),
            _ => None,
        }
    }

    pub fn parse_message(literal: &str) -> Result<Self, WireError> {
        let value = QuotedNest
            .parse_literal(literal, &Schema::n_d(1))
            .map_err(|cause| WireError::Message(cause.to_string()))?;
        let BashVal::Arr(items) = value else { unreachable!("n_d(1) yields an array") };
        let words: Vec<String> = items
            .into_iter()
            .map(|item| match item {
                BashVal::Str(word) => word,
                BashVal::Arr(_) => unreachable!("n_d(1) yields scalars"),
            })
            .collect();
        if words.is_empty() {
            return Err(WireError::Message("empty message".into()));
        }
        Ok(Self { words })
    }

    pub fn to_message(&self) -> String {
        format!("({})", value::emit_q_words(&self.words))
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

/// Typed decode of one record family.
///
/// `None` declines: this record is not of this family. That is what lets
/// several tools share one wire without the rig knowing anything about
/// either. A tool that wants a leading discriminator gets it from
/// [`Record::behind`].
pub trait FromRecord: Sized {
    type Err;

    fn from_record(record: &Record) -> Option<Result<Self, Self::Err>>;
}

/// Value of the first `key value` pair with this key. A convenience for the
/// commonest payload shape, over the words a decoder has already claimed.
pub fn field<'a>(words: &'a [String], key: &str) -> Option<&'a str> {
    words.chunks_exact(2).find(|pair| pair[0] == key).map(|pair| pair[1].as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A message survives everything bash can put in a word, and nesting
    /// survives because an element may itself be a literal.
    #[test]
    fn messages_round_trip() {
        let inner = Record::new(["INNER", "x y"]).to_message();
        let record =
            Record::new(["TAG", "a space", "quote'inside", "", "two\nlines", inner.as_str()]);
        let wire = record.to_message();
        assert!(!wire.contains('\n'), "a message is always one line");

        let back = Record::parse_message(&wire).unwrap();
        assert_eq!(back, record);
        assert_eq!(Record::parse_message(&back.words[5]).unwrap().behind("INNER").unwrap(), ["x y"]);
    }

    #[test]
    fn epochs_fields_and_leads_decode() {
        assert_eq!(Micros::parse_epoch("1785922874.170358"), Some(Micros(1785922874170358)));
        assert_eq!(Micros::parse_epoch("1785922874,170358"), Some(Micros(1785922874170358)));
        assert_eq!(Micros::parse_epoch("nope"), None);

        let record = Record::new(["T", "k", "v"]);
        let claimed = record.behind("T").expect("T is how it begins");
        assert_eq!(record.behind("U"), None);
        assert_eq!(field(claimed, "k"), Some("v"));
        assert_eq!(field(claimed, "missing"), None);
    }
}

//! A message is one bash array literal, the same shape both ways.

use std::fmt;

use crate::bash::rig::error::{Doing, RigError};
use crate::bash::value::{self, BashCodec, BashVal, QuotedNest, Schema};

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
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

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Pid(pub u32);

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
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

pub const ASK_TAG: &str = "__ASK__";

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Record {
    pub words: Vec<String>,
}

impl Record {
    pub fn new(words: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self { words: words.into_iter().map(Into::into).collect() }
    }

    pub fn behind(&self, lead: &str) -> Option<&[String]> {
        match self.words.split_first() {
            Some((first, rest)) if first == lead => Some(rest),
            _ => None,
        }
    }

    pub fn asked(&self) -> Option<&[String]> {
        self.behind(ASK_TAG)
    }

    pub fn parse_message(literal: &str) -> Result<Self, RigError> {
        let value = QuotedNest
            .parse_literal(literal, &Schema::n_d(1))
            .doing(|| format!("reading the message {literal:?}"))?;
        let BashVal::Arr(items) = value else { unreachable!("n_d(1) yields an array") };

        Ok(Self::new(items.into_iter().map(|item| match item {
            BashVal::Str(word) => word,
            BashVal::Arr(_) => unreachable!("n_d(1) yields scalars"),
        })))
    }

    pub fn to_message(&self) -> String {
        literal(&self.words)
    }
}

pub(crate) fn literal(words: &[String]) -> String {
    format!("({})", value::emit_q_words(words))
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Stamped<T> {
    pub stamp: Stamp,
    pub value: T,
}

pub type Line = Stamped<Record>;

pub trait FromRecord: Sized {
    type Err;

    fn from_record(record: &Record) -> Option<Result<Self, Self::Err>>;
}

pub fn field<'a>(words: &'a [String], key: &str) -> Option<&'a str> {
    words.chunks_exact(2).find(|pair| pair[0] == key).map(|pair| pair[1].as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let empty = Record::new(Vec::<String>::new());
        assert_eq!(Record::parse_message(&empty.to_message()).unwrap(), empty);
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

        assert_eq!(record.asked(), None, "a say is not a question");
        assert_eq!(Record::new([ASK_TAG, "at"]).asked(), Some(&["at".to_string()][..]));
    }
}

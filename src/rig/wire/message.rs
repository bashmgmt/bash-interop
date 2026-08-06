//! One message, where it came from, and the one that goes back.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::bash::value::{self, BashCodec, QuotedNest};
use crate::failure::{Doing, Failure};

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Micros(pub u64);

impl Micros {
    pub(crate) fn now() -> Self {
        let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        Self(since.as_micros() as u64)
    }

    /// `$EPOCHREALTIME`: seconds, the locale's decimal separator, and exactly
    /// six digits of microseconds.
    pub(crate) fn parse_epoch(text: &str) -> Option<Self> {
        let (seconds, micros) = text.split_once(['.', ','])?;
        if micros.len() != 6 {
            return None;
        }

        Some(Self(seconds.parse::<u64>().ok()? * 1_000_000 + micros.parse::<u64>().ok()?))
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Pid(pub u32);

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Reserved: it marks a question rather than any payload.
const ASK_TAG: &str = "__ASK__";

/// What one shell said, once, with the provenance the wire gives it.
#[derive(Debug)]
pub struct Line {
    /// The sending shell's `$EPOCHREALTIME` when it wrote the first frame.
    pub sent_at: Micros,

    /// The rig's clock when the last frame of the message arrived.
    pub heard_at: Micros,

    pub pid: Pid,

    /// Counted per shell, from its first message.
    pub seq: u32,

    pub words: Vec<String>,
}

impl Line {
    /// The words after `lead`, if this message begins with it — how a
    /// decoder claims one family of messages and declines the rest.
    pub fn behind(&self, lead: &str) -> Option<&[String]> {
        match self.words.split_first() {
            Some((first, rest)) if first == lead => Some(rest),
            _ => None,
        }
    }

    /// The question a blocked shell asked, if this is one.
    pub fn asked(&self) -> Option<&[String]> {
        self.behind(ASK_TAG)
    }
}

/// Value of the first `key value` pair with this key.
pub fn field<'a>(words: &'a [String], key: &str) -> Option<&'a str> {
    words.chunks_exact(2).find(|pair| pair[0] == key).map(|pair| pair[1].as_str())
}

/// What a blocked shell is told to run next: one command, as an arglist — the
/// same shape a message has, on the same wire, encoded the same way.
#[derive(Debug)]
pub struct Answer(Vec<String>);

impl Answer {
    pub fn of(words: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self(words.into_iter().map(Into::into).collect())
    }

    /// The command `return code`.
    pub fn status(code: u8) -> Self {
        Self::of(["return".to_string(), code.to_string()])
    }

    pub(crate) fn to_message(&self) -> String {
        literal(&self.0)
    }
}

pub(crate) fn parse_message(literal: &str) -> Result<Vec<String>, Failure> {
    QuotedNest.words(literal).doing(|| format!("reading the message {literal:?}"))
}

pub(crate) fn literal(words: &[String]) -> String {
    format!("({})", value::emit_q_words(words))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn messages_round_trip() {
        let nested = literal(&words(&["INNER", "x y"]));
        let sent = words(&["TAG", "a space", "quote'inside", "", "two\nlines", &nested]);

        let wire = literal(&sent);
        assert!(!wire.contains('\n'), "a message is always one line");
        assert_eq!(parse_message(&wire).unwrap(), sent);

        // A word may itself be a message, decoded one level at a time.
        assert_eq!(parse_message(&nested).unwrap(), words(&["INNER", "x y"]));

        assert_eq!(parse_message(&literal(&[])).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn an_epoch_reads_to_the_microsecond() {
        assert_eq!(Micros::parse_epoch("1785922874.170358"), Some(Micros(1785922874170358)));
        assert_eq!(Micros::parse_epoch("1785922874,170358"), Some(Micros(1785922874170358)));
        assert_eq!(Micros::parse_epoch("1785922874.1703"), None, "six digits, as bash prints");
        assert_eq!(Micros::parse_epoch("1785922874.17035x"), None);
        assert_eq!(Micros::parse_epoch("nope"), None);
    }
}

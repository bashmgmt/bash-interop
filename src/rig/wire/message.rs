//! One message, where it came from, and the one that goes back.
//!
//! A message is a bash array literal. The protocol puts its own words in
//! front — the kind, then `key=value` context — and the reader shifts exactly
//! those back off, leaving the client's arglist. Nothing here is needed to
//! reassemble a message; that is `framing`'s concern and it happens first.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use std::vec;

use crate::bash::value::{self, BashCodec, QuotedNest};
use crate::failure::{Doing, Failure};

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Micros(pub u64);

impl Micros {
    /// The run's own clock. A system clock behind the epoch is a broken
    /// machine rather than a zero reading, so it ends the run.
    pub(crate) fn now() -> Result<Self, Failure> {
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .doing(|| "reading the run's clock".into())?;

        Ok(Self(since.as_micros() as u64))
    }

    /// `$EPOCHREALTIME`: seconds, the locale's decimal separator, and exactly
    /// six digits of microseconds.
    fn parse_epoch(text: &str) -> Option<Self> {
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

/// What the protocol says a message is. A word outside this set is a defect
/// in the bash, never a client's choice — a client's own tag is a payload
/// word, and the protocol never reads one.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    Say,
    Ask,
}

impl Kind {
    fn read(word: &str) -> Result<Self, &'static str> {
        match word {
            "SAY" => Ok(Self::Say),
            "ASK" => Ok(Self::Ask),
            _ => Err("unknown kind"),
        }
    }
}

/// What one shell said, once, with the provenance the protocol put in front
/// of it.
#[derive(Debug)]
pub struct Line {
    pub kind: Kind,

    /// The sending shell's `$EPOCHREALTIME`.
    pub sent_at: Micros,

    /// The run's clock when the last frame of the message arrived.
    pub heard_at: Micros,

    pub pid: Pid,

    /// The shell that emitted before this one forked. Not `$PPID`, which
    /// names the grandparent inside a subshell.
    pub parent: Pid,

    pub shlvl: u32,

    /// Counted per shell from its first message, so `0` is a shell that has
    /// just joined.
    pub seq: u32,

    /// The client's arglist, and nothing of the protocol's.
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

    pub(super) fn read(
        pid: Pid,
        seq: u32,
        heard_at: Micros,
        literal: &str,
    ) -> Result<Self, Failure> {
        let at = || format!("reading the message {literal:?}");

        let words = QuotedNest.words(literal).doing(at)?;

        Self::shifted(pid, seq, heard_at, words).doing(at)
    }

    fn shifted(
        pid: Pid,
        seq: u32,
        heard_at: Micros,
        words: Vec<String>,
    ) -> Result<Self, &'static str> {
        let mut ahead = Ahead(words.into_iter());

        let kind = Kind::read(&ahead.word()?)?;
        let sent_at = Micros::parse_epoch(&ahead.header("at")?).ok_or("bad at")?;
        let parent = ahead.header("parent")?.parse().map(Pid).map_err(|_| "bad parent")?;
        let shlvl = ahead.header("shlvl")?.parse().map_err(|_| "bad shlvl")?;

        Ok(Self { kind, sent_at, heard_at, pid, parent, shlvl, seq, words: ahead.rest() })
    }
}

/// The protocol's own words, taken off the front one at a time. `shift`, as
/// the bash that wrote them would do it.
struct Ahead(vec::IntoIter<String>);

impl Ahead {
    fn word(&mut self) -> Result<String, &'static str> {
        self.0.next().ok_or("the message ended early")
    }

    /// One `key=value` header, whose key must be `key`.
    fn header(&mut self, key: &'static str) -> Result<String, &'static str> {
        match self.word()?.split_once('=') {
            Some((found, value)) if found == key => Ok(value.to_string()),
            _ => Err(key),
        }
    }

    fn rest(self) -> Vec<String> {
        self.0.collect()
    }
}

/// Value of the first `key value` pair with this key — a convention clients
/// may write their payload in, unrelated to the `key=value` headers the
/// protocol puts in front of one.
pub fn field<'a>(words: &'a [String], key: &str) -> Option<&'a str> {
    words.chunks_exact(2).find(|pair| pair[0] == key).map(|pair| pair[1].as_str())
}

/// What a blocked shell is told to run next: one command, as an arglist — the
/// same shape a message has, on the same wire, encoded the same way.
#[derive(Debug)]
pub struct Answer(Vec<String>);

impl Answer {
    /// A command and the arguments it is given. The command word stands apart
    /// from them because a command of no words is not one: it would run
    /// nothing and leave the shell holding whatever status it had.
    pub fn of(
        command: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let mut words = vec![command.into()];
        words.extend(args.into_iter().map(Into::into));

        Self(words)
    }

    /// The command `return code`.
    pub fn status(code: u8) -> Self {
        Self::of("return", [code.to_string()])
    }

    pub(crate) fn to_message(&self) -> String {
        literal(&self.0)
    }
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

    fn sent(payload: &[&str]) -> String {
        let mut all = words(&["SAY", "at=1.000002", "parent=7", "shlvl=4"]);
        all.extend(words(payload));

        literal(&all)
    }

    #[test]
    fn the_protocols_words_come_off_and_the_clients_remain() {
        let line = Line::read(Pid(9), 3, Micros(50), &sent(&["REC", "a space", ""])).unwrap();

        assert_eq!(line.kind, Kind::Say);
        assert_eq!(line.sent_at, Micros(1_000_002));
        assert_eq!(line.heard_at, Micros(50), "the run's clock, not the wire's");
        assert_eq!((line.pid, line.parent, line.shlvl, line.seq), (Pid(9), Pid(7), 4, 3));
        assert_eq!(line.words, words(&["REC", "a space", ""]), "the payload alone");
        assert_eq!(line.behind("REC"), Some(words(&["a space", ""]).as_slice()));
    }

    #[test]
    fn a_message_may_carry_nothing_of_its_own() {
        let line = Line::read(Pid(9), 0, Micros(0), &sent(&[])).unwrap();

        assert!(line.words.is_empty());
        assert_eq!(line.behind("REC"), None);
    }

    #[test]
    fn a_header_the_protocol_did_not_write_is_an_error() {
        let bad = [
            literal(&words(&["MUMBLE", "at=1.000002", "parent=7", "shlvl=4"])),
            literal(&words(&["SAY", "when=1.000002", "parent=7", "shlvl=4"])),
            literal(&words(&["SAY", "at=1.0", "parent=7", "shlvl=4"])),
            literal(&words(&["SAY", "at=1.000002", "parent=x", "shlvl=4"])),
            literal(&words(&["SAY", "at=1.000002", "parent=7"])),
            "(unquoted".to_string(),
        ];
        for literal in bad {
            assert!(Line::read(Pid(9), 0, Micros(0), &literal).is_err(), "{literal} should not read");
        }
    }

    /// An answer is one line, and the shell reads it with `read -r` — which
    /// stops at a newline. So a word carrying one has to arrive escaped, and
    /// the delimiter the run appends is the only newline on that pipe.
    #[test]
    fn an_answer_is_one_line_whatever_it_carries() {
        let carried = ["%s", "two\nlines", "a\ttab", "\u{ff}", "it's", ""];
        let message = Answer::of("printf", carried).to_message();

        assert!(!message.contains('\n'), "a raw newline would truncate the read: {message}");

        let mut expected = vec!["printf".to_string()];
        expected.extend(carried.iter().map(|word| word.to_string()));
        assert_eq!(QuotedNest.words(&message).unwrap(), expected, "and it still reads back");
    }

    #[test]
    fn messages_round_trip() {
        let nested = literal(&words(&["INNER", "x y"]));
        let payload = words(&["TAG", "quote'inside", "two\nlines", &nested]);

        let line = Line::read(Pid(1), 0, Micros(0), &sent(&["TAG", "quote'inside", "two\nlines", &nested]))
            .unwrap();
        assert_eq!(line.words, payload, "a message is one line, whatever it carries");

        // A word may itself be a message, decoded one level at a time.
        assert_eq!(QuotedNest.words(&nested).unwrap(), words(&["INNER", "x y"]));
    }
}

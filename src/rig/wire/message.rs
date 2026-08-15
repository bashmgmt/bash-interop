//! One message, where it came from, and the one that goes back.
//!
//! A message is a bash array literal. The protocol puts its own words in
//! front — the kind, then the sending shell's clock — and the reader shifts
//! exactly those back off, leaving what the shell wrote. Reassembly happens
//! first and is `framing`'s concern.
//!
//! What a shell wrote is one of two things and they do not mix: an account of
//! itself, which it gives once, or an arglist of its client's. [`Message`] is only
//! ever the second.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use std::vec;

use serde::{Deserialize, Serialize};

use crate::bash::value::{emit_array, parse_array};
use crate::failure::{Doing, Failure};

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct Micros(pub u64);

impl Micros {
    /// The run's own clock, to sit beside the sending shell's
    /// `$EPOCHREALTIME`.
    pub(crate) fn now() -> Self {
        let since =
            SystemTime::now().duration_since(UNIX_EPOCH).expect("a clock at or after the epoch");

        Self(since.as_micros() as u64)
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

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Pid(pub u32);

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Whether the shell is waiting for something back. A word outside this set is
/// a defect in the bash, never a client's choice — a client's own tag is a
/// payload word, and the protocol never reads one.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verb {
    Say,
    Ask,
}

/// When one message was written and when it arrived.
///
/// Nothing about the *shell* is here. Its pid, its parent and its `$SHLVL` do
/// not change while it lives, so they are what it said once, on joining, and
/// are reached through the shell a reaction was handed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    /// Counted over the whole run, in the order the run read messages. What
    /// puts per-shell foldings back into arrival order.
    pub nth: u64,

    /// Counted per shell from its own account of itself, which is `0`.
    pub seq: u32,

    /// The sending shell's `$EPOCHREALTIME`.
    pub sent_at: Micros,

    /// The run's clock when the last frame of the message arrived. Everything
    /// one read carried shares it.
    pub heard_at: Micros,
}

/// What one shell's client said, once.
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub verb: Verb,
    pub stamp: Stamp,

    /// The client's arglist, and nothing of the protocol's.
    pub words: Vec<String>,
}

impl Message {
    /// The words after `lead`, if this message begins with it — how a decoder
    /// claims one family of messages and declines the rest.
    pub fn behind(&self, lead: &str) -> Option<&[String]> {
        match self.words.split_first() {
            Some((first, rest)) if first == lead => Some(rest),
            _ => None,
        }
    }
}

/// What the frame around a message said, and when the run read it. The sending
/// shell's own clock is a word inside the message and is read there.
#[derive(Copy, Clone, Debug)]
pub(super) struct Envelope {
    pub pid: Pid,

    /// Counted over the whole run, in the order the frames completed.
    pub nth: u64,

    pub seq: u32,
    pub heard_at: Micros,
}

/// What came off the wire. A shell's account of itself is not a [`Message`] and
/// cannot become one: it is what makes a shell, and a `Message` presupposes one.
///
/// `pid` is routing and stops here. What a reaction sees of the sending shell
/// is the shell it was built with.
#[derive(Debug)]
pub(crate) enum Arrived {
    /// A shell's account of itself, which is what makes a shell.
    Account { pid: Pid, stamp: Stamp, words: Vec<String> },

    /// Anything else it said, which presupposes one.
    Message { pid: Pid, message: Message },
}

impl Arrived {
    pub(super) fn read(envelope: Envelope, literal: &str) -> Result<Self, Failure> {
        let at = || format!("reading the message {literal:?}");

        let words = parse_array(literal).doing(at)?;

        Self::shifted(envelope, words).doing(at)
    }

    fn shifted(envelope: Envelope, words: Vec<String>) -> Result<Self, &'static str> {
        let Envelope { pid, nth, seq, heard_at } = envelope;
        let mut ahead = Ahead(words.into_iter());

        let verb = ahead.word()?;
        let sent_at = Micros::parse_epoch(&ahead.header("at")?).ok_or("bad at")?;
        let stamp = Stamp { nth, seq, sent_at, heard_at };

        Ok(match verb.as_str() {
            "JOIN" => Self::Account { pid, stamp, words: ahead.rest() },
            "SAY" => Self::spoke(pid, Verb::Say, stamp, ahead),
            "ASK" => Self::spoke(pid, Verb::Ask, stamp, ahead),
            _ => return Err("unknown verb"),
        })
    }

    fn spoke(pid: Pid, verb: Verb, stamp: Stamp, ahead: Ahead) -> Self {
        Self::Message { pid, message: Message { verb, stamp, words: ahead.rest() } }
    }
}

/// The protocol's own words, taken off the front one at a time. `shift`, as the
/// bash that wrote them would do it.
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

/// Value of the first `key value` pair with this key — a convention clients may
/// write their payload in, unrelated to the `key=value` headers the protocol
/// puts in front of one.
pub fn field<'a>(words: &'a [String], key: &str) -> Option<&'a str> {
    words.chunks_exact(2).find(|pair| pair[0] == key).map(|pair| pair[1].as_str())
}

/// What a blocked shell is told to run next: one command, as an arglist — the
/// same shape a message has, on the same wire, encoded the same way.
#[derive(Debug)]
pub struct Answer(Vec<String>);

impl Answer {
    /// A command and the arguments it is given. The command word stands apart
    /// from them because a command of no words is not one: it would run nothing
    /// and leave the shell holding whatever status it had.
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

    /// A word this rig has no answer for. 127 is bash's own "command not
    /// found", which is the claim being made.
    pub fn unknown() -> Self {
        Self::status(127)
    }

    /// Ran, and nothing to say about it.
    pub fn ok() -> Self {
        Self::status(0)
    }
}

/// One line, whatever it carries: the bash array literal a shell reads back
/// with `declare -a`. It is how an answer travels the reply pipe, and how a
/// session's address travels whatever channel its initiator gave it.
impl fmt::Display for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", emit_array(&self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn envelope() -> Envelope {
        Envelope { pid: Pid(9), nth: 3, seq: 7, heard_at: Micros(50) }
    }

    fn read(kind: &str, payload: &[&str]) -> Result<Arrived, Failure> {
        let mut all = words(&[kind, "at=1.000002"]);
        all.extend(words(payload));

        Arrived::read(envelope(), &emit_array(&all))
    }

    fn spoke(payload: &[&str]) -> Message {
        match read("SAY", payload).expect("a message") {
            Arrived::Message { message, .. } => message,
            other => panic!("not a message: {other:?}"),
        }
    }

    #[test]
    fn the_protocols_words_come_off_and_the_clients_remain() {
        let message = spoke(&["REC", "a space", ""]);

        assert_eq!(message.verb, Verb::Say);
        assert_eq!(message.stamp.sent_at, Micros(1_000_002));
        assert_eq!(message.stamp.heard_at, Micros(50), "the run's clock, not the wire's");
        assert_eq!((message.stamp.nth, message.stamp.seq), (3, 7));
        assert_eq!(message.words, words(&["REC", "a space", ""]), "the payload alone");
        assert_eq!(message.behind("REC"), Some(words(&["a space", ""]).as_slice()));
    }

    #[test]
    fn a_message_may_carry_nothing_of_its_own() {
        let message = spoke(&[]);

        assert!(message.words.is_empty());
        assert_eq!(message.behind("REC"), None);
    }

    /// An account is not a message, and the reader says which it read rather than
    /// handing on a `Message` whose words are the protocol's.
    #[test]
    fn an_account_of_a_shell_is_not_a_line() {
        let read = read("JOIN", &["zero", "x.bash", "flags", "hB"]).expect("an account");

        let Arrived::Account { pid, words, .. } = read else { panic!("read as a message: {read:?}") };
        assert_eq!(pid, Pid(9));
        assert_eq!(field(&words, "zero"), Some("x.bash"));
    }

    #[test]
    fn a_header_the_protocol_did_not_write_is_an_error() {
        let bad = [
            emit_array(&words(&["MUMBLE", "at=1.000002"])),
            emit_array(&words(&["SAY", "when=1.000002"])),
            emit_array(&words(&["SAY", "at=1.0"])),
            emit_array(&words(&["SAY"])),
            "(unquoted".to_string(),
        ];
        for literal in bad {
            assert!(Arrived::read(envelope(), &literal).is_err(), "{literal} should not read");
        }
    }

    /// An answer is one message, and the shell reads it with `read -r` — which
    /// stops at a newline. So a word carrying one has to arrive escaped, and
    /// the delimiter the run appends is the only newline on that pipe.
    #[test]
    fn an_answer_is_one_line_whatever_it_carries() {
        let carried = ["%s", "two\nlines", "a\ttab", "\u{ff}", "it's", ""];
        let message = Answer::of("printf", carried).to_string();

        assert!(!message.contains('\n'), "a raw newline would truncate the read: {message}");

        let mut expected = vec!["printf".to_string()];
        expected.extend(carried.iter().map(|word| word.to_string()));
        assert_eq!(parse_array(&message).unwrap(), expected, "and it still reads back");
    }

    /// A word may itself be a message, decoded one level at a time.
    #[test]
    fn messages_round_trip() {
        let nested = emit_array(&words(&["INNER", "x y"]));
        let message = spoke(&["TAG", "quote'inside", "two\nlines", &nested]);

        assert_eq!(message.words, words(&["TAG", "quote'inside", "two\nlines", &nested]));
        assert_eq!(parse_array(&nested).unwrap(), words(&["INNER", "x y"]));
    }
}

//! What travels: a shell's account of itself, the messages it sends, and the
//! answers it is sent.
//!
//! Each is a bash array literal. The protocol puts its own words in front — a
//! verb where there is one, then the sending shell's clock as `at=` — and the
//! reader shifts exactly those back off, leaving what the shell wrote.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use std::vec;

use serde::{Deserialize, Serialize};

use crate::failure::{Doing, Failure};
use bash_strings::{emit_array, parse_array};

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct Micros(pub u64);

impl Micros {
    /// The run's own clock, to sit beside the sending shell's `$EPOCHREALTIME`.
    pub(crate) fn now() -> Result<Self, Failure> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| Self(since.as_micros() as u64))
            .doing(|| "reading the run's clock".into())
    }

    /// `$EPOCHREALTIME`: seconds, the locale's decimal separator, six digits.
    fn parse_epoch(text: &str) -> Option<Self> {
        let (seconds, micros) = text.split_once(['.', ','])?;
        if micros.len() != 6 {
            return None;
        }

        Some(Self(
            seconds.parse::<u64>().ok()? * 1_000_000 + micros.parse::<u64>().ok()?,
        ))
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
/// a defect in the bash, never a client's choice: a client's own tag is a
/// payload word, and the protocol never reads one.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verb {
    Say,
    Ask,
}

/// When one line was written and when it was read.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    /// The sending shell's `$EPOCHREALTIME`.
    pub sent_at: Micros,

    /// The run's clock at the read that completed the line.
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

    /// One line off a shell's pipe: the verb, the clock, the words.
    pub(crate) fn read(line: Line) -> Result<Self, Failure> {
        let refused = |why: &str| {
            Failure::new(
                format!("reading the line {:?}", line.text),
                why,
            )
        };
        let mut ahead = Ahead::over(&line.text).map_err(|why| refused(&why))?;

        let verb = match ahead.word().map_err(refused)?.as_str() {
            "SAY" => Verb::Say,
            "ASK" => Verb::Ask,
            other => {
                return Err(refused(&format!(
                    "{other} is not a verb"
                )));
            }
        };
        let sent_at = ahead.clock().map_err(refused)?;

        Ok(Self {
            verb,
            stamp: Stamp {
                sent_at,
                heard_at: line.heard_at,
            },
            words: ahead.rest(),
        })
    }
}

/// A shell's account of itself, as it announces itself: the clock, then the
/// pairs [`Shell::of`](crate::shell::Shell::of) reads.
#[derive(Debug)]
pub(crate) struct Account {
    pub stamp: Stamp,
    pub words: Vec<String>,
}

impl Account {
    pub(crate) fn read(text: &str, heard_at: Micros) -> Result<Self, Failure> {
        let refused = |why: &str| {
            Failure::new(
                format!("reading the account {text:?}"),
                why,
            )
        };
        let mut ahead = Ahead::over(text).map_err(|why| refused(&why))?;
        let sent_at = ahead.clock().map_err(refused)?;

        Ok(Self {
            stamp: Stamp { sent_at, heard_at },
            words: ahead.rest(),
        })
    }
}

/// One line as read off a shell's pipe, with the run's clock at the read.
#[derive(Debug)]
pub(crate) struct Line {
    pub text: String,
    pub heard_at: Micros,
}

/// The protocol's own words, taken off the front one at a time.
struct Ahead(vec::IntoIter<String>);

impl Ahead {
    fn over(text: &str) -> Result<Self, String> {
        parse_array(text)
            .map(|words| Self(words.into_iter()))
            .map_err(|why| why.to_string())
    }

    fn word(&mut self) -> Result<String, &'static str> {
        self.0.next().ok_or("the line ended early")
    }

    /// The `at=` header: the sender's `$EPOCHREALTIME`.
    fn clock(&mut self) -> Result<Micros, &'static str> {
        match self.word()?.split_once('=') {
            Some(("at", value)) => Micros::parse_epoch(value).ok_or("bad at"),
            _ => Err("no at= header"),
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
    words
        .chunks_exact(2)
        .find(|pair| pair[0] == key)
        .map(|pair| pair[1].as_str())
}

/// What a blocked shell is told to run next: one command, as an arglist — the
/// same shape a message has, encoded the same way.
#[derive(Debug)]
pub struct Answer(Vec<String>);

impl Answer {
    /// A command and the arguments it is given. The command word stands apart
    /// because a command of no words is not one.
    pub fn of(command: impl Into<String>, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut words = vec![command.into()];
        words.extend(args.into_iter().map(Into::into));

        Self(words)
    }

    /// The command `return code`.
    pub fn status(code: u8) -> Self {
        Self::of("return", [code.to_string()])
    }

    /// A word this rig has no answer for: `return 127`, bash's own "command
    /// not found".
    pub fn unknown() -> Self {
        Self::status(127)
    }
}

/// One line, whatever it carries: the bash array literal a shell reads back
/// with `declare -a` — how an answer travels the reply pipe.
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

    fn line(kind: &str, payload: &[&str]) -> Line {
        let mut all = words(&[kind, "at=1.000002"]);
        all.extend(words(payload));

        Line {
            text: emit_array(&all),
            heard_at: Micros(50),
        }
    }

    fn spoke(payload: &[&str]) -> Message {
        Message::read(line("SAY", payload)).expect("a message")
    }

    #[test]
    fn the_protocols_words_come_off_and_the_clients_remain() {
        let message = spoke(&["REC", "a space", ""]);

        assert_eq!(message.verb, Verb::Say);
        assert_eq!(message.stamp.sent_at, Micros(1_000_002));
        assert_eq!(
            message.stamp.heard_at,
            Micros(50),
            "the run's clock, from the read"
        );
        assert_eq!(
            message.words,
            words(&["REC", "a space", ""]),
            "the payload alone"
        );
        assert_eq!(
            message.behind("REC"),
            Some(words(&["a space", ""]).as_slice())
        );
    }

    #[test]
    fn a_message_may_carry_nothing_of_its_own() {
        let message = spoke(&[]);

        assert!(message.words.is_empty());
        assert_eq!(message.behind("REC"), None);
    }

    /// An account has no verb: the clock comes first.
    #[test]
    fn an_account_is_the_clock_and_the_pairs() {
        let text = emit_array(&words(&[
            "at=1.000002",
            "zero",
            "x.bash",
            "flags",
            "hB",
        ]));
        let account = Account::read(&text, Micros(50)).unwrap();

        assert_eq!(
            account.stamp,
            Stamp {
                sent_at: Micros(1_000_002),
                heard_at: Micros(50)
            }
        );
        assert_eq!(
            field(&account.words, "zero"),
            Some("x.bash")
        );

        let verbed = emit_array(&words(&["JOIN", "at=1.000002"]));
        assert!(
            Account::read(&verbed, Micros(0)).is_err(),
            "a verb where the clock goes"
        );
    }

    #[test]
    fn a_header_the_protocol_did_not_write_is_an_error() {
        let bad = [
            emit_array(&words(&["MUMBLE", "at=1.000002"])),
            emit_array(&words(&["JOIN", "at=1.000002"])),
            emit_array(&words(&["SAY", "when=1.000002"])),
            emit_array(&words(&["SAY", "at=1.0"])),
            emit_array(&words(&["SAY"])),
            "(unquoted".to_string(),
        ];
        for text in bad {
            let refused = Message::read(Line {
                text: text.clone(),
                heard_at: Micros(0),
            });
            assert!(
                refused.is_err(),
                "{text} should not read"
            );
        }
    }

    /// The shell reads an answer with `read -r`, which stops at a newline, so a
    /// word carrying one arrives escaped and the delimiter is the only newline
    /// on that pipe.
    #[test]
    fn an_answer_is_one_line_whatever_it_carries() {
        let carried = ["%s", "two\nlines", "a\ttab", "\u{ff}", "it's", ""];
        let message = Answer::of("printf", carried).to_string();

        assert!(
            !message.contains('\n'),
            "a raw newline would truncate the read: {message}"
        );

        let mut expected = vec!["printf".to_string()];
        expected.extend(carried.iter().map(|word| word.to_string()));
        assert_eq!(
            parse_array(&message).unwrap(),
            expected,
            "and it still reads back"
        );
    }

    /// A word may itself be a message, decoded one level at a time.
    #[test]
    fn messages_round_trip() {
        let nested = emit_array(&words(&["INNER", "x y"]));
        let message = spoke(&["TAG", "quote'inside", "two\nlines", &nested]);

        assert_eq!(
            message.words,
            words(&["TAG", "quote'inside", "two\nlines", &nested])
        );
        assert_eq!(
            parse_array(&nested).unwrap(),
            words(&["INNER", "x y"])
        );
    }
}

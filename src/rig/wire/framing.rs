//! Bytes in, whole messages out.
//!
//! ```text
//! <at> <pid> <seq> <marker> <chunk>\n
//! ```
//!
//! **[`DELIMITER`] separates frames and is part of none of them.** It is
//! appended after a frame is built and consumed before one is parsed, so no
//! frame and no message ever holds it. Both emitters escape one inside a
//! value — bash through `@Q`, Rust through
//! [`emit_q_words`](crate::bash::value::emit_q_words), each rendering it
//! `$'\n'` — so splitting the stream is exact rather than a heuristic, and a
//! frame needs no length prefix.
//!
//! A shared pipe guarantees only that a write of at most `PIPE_BUF` bytes
//! lands whole; one byte past it, concurrent shells interleave. So a message
//! wider than one atomic write arrives as several frames, each newline
//! terminated, and rejoining them is the only reason this layer keeps state.
//! Nothing above it sees a frame.
//!
//! The header exists to route a continuation before there is a message to
//! parse, and does nothing else. Whether a sender is waiting is in the
//! message, at [`Record::asked`](super::Record::asked).

use std::collections::HashMap;

use super::record::{Line, Micros, Pid, Record, Stamp, Stamped};
use crate::bash::rig::error::{Doing, RigError};

/// Below `PIPE_BUF` (4096) with room for the header, so every frame is one
/// atomic write and concurrent shells cannot interleave.
pub const FRAME_LIMIT: usize = 3900;

/// Separates frames. Never inside one.
pub const DELIMITER: char = '\n';

const CONTINUES: &str = "+";
const ENDS: &str = ".";

/// What has arrived but is not yet whole. Empty between messages.
#[derive(Default)]
pub struct Reassembly {
    /// Bytes not yet terminated: a read landed mid-frame.
    pending: String,

    /// Frames without their last, keyed by `(pid, seq)`.
    message: HashMap<(Pid, u32), String>,
}

impl Reassembly {
    /// Every message these bytes completed. A frame is taken up to the
    /// delimiter and the delimiter is dropped, so nothing below this line
    /// ever sees one.
    pub fn feed(&mut self, bytes: &str) -> Result<Vec<Line>, RigError> {
        self.pending.push_str(bytes);
        let mut whole = Vec::new();

        while let Some(end) = self.pending.find(DELIMITER) {
            let frame: String = self.pending.drain(..end).collect();
            self.pending.drain(..DELIMITER.len_utf8());

            if let Some(line) = self.accept(&frame)? {
                whole.push(line);
            }
        }
        Ok(whole)
    }

    /// Errors if a frame lacks its delimiter or a message its last chunk.
    pub fn finish(self) -> Result<(), RigError> {
        let cut = |what: String| Err(RigError::new("draining the instrumentation pipe", what));

        if !self.pending.is_empty() {
            return cut(format!("a frame was cut short: {:?}", self.pending));
        }
        match self.message.into_iter().next() {
            Some(((pid, seq), text)) => cut(format!("message {pid}.{seq} stopped at {text:?}")),
            None => Ok(()),
        }
    }

    fn accept(&mut self, framed: &str) -> Result<Option<Line>, RigError> {
        let frame = Frame::parse(framed)?;
        let key = (frame.stamp.pid, frame.stamp.seq);

        let message = match self.message.remove(&key) {
            Some(head) => head + &frame.chunk,
            None => frame.chunk,
        };
        if frame.continues {
            self.message.insert(key, message);
            return Ok(None);
        }

        Ok(Some(Stamped { stamp: frame.stamp, value: Record::parse_message(&message)? }))
    }
}

struct Frame {
    stamp: Stamp,
    continues: bool,
    chunk: String,
}

impl Frame {
    fn parse(raw: &str) -> Result<Self, RigError> {
        Self::read(raw).doing(|| format!("reading the frame {raw:?}"))
    }

    fn read(raw: &str) -> Result<Self, &'static str> {
        let mut fields = raw.splitn(5, ' ');

        let at = fields.next().and_then(Micros::parse_epoch).ok_or("bad timestamp")?;
        let pid = fields.next().and_then(|raw| raw.parse().ok()).map(Pid).ok_or("bad pid")?;
        let seq = fields.next().and_then(|raw| raw.parse().ok()).ok_or("bad sequence number")?;
        let continues = fields.next().and_then(marker).ok_or("bad marker")?;
        let chunk = fields.next().ok_or("no message")?.to_string();

        Ok(Self { stamp: Stamp { at, pid, seq }, continues, chunk })
    }
}

fn marker(text: &str) -> Option<bool> {
    match text {
        CONTINUES => Some(true),
        ENDS => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &Line) -> Vec<&str> {
        line.value.words.iter().map(String::as_str).collect()
    }

    /// A read boundary lands anywhere, so the same stream fed one byte at a
    /// time must yield exactly what one feed yields.
    #[test]
    fn a_message_survives_any_read_boundary() {
        let stream = "1.000000 7 0 . ('A' 'one')\n1.000001 7 1 . ('B' 'two')\n";

        let whole = Reassembly::default().feed(stream).unwrap();
        assert_eq!(whole.len(), 2);
        assert_eq!(words(&whole[0]), ["A", "one"]);

        let mut dribbled = Reassembly::default();
        let mut seen = Vec::new();
        for byte in stream.chars() {
            seen.extend(dribbled.feed(&byte.to_string()).unwrap());
        }
        assert_eq!(seen.len(), 2);
        assert_eq!(words(&seen[1]), ["B", "two"]);
        dribbled.finish().unwrap();
    }

    /// Split frames rejoin by `(pid, seq)`, so two shells mid-message at once
    /// cannot bleed into each other.
    #[test]
    fn interleaved_split_messages_rejoin_per_shell() {
        let mut incoming = Reassembly::default();

        assert!(incoming.feed("1.000000 7 0 + ('WIDE' 'aa\n").unwrap().is_empty());
        assert!(incoming.feed("1.000001 9 0 + ('OTHER' 'bb\n").unwrap().is_empty());

        let mine = incoming.feed("1.000002 7 0 . aa')\n").unwrap();
        assert_eq!(words(&mine[0]), ["WIDE", "aaaa"]);

        let theirs = incoming.feed("1.000003 9 0 . bb')\n").unwrap();
        assert_eq!(words(&theirs[0]), ["OTHER", "bbbb"]);
        incoming.finish().unwrap();
    }

    #[test]
    fn nothing_may_be_left_part_way() {
        let mut cut = Reassembly::default();
        cut.feed("1.000000 7 0 . ('A')").unwrap();
        assert!(cut.finish().is_err(), "a frame without its delimiter");

        let mut unfinished = Reassembly::default();
        unfinished.feed("1.000000 7 0 + ('A'\n").unwrap();
        assert!(unfinished.finish().is_err(), "a message without its last chunk");

        Reassembly::default().finish().unwrap();
    }

    #[test]
    fn a_frame_that_will_not_parse_is_an_error() {
        for bad in ["nonsense\n", "1.0 x 0 . ()\n", "1.0 1 0 ? ()\n", "1.0 1 0 .\n"] {
            assert!(Reassembly::default().feed(bad).is_err(), "{bad:?} should not parse");
        }
        assert!(Reassembly::default().feed("1.000000 7 0 . (unquoted\n").is_err());
    }
}

//! Bytes in, whole messages out.
//!
//! A frame on the up pipe is `<at> <pid> <seq> <marker> <chunk>` and a
//! delimiter. A reply carries no header — the shell that asked is its only
//! reader — so `pipes` writes a message and a delimiter directly.

use std::collections::HashMap;

use super::message::{self, Line, Micros, Pid};
use crate::failure::{Doing, Failure};

pub const DELIMITER: u8 = b'\n';

const CONTINUES: &str = "+";
const ENDS: &str = ".";

#[derive(Default)]
pub struct Reassembly {
    /// Bytes no delimiter has terminated yet. Bytes rather than text, because
    /// a read boundary falls anywhere, including inside a character.
    bytes: Vec<u8>,

    /// Messages whose last chunk has not arrived, by the `(pid, seq)` their
    /// chunks share.
    partial: HashMap<(Pid, u32), String>,
}

impl Reassembly {
    /// Everything in one read arrived at one moment, which is the `heard_at`
    /// every message completed by it carries.
    pub fn feed(&mut self, bytes: &[u8], heard_at: Micros) -> Result<Vec<Line>, Failure> {
        self.bytes.extend_from_slice(bytes);
        let mut whole = Vec::new();

        while let Some(end) = self.bytes.iter().position(|byte| *byte == DELIMITER) {
            let mut framed: Vec<u8> = self.bytes.drain(..=end).collect();
            framed.pop();

            let text = std::str::from_utf8(&framed)
                .doing(|| format!("reading a frame of {} bytes", framed.len()))?;

            if let Some(line) = self.accept(text, heard_at)? {
                whole.push(line);
            }
        }
        Ok(whole)
    }

    /// Nothing may be left half-read.
    pub fn finish(self) -> Result<(), Failure> {
        let cut = |what: String| Err(Failure::new("draining the instrumentation pipe", what));

        if !self.bytes.is_empty() {
            let text = String::from_utf8_lossy(&self.bytes);
            return cut(format!("a frame was cut short: {text:?}"));
        }
        match self.partial.into_iter().next() {
            Some(((pid, seq), text)) => cut(format!("message {pid}.{seq} stopped at {text:?}")),
            None => Ok(()),
        }
    }

    /// One frame in; a `Line` out once it completed a message.
    fn accept(&mut self, framed: &str, heard_at: Micros) -> Result<Option<Line>, Failure> {
        let frame =
            Frame::read(framed).doing(|| format!("reading the frame {framed:?}"))?;
        let key = (frame.pid, frame.seq);

        let message = match self.partial.remove(&key) {
            Some(head) => head + &frame.chunk,
            None => frame.chunk,
        };
        if frame.continues {
            self.partial.insert(key, message);
            return Ok(None);
        }

        Ok(Some(Line {
            sent_at: frame.sent_at,
            heard_at,
            pid: frame.pid,
            seq: frame.seq,
            words: message::parse_message(&message)?,
        }))
    }
}

struct Frame {
    sent_at: Micros,
    pid: Pid,
    seq: u32,
    continues: bool,
    chunk: String,
}

impl Frame {
    /// `&'static str` for the cause: the frame's own text is attached once,
    /// by the caller.
    fn read(raw: &str) -> Result<Self, &'static str> {
        let mut fields = raw.splitn(5, ' ');

        let sent_at = fields.next().and_then(Micros::parse_epoch).ok_or("bad timestamp")?;
        let pid = fields.next().and_then(|raw| raw.parse().ok()).map(Pid).ok_or("bad pid")?;
        let seq = fields.next().and_then(|raw| raw.parse().ok()).ok_or("bad sequence number")?;
        let continues = match fields.next() {
            Some(CONTINUES) => true,
            Some(ENDS) => false,
            _ => return Err("bad marker"),
        };
        let chunk = fields.next().ok_or("no message")?.to_string();

        Ok(Self { sent_at, pid, seq, continues, chunk })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Micros = Micros(9);

    fn words(line: &Line) -> Vec<&str> {
        line.words.iter().map(String::as_str).collect()
    }

    fn fed(incoming: &mut Reassembly, bytes: &[u8]) -> Vec<Line> {
        incoming.feed(bytes, AT).unwrap()
    }

    #[test]
    fn a_message_survives_any_read_boundary() {
        let stream = "1.000000 7 0 . ('A' 'one')\n1.000001 7 1 . ('B' 'two')\n";

        let whole = fed(&mut Reassembly::default(), stream.as_bytes());
        assert_eq!(whole.len(), 2);
        assert_eq!(words(&whole[0]), ["A", "one"]);
        assert_eq!(whole[0].sent_at, Micros(1_000_000), "the sender's clock, as it wrote it");
        assert_eq!(whole[0].heard_at, AT, "the reader's, as it was handed in");

        let mut dribbled = Reassembly::default();
        let mut seen = Vec::new();
        for byte in stream.as_bytes() {
            seen.extend(fed(&mut dribbled, &[*byte]));
        }
        assert_eq!(seen.len(), 2);
        assert_eq!(words(&seen[1]), ["B", "two"]);
        dribbled.finish().unwrap();
    }

    #[test]
    fn interleaved_split_messages_rejoin_per_sender() {
        let mut incoming = Reassembly::default();

        assert!(fed(&mut incoming, b"1.000000 7 0 + ('WIDE' 'aa\n").is_empty());
        assert!(fed(&mut incoming, b"1.000001 9 0 + ('OTHER' 'bb\n").is_empty());

        let mine = fed(&mut incoming, b"1.000002 7 0 . aa')\n");
        assert_eq!(words(&mine[0]), ["WIDE", "aaaa"]);

        let theirs = fed(&mut incoming, b"1.000003 9 0 . bb')\n");
        assert_eq!(words(&theirs[0]), ["OTHER", "bbbb"]);
        incoming.finish().unwrap();
    }

    #[test]
    fn nothing_may_be_left_part_way() {
        let mut cut = Reassembly::default();
        fed(&mut cut, b"1.000000 7 0 . ('A')");
        assert!(cut.finish().is_err(), "a frame without its delimiter");

        let mut unfinished = Reassembly::default();
        fed(&mut unfinished, b"1.000000 7 0 + ('A'\n");
        assert!(unfinished.finish().is_err(), "a message without its last chunk");

        Reassembly::default().finish().unwrap();
    }

    /// One field wrong per line, in the order `read` takes them.
    #[test]
    fn a_frame_that_will_not_parse_is_an_error() {
        let bad = [
            "nonsense\n",
            "1.0 7 0 . ()\n",
            "1.000000 x 0 . ()\n",
            "1.000000 7 z . ()\n",
            "1.000000 7 0 ? ()\n",
            "1.000000 7 0 .\n",
            "1.000000 7 0 . (unquoted\n",
        ];
        for frame in bad {
            assert!(
                Reassembly::default().feed(frame.as_bytes(), AT).is_err(),
                "{frame:?} should not parse"
            );
        }
    }
}

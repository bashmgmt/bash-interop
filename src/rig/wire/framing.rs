//! Bytes in, whole messages out. `<at> <pid> <seq> <marker> <chunk>\n`

use std::collections::HashMap;

use super::message::{Line, Micros, Pid, Record, Stamp, Stamped};
use crate::bash::rig::error::{Doing, RigError};

pub const FRAME_LIMIT: usize = 3900;

pub const DELIMITER: char = '\n';

const CONTINUES: &str = "+";
const ENDS: &str = ".";

pub fn frame(message: &str) -> String {
    let mut framed = String::with_capacity(message.len() + DELIMITER.len_utf8());
    framed.push_str(message);
    framed.push(DELIMITER);
    framed
}

#[derive(Default)]
pub struct Reassembly {
    pending: String,

    message: HashMap<(Pid, u32), String>,
}

impl Reassembly {
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

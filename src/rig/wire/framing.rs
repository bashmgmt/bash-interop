//! Bytes in, whole messages out.
//!
//! A frame on the up pipe is `<marker> <pid> <seq> <chunk>` and a delimiter.
//! The header carries only what reassembly needs: whether more chunks follow,
//! and the key they share. Everything else about a message is inside it, and
//! is not read until the message is whole.
//!
//! A reply carries no header at all — the shell that asked is its only reader
//! — so `pipes` writes a message and a delimiter directly.

use std::collections::HashMap;

use super::message::{Line, Micros, Pid};
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
    /// chunks share. Bytes for the same reason as above: the sender cuts a
    /// message where the frame fills, which is a byte and may be inside a
    /// character.
    partial: HashMap<(Pid, u32), Vec<u8>>,
}

impl Reassembly {
    /// Everything in one read arrived at one moment, which is the `heard_at`
    /// every message completed by it carries.
    ///
    /// The buffer is cut once, after every frame in it has been found. Taking
    /// them off the front one at a time would rescan and move what is left
    /// behind each of them, which is quadratic in the frames one read carries
    /// — and a busy read carries hundreds.
    pub fn feed(&mut self, bytes: &[u8], heard_at: Micros) -> Result<Vec<Line>, Failure> {
        self.bytes.extend_from_slice(bytes);

        let mut framed: Vec<Vec<u8>> = Vec::new();
        let mut cut = 0;
        while let Some(offset) = self.bytes[cut..].iter().position(|byte| *byte == DELIMITER) {
            let end = cut + offset;

            framed.push(self.bytes[cut..end].to_vec());
            cut = end + 1;
        }
        self.bytes.drain(..cut);

        let mut whole = Vec::new();
        for frame in &framed {
            if let Some(line) = self.accept(frame, heard_at)? {
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
            Some(((pid, seq), bytes)) => {
                let text = String::from_utf8_lossy(&bytes);
                cut(format!("message {pid}.{seq} stopped at {text:?}"))
            }
            None => Ok(()),
        }
    }

    /// One frame in; a `Line` out once it completed a message.
    ///
    /// A message is decoded when its last chunk arrives, never before: the
    /// sender cuts where the frame fills, so a character can span two of them.
    fn accept(&mut self, framed: &[u8], heard_at: Micros) -> Result<Option<Line>, Failure> {
        let shown = || format!("reading the frame {:?}", String::from_utf8_lossy(framed));
        let frame = Frame::read(framed).doing(shown)?;
        let key = (frame.pid, frame.seq);

        let mut message = self.partial.remove(&key).unwrap_or_default();
        message.extend_from_slice(frame.chunk);

        if frame.continues {
            self.partial.insert(key, message);
            return Ok(None);
        }

        let (pid, seq) = key;
        let text = String::from_utf8(message)
            .doing(|| format!("reading message {pid}.{seq} as text"))?;

        Line::read(pid, seq, heard_at, &text).map(Some)
    }
}

/// A frame's header, and the bytes behind it. The header is the protocol's own
/// and is always ASCII; the chunk is whatever the subject wrote.
struct Frame<'a> {
    continues: bool,
    pid: Pid,
    seq: u32,
    chunk: &'a [u8],
}

impl<'a> Frame<'a> {
    /// `&'static str` for the cause: the frame's own text is attached once,
    /// by the caller.
    fn read(raw: &'a [u8]) -> Result<Self, &'static str> {
        let mut fields = raw.splitn(4, |byte| *byte == b' ');
        let number = |field: Option<&[u8]>| {
            field.and_then(|raw| std::str::from_utf8(raw).ok()).and_then(|raw| raw.parse().ok())
        };

        let continues = match fields.next() {
            Some(marker) if marker == CONTINUES.as_bytes() => true,
            Some(marker) if marker == ENDS.as_bytes() => false,
            _ => return Err("bad marker"),
        };
        let pid = number(fields.next()).map(Pid).ok_or("bad pid")?;
        let seq = number(fields.next()).ok_or("bad sequence number")?;
        let chunk = fields.next().ok_or("no message")?;

        Ok(Self { continues, pid, seq, chunk })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::bash::value::emit_array;

    const AT: Micros = Micros(9);

    /// A whole message, as the bash writes one: the protocol's words in
    /// front, then the client's.
    fn whole(payload: &[&str]) -> String {
        let ahead = ["SAY", "at=1.000002", "parent=7", "shlvl=4"];
        let words: Vec<String> =
            ahead.iter().chain(payload).map(|word| word.to_string()).collect();

        emit_array(&words)
    }

    fn frame(pid: u32, seq: u32, body: &str) -> String {
        format!(". {pid} {seq} {body}\n")
    }

    fn words(line: &Line) -> Vec<&str> {
        line.words.iter().map(String::as_str).collect()
    }

    fn fed(incoming: &mut Reassembly, bytes: &[u8]) -> Vec<Line> {
        incoming.feed(bytes, AT).unwrap()
    }

    #[test]
    fn a_message_survives_any_read_boundary() {
        let stream = frame(7, 0, &whole(&["A", "one"])) + &frame(7, 1, &whole(&["B", "two"]));

        let seen = fed(&mut Reassembly::default(), stream.as_bytes());
        assert_eq!(seen.len(), 2);
        assert_eq!(words(&seen[0]), ["A", "one"]);
        assert_eq!(seen[0].sent_at, Micros(1_000_002), "the sender's clock, from the message");
        assert_eq!(seen[0].heard_at, AT, "the reader's, as it was handed in");
        assert_eq!((seen[0].pid, seen[0].seq), (Pid(7), 0), "from the frame header");

        let mut dribbled = Reassembly::default();
        let mut byte_at_a_time = Vec::new();
        for byte in stream.as_bytes() {
            byte_at_a_time.extend(fed(&mut dribbled, &[*byte]));
        }
        assert_eq!(byte_at_a_time.len(), 2);
        assert_eq!(words(&byte_at_a_time[1]), ["B", "two"]);
        dribbled.finish().unwrap();
    }

    #[test]
    fn interleaved_split_messages_rejoin_per_sender() {
        let (mine, theirs) = (whole(&["WIDE", "aaaa"]), whole(&["OTHER", "bbbb"]));
        let cut = 12;
        let mut incoming = Reassembly::default();

        assert!(fed(&mut incoming, format!("+ 7 0 {}\n", &mine[..cut]).as_bytes()).is_empty());
        assert!(fed(&mut incoming, format!("+ 9 0 {}\n", &theirs[..cut]).as_bytes()).is_empty());

        let rejoined = fed(&mut incoming, format!(". 7 0 {}\n", &mine[cut..]).as_bytes());
        assert_eq!(words(&rejoined[0]), ["WIDE", "aaaa"]);

        let other = fed(&mut incoming, format!(". 9 0 {}\n", &theirs[cut..]).as_bytes());
        assert_eq!(words(&other[0]), ["OTHER", "bbbb"]);
        incoming.finish().unwrap();
    }

    #[test]
    fn nothing_may_be_left_part_way() {
        let mut cut = Reassembly::default();
        let no_delimiter = frame(7, 0, &whole(&["A"]));
        fed(&mut cut, no_delimiter.trim_end().as_bytes());
        assert!(cut.finish().is_err(), "a frame without its delimiter");

        let mut unfinished = Reassembly::default();
        fed(&mut unfinished, b"+ 7 0 ('SAY'\n");
        assert!(unfinished.finish().is_err(), "a message without its last chunk");

        Reassembly::default().finish().unwrap();
    }

    /// One thing wrong per line: first the frame header, in the order `read`
    /// takes it, then the message the whole frame carries.
    #[test]
    fn a_frame_that_will_not_read_is_an_error() {
        let bad = [
            "nonsense\n".to_string(),
            format!("? 7 0 {}\n", whole(&[])),
            format!(". x 0 {}\n", whole(&[])),
            format!(". 7 z {}\n", whole(&[])),
            ". 7 0\n".to_string(),
            ". 7 0 (unquoted\n".to_string(),
            format!(". 7 0 {}\n", emit_array(&["MUMBLE".to_string()])),
        ];
        for frame in bad {
            assert!(
                Reassembly::default().feed(frame.as_bytes(), AT).is_err(),
                "{frame:?} should not read"
            );
        }
    }
}

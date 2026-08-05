//! The transport layer: one line, one frame.
//!
//! ```text
//! <at> <pid> <seq> <kind> <chunk>
//! ```
//!
//! The header sits outside the message because a continuation has to be
//! routed before there is a message to parse. `kind` carries whether the
//! sender is waiting, so the obligation to reply is a property of the parsed
//! type rather than something inferred from the payload.

use super::record::{Micros, Pid, Stamp, WireError};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// More chunks follow.
    Continues,

    /// Last chunk. Fire and forget.
    Post,

    /// Last chunk. The sender is blocked on its reply pipe.
    Ask,
}

impl Kind {
    pub fn marker(self) -> char {
        match self {
            Self::Continues => '+',
            Self::Post => '.',
            Self::Ask => '?',
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "+" => Some(Self::Continues),
            "." => Some(Self::Post),
            "?" => Some(Self::Ask),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub stamp: Stamp,
    pub kind: Kind,
    pub chunk: String,
}

impl Frame {
    pub fn parse(raw: &str) -> Result<Self, WireError> {
        let mut fields = raw.splitn(5, ' ');
        let shape = |what: &str| WireError::Shape(what.to_string());

        let at = fields.next().and_then(Micros::parse_epoch).ok_or_else(|| shape("timestamp"))?;
        let pid =
            fields.next().and_then(|raw| raw.parse().ok()).map(Pid).ok_or_else(|| shape("pid"))?;
        let seq = fields.next().and_then(|raw| raw.parse().ok()).ok_or_else(|| shape("seq"))?;
        let kind = fields.next().and_then(Kind::parse).ok_or_else(|| shape("kind"))?;
        let chunk = fields.next().ok_or_else(|| shape("missing message"))?.to_string();

        Ok(Self { stamp: Stamp { at, pid, seq }, kind, chunk })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_shapes() {
        let frame = Frame::parse("1.000000 42 7 ? ('a')").unwrap();
        assert_eq!(frame.kind, Kind::Ask);
        assert_eq!(frame.stamp.pid, Pid(42));
        assert_eq!(frame.chunk, "('a')");

        assert_eq!(Frame::parse("1.000000 42 7 . ()").unwrap().kind, Kind::Post);
        assert_eq!(Frame::parse("1.000000 42 7 + (").unwrap().kind, Kind::Continues);

        for bad in ["", "x 1 0 . ()", "1.0 x 0 . ()", "1.0 1 x . ()", "1.0 1 0 @ ()", "1.0 1 0 ."] {
            assert!(Frame::parse(bad).is_err(), "{bad:?} should not parse");
        }
    }
}

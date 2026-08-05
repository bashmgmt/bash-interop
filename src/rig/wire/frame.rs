//! The transport layer: one line, one frame.
//!
//! ```text
//! <at> <pid> <seq> <marker> <chunk>
//! ```
//!
//! The header sits outside the message because a continuation has to be
//! routed before there is a message to parse — which is the only thing the
//! header is for. Whether the sender is waiting for an answer is in the
//! message, where [`Record::asked`](super::Record::asked) reads it, so the
//! frame layer knows nothing about what a message means.

use super::record::{Micros, Pid, Stamp};
use crate::bash::rig::error::{Doing, RigError};

const CONTINUES: &str = "+";
const ENDS: &str = ".";

#[derive(Clone, Debug)]
pub struct Frame {
    pub stamp: Stamp,

    /// Further chunks of this message follow, to be rejoined by `(pid, seq)`.
    pub partial: bool,

    pub chunk: String,
}

impl Frame {
    pub fn parse(raw: &str) -> Result<Self, RigError> {
        Self::read(raw).doing(|| format!("reading the frame {raw:?}"))
    }

    fn read(raw: &str) -> Result<Self, &'static str> {
        let mut fields = raw.splitn(5, ' ');

        let at = fields.next().and_then(Micros::parse_epoch).ok_or("bad timestamp")?;
        let pid = fields.next().and_then(|raw| raw.parse().ok()).map(Pid).ok_or("bad pid")?;
        let seq = fields.next().and_then(|raw| raw.parse().ok()).ok_or("bad sequence number")?;
        let partial = fields.next().and_then(continues).ok_or("bad marker")?;
        let chunk = fields.next().ok_or("no message")?.to_string();

        Ok(Self { stamp: Stamp { at, pid, seq }, partial, chunk })
    }
}

fn continues(marker: &str) -> Option<bool> {
    match marker {
        CONTINUES => Some(true),
        ENDS => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_and_shapes() {
        let frame = Frame::parse("1.000000 42 7 . ('a')").unwrap();
        assert!(!frame.partial);
        assert_eq!(frame.stamp.pid, Pid(42));
        assert_eq!(frame.chunk, "('a')");

        assert!(Frame::parse("1.000000 42 7 + (").unwrap().partial);

        for bad in ["", "x 1 0 . ()", "1.0 x 0 . ()", "1.0 1 x . ()", "1.0 1 0 ? ()", "1.0 1 0 ."] {
            assert!(Frame::parse(bad).is_err(), "{bad:?} should not parse");
        }
    }
}

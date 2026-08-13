//! A walk: the frames one instrument reported, innermost first.

use std::iter::once;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use super::Frame;

/// A walk, innermost first. Never empty: the frame it was taken in is always
/// one of them, and a walk that reaches no frame is refused where it is read.
///
/// One array in JSON, and one field wherever an instrument reports where it
/// was. Which frame is the call site is [`at`](Stack::at), not a second field
/// beside the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    at: Frame,
    outer: Vec<Frame>,
}

impl Stack {
    /// `None` for no frames at all, which is not a walk.
    pub fn of(frames: Vec<Frame>) -> Option<Self> {
        let mut frames = frames.into_iter();

        Some(Self { at: frames.next()?, outer: frames.collect() })
    }

    /// The frame the walk was taken in.
    pub fn at(&self) -> &Frame {
        &self.at
    }

    /// The frames above it, outermost last.
    pub fn outer(&self) -> &[Frame] {
        &self.outer
    }

    pub fn frames(&self) -> impl Iterator<Item = &Frame> {
        once(&self.at).chain(&self.outer)
    }
}

impl Serialize for Stack {
    fn serialize<S: Serializer>(&self, into: S) -> Result<S::Ok, S::Error> {
        into.collect_seq(self.frames())
    }
}

impl<'de> Deserialize<'de> for Stack {
    fn deserialize<D: Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        Stack::of(Vec::deserialize(from)?)
            .ok_or_else(|| de::Error::custom("a call stack with no frames"))
    }
}


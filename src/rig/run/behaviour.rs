//! What a rig does.
//!
//! Complete at construction: the bash a subject gets, and what it is told when
//! it asks. A tool ships as one of these.
//!
//! Bash composes — `BashSrc::seq` — so several tools' contributions combine
//! into one behaviour's `speak`. Answers do not: there is one control channel
//! and one conversation, so a behaviour has exactly one answer, and it is
//! total. The types say which is which.

use crate::bash::rig::capture::Capture;
use crate::bash::rig::source::BashSrc;
use crate::bash::rig::wire::{Ask, Reply};

/// What a shell that asks is told when the behaviour has nothing to say to it.
pub const UNANSWERED: i32 = 127;

type Answer = Box<dyn Fn(&Ask, &Capture) -> Reply + Send + Sync>;

pub struct Behaviour {
    speak: BashSrc,
    answer: Answer,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self::new()
    }
}

impl Behaviour {
    /// Says nothing, and tells anything that asks [`UNANSWERED`].
    pub fn new() -> Self {
        Self {
            speak: BashSrc::empty(),
            answer: Box::new(|_, _| Reply::Continue { status: UNANSWERED }),
        }
    }

    pub fn speaking(mut self, bash: BashSrc) -> Self {
        self.speak = bash;
        self
    }

    /// One function, always given an answer to give: it sees the question and
    /// everything the run has recorded, so a controlled session has nowhere
    /// else for its state to live and needs nowhere else.
    pub fn answering(
        mut self,
        answer: impl Fn(&Ask, &Capture) -> Reply + Send + Sync + 'static,
    ) -> Self {
        self.answer = Box::new(answer);
        self
    }

    pub fn bash(&self) -> &BashSrc {
        &self.speak
    }

    pub fn reply(&self, ask: &Ask, seen: &Capture) -> Reply {
        (self.answer)(ask, seen)
    }
}

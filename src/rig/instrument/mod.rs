//! A mechanism, as a value.
//!
//! Two contribution points, matching the two things a subject can do: speak
//! (a function that ships a message) and ask (a verb that answers one). An
//! instrument is data, so a one-off one is an expression rather than a type.

pub mod dispatch;
pub mod repl;

pub use dispatch::Dispatch;
pub use repl::{repl, Ran, Turn};

use super::capture::Capture;
use super::codegen::{BashSrc, Codegen};
use super::wire::{Ask, Reply};

pub struct Instrument {
    pub name: String,
    bash: Box<dyn Fn(&Codegen) -> BashSrc + Send + Sync>,
    verbs: Vec<Verb>,
}

impl Instrument {
    pub fn new(
        name: impl Into<String>,
        bash: impl Fn(&Codegen) -> BashSrc + Send + Sync + 'static,
    ) -> Self {
        Self { name: name.into(), bash: Box::new(bash), verbs: Vec::new() }
    }

    /// An instrument that is nothing but bash source.
    pub fn text(name: impl Into<String>, src: impl Into<String>) -> Self {
        let src = src.into();
        Self::new(name, move |_| BashSrc::raw(src.clone()))
    }

    pub fn answering(mut self, verb: Verb) -> Self {
        self.verbs.push(verb);
        self
    }

    pub fn render(&self, codegen: &Codegen) -> BashSrc {
        (self.bash)(codegen)
    }

    pub fn verbs(&self) -> &[Verb] {
        &self.verbs
    }
}

/// A named handler for one kind of question. It sees the question and
/// everything the run has recorded so far, so the whole of a controlled
/// session is one function of `(&Ask, &Capture)` — there is nowhere else for
/// its state to live, and nowhere else it needs to.
pub struct Verb {
    pub name: String,
    handle: Box<dyn Fn(&Ask, &Capture) -> Reply + Send + Sync>,
}

impl Verb {
    pub fn new(
        name: impl Into<String>,
        handle: impl Fn(&Ask, &Capture) -> Reply + Send + Sync + 'static,
    ) -> Self {
        Self { name: name.into(), handle: Box::new(handle) }
    }

    pub fn answer(&self, ask: &Ask, seen: &Capture) -> Reply {
        (self.handle)(ask, seen)
    }
}

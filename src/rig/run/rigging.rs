//! A rig assembled from parts rather than declared as a type.

use super::{Rig, Setup, Turn};
use crate::bash::rig::wire::Reply;

type Answer = Box<dyn FnMut(&Turn) -> Reply + Send>;

/// For a rig that is not worth naming. The closure is `FnMut`, so it owns its
/// state the same way an implementation owns its fields.
pub struct Rigging {
    setup: Setup,
    answer: Answer,
}

impl Rigging {
    pub fn new(setup: Setup, answer: impl FnMut(&Turn) -> Reply + Send + 'static) -> Self {
        Self { setup, answer: Box::new(answer) }
    }
}

impl Rig for Rigging {
    fn setup(&self) -> Setup {
        self.setup.clone()
    }

    fn answer(&mut self, turn: &Turn) -> Reply {
        (self.answer)(turn)
    }
}

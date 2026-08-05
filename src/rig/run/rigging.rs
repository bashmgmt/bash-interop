//! A rig assembled from parts rather than declared as a type.

use super::{Rig, Setup, Turn};
use crate::bash::rig::error::RigError;
use crate::bash::rig::wire::Reply;

type Answering = Result<Reply, RigError>;

/// For a rig not worth naming. `FnMut`, so the closure owns its state.
pub struct Rigging {
    setup: Setup,
    answer: Box<dyn FnMut(&Turn) -> Answering>,
}

impl Rigging {
    pub fn new(setup: Setup, answer: impl FnMut(&Turn) -> Answering + 'static) -> Self {
        Self { setup, answer: Box::new(answer) }
    }
}

impl Rig for Rigging {
    fn setup(&self) -> Result<Setup, RigError> {
        Ok(self.setup.clone())
    }

    fn answer(&mut self, turn: &Turn) -> Answering {
        (self.answer)(turn)
    }
}

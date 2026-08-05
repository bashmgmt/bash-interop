//! A bash contribution, as a value.
//!
//! An instrument is named bash and nothing else, so a one-off one is an
//! expression rather than a type. How questions are answered belongs to the
//! [`Rig`](crate::bash::rig::Rig), which has exactly one answer for all of
//! them.

pub mod dispatch;

pub use dispatch::Dispatch;

use crate::bash::rig::codegen::{BashSrc, Codegen};

pub struct Instrument {
    pub name: String,
    bash: Box<dyn Fn(&Codegen) -> BashSrc + Send + Sync>,
}

impl Instrument {
    pub fn new(
        name: impl Into<String>,
        bash: impl Fn(&Codegen) -> BashSrc + Send + Sync + 'static,
    ) -> Self {
        Self { name: name.into(), bash: Box::new(bash) }
    }

    /// An instrument that is nothing but bash source.
    pub fn text(name: impl Into<String>, src: impl Into<String>) -> Self {
        let src = src.into();
        Self::new(name, move |_| BashSrc::raw(src.clone()))
    }

    pub fn render(&self, codegen: &Codegen) -> BashSrc {
        (self.bash)(codegen)
    }
}

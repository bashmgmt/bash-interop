//! A bash call stack, as an instrument records it — both halves.
//!
//! `stack.bash` ships bash's five parallel arrays as they are; [`Columns`]
//! puts them back together. What comes out is a [`Stack`]: the frames one
//! instrument reported, innermost first, never empty.

mod columns;
mod frame;

pub use columns::{Args, Columns};
pub use frame::{Frame, Site, Source, Stack};

/// `__bc_stack`, which ships the five arrays. Private: an instrument reaches
/// it through [`with_walk`], which is what puts it in the right place.
const BASH: &str = include_str!("stack.bash");

/// The frame walk, and a tool's own bash after it — the order the shells
/// source them in.
///
/// `__bc_stack` has to be defined before anything calls it, so every
/// instrument that reports a walk is built here rather than by joining strings
/// at each tool.
pub fn with_walk(bash: &[&str]) -> String {
    let mut all = vec![BASH];
    all.extend_from_slice(bash);

    all.join("\n")
}

#[cfg(test)]
mod tests;

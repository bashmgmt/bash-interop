pub mod rig;
pub mod stack;
pub mod value;

/// The instrument that records a call stack, sourced by any tool that wants
/// one. `bash::stack` is the other half.
pub const STACK: &str = include_str!("stack.bash");

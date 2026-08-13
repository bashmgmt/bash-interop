//! Talking to bash: three layers, and nothing above them.
//!
//! | | | |
//! |---|---|---|
//! | [`value`] | bash's own quoted forms — `@Q`, `@A`, `declare -p` | depends on nothing |
//! | [`stack`] | a call stack, as an instrument records it and as Rust reads it | on `value` |
//! | [`rig`] | run a bash program, hear every shell in it, answer what they ask | on `value` |
//!
//! `stack` and `rig` are siblings: neither knows the other. A tool composes
//! them — the walk goes into the bash a rig injects, through
//! [`stack::with`] — which is what [`bashcap`](crate::bashcap) and
//! [`bashprof`](crate::bashprof) are.

pub mod rig;
pub mod stack;
pub mod value;

//! Talking to bash: three layers, and nothing above them.
//!
//! | | | |
//! |---|---|---|
//! | [`value`] | bash's own quoted forms — `@Q`, `@A`, `declare -p` | depends on nothing |
//! | [`shell`] | which bash a shell is, how it was started, what it has on | on `value` |
//! | [`stack`] | a call stack, as an instrument records it and as Rust reads it | on `value`, `shell` |
//! | [`rig`] | run a bash program, hear every shell in it, answer what they ask | on `value`, `shell` |
//!
//! `stack` and `rig` are siblings: neither knows the other. A tool composes
//! them — the walk goes into the bash a rig injects, through
//! [`stack::with_walk`] — which is what [`bashcap`](crate::bashcap) and
//! [`bashprof`](crate::bashprof) are.
//!
//! Both read a walk against the shell it was taken in, which is why `shell`
//! sits under both: bash writes `$0` into `BASH_SOURCE` for code it was given
//! rather than read, and no walk can say on its own which word that is.

pub mod rig;
pub mod shell;
pub mod stack;
pub mod value;

/// What the protocol and the walk bring into a shell — the entry words, the
/// guards, the walk — and bash meant to be read the same with a tool and
/// without it may therefore not say. That is what lets a tool ship the words
/// a call site says as one file, injected and vendored, with only their effect
/// existing twice: the words name a hook, and the hook is where these appear.
pub const INJECTED_NAMES: [&str; 5] =
    ["BC_JOIN", "BC_INSTR", "__BC_BAIL", "__BC_THROW", "__bc_stack"];

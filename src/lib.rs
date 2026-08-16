//! Run bash under instrumentation, and hear what it says.
//!
//! | | | |
//! |---|---|---|
//! | [`shell`] | which bash a shell is, how it was started, what it has on | on `bash_strings` |
//! | [`stack`] | a call stack, as an instrument records it and as Rust reads it | on `shell` |
//! | [`rig`] | run a bash program, hear every shell in it, answer what they ask | on `shell` |
//! | [`failure`] | one error for anything that stops work | on nothing |
//! | [`scratch`] | a directory of bash scripts, and helpers a test builds shells from | |
//!
//! Values travel as bash's own quoted forms — the `bash-strings` crate.
//! `stack` and `rig` are siblings: neither knows the other. A tool composes
//! them — the walk goes into the bash a rig injects, through
//! [`stack::with_walk`] — which is what `bashcap` and `bashprof` are.
//!
//! Both read a walk against the shell it was taken in, which is why `shell`
//! sits under both: bash writes `$0` into `BASH_SOURCE` for code it was given
//! rather than read, and no walk can say on its own which word that is.
//!
//! Start with [`rig`]'s module doc; the book is `docs/` — `docs/overview.md`
//! in this crate's repository.

pub mod failure;
pub mod rig;
pub mod scratch;
pub mod shell;
pub mod stack;

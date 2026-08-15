//! The account a shell gives of itself, built by hand.
//!
//! One builder, so a test that needs the words and a test that needs the shell
//! read the same one. The numbers are a real 5.3.9's: what is under test
//! wherever this is used is the arrangement or the walk, never the reading of
//! this payload — that is tested against a live shell.

use std::sync::Arc;

use crate::bash::rig::{Micros, Pid, Shell, Stamp};
use crate::bash::value::emit_array;

/// The words a `JOIN` message carries. `zero` is the shell's `$0` and `flags`
/// its `$-`, which between them decide how a walk taken in it reads.
pub fn account(zero: &str, flags: &str) -> Vec<String> {
    let versinfo =
        emit_array(&["5", "3", "9", "1", "release", "x86_64-pc-linux-gnu"].map(String::from));
    let command = if flags.contains('c') { "true" } else { "" };

    [
        "parent", "1",
        "shlvl", "5",
        "subshell", "0",
        "versinfo", versinfo.as_str(),
        "bash", "/usr/bin/bash",
        "zero", zero,
        "flags", flags,
        "shellopts", "braceexpand:hashall",
        "bashopts", "checkwinsize",
        "command", command,
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

/// A shell, as one would arrive.
pub fn shell(nth: usize, pid: u32, zero: &str, flags: &str) -> Arc<Shell> {
    let joined = Stamp { nth: 0, seq: 0, sent_at: Micros(100), heard_at: Micros(101) };

    Arc::new(Shell::of(nth, Pid(pid), joined, &account(zero, flags)).expect("an account"))
}

/// A shell bash was handed a file to read.
pub fn reading(zero: &str) -> Arc<Shell> {
    shell(0, 7, zero, "hB")
}

/// A shell bash was given its code directly, where `$0` is a word and not a
/// path.
pub fn given(zero: &str) -> Arc<Shell> {
    shell(0, 7, zero, "hBc")
}

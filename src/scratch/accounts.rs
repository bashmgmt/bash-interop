//! The account a shell gives of itself, built by hand.
//!
//! One builder, so a test that needs the words and a test that needs the value
//! read the same shell. The numbers are a real 5.3.9's: what is under test
//! wherever this is used is the arrangement or the walk, never the reading of
//! this payload — that is tested against a live shell.

use crate::bash::shell::Bash;
use crate::bash::value::emit_array;

/// The words a `JOIN` message carries. `zero` is the shell's `$0` and `flags`
/// its `$-`, which between them decide how a walk taken in it reads.
pub fn account(zero: &str, flags: &str) -> Vec<String> {
    let versinfo =
        emit_array(&["5", "3", "9", "1", "release", "x86_64-pc-linux-gnu"].map(String::from));
    let command = if flags.contains('c') { "true" } else { "" };

    [
        "versinfo",
        versinfo.as_str(),
        "bash",
        "/usr/bin/bash",
        "zero",
        zero,
        "flags",
        flags,
        "shellopts",
        "braceexpand:hashall",
        "bashopts",
        "checkwinsize",
        "command",
        command,
        "subshell",
        "0",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

/// A shell bash was handed a file to read.
pub fn reading(zero: &str) -> Bash {
    Bash::of(&account(zero, "hB")).expect("an account of a shell reading a file")
}

/// A shell bash was given its code directly, where `$0` is a word and not a
/// path.
pub fn given(zero: &str) -> Bash {
    Bash::of(&account(zero, "hBc")).expect("an account of a shell given its code")
}

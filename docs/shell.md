# The shell

`src/shell.rs`.

```rust
pub struct Shell {
    pub nth: usize,        // the order it joined in, counting from zero
    pub pid: Pid,
    pub shlvl: u32,
    pub subshell: u32,     // $BASH_SUBSHELL
    pub joined: Stamp,     // when it joined, on both clocks
    pub bash: Bash,        // which bash, and how it was invoked
    pub options: Options,  // what it had switched on then, which may change after
    pub brought: Vec<String>, // the words its join carried, verbatim
}

pub struct Bash { pub version: Version, pub binary: PathBuf, pub zero: String, pub invocation: Invocation }
pub struct Invocation { pub command: Option<String>, pub standard_input: bool, pub interactive: bool }
pub struct Options { pub flags: Flags, pub shellopts: Vec<String>, pub bashopts: Vec<String> }
```

A shell opens with an account of itself, stated by the shell rather than
inferred from what it went on to write. The account travels with the
announcement, on the control fifo, in frames, before the shell's pipe is
opened, so the run knows the whole of it before releasing the shell. The
account is what makes a shell, and a `Message` presupposes one, so the account
is never a message.

One prelude function builds it, and reads as the checklist of what a shell
states about itself. These are the shipped bytes:

```bash
__bc_account() {
    local __bc_out=$1 IFS=' '
    local -a __bc_meta="(${__BC__META[$2]-})"
    set -- "at=$EPOCHREALTIME" \
        pid       "$BASHPID" \
        shlvl     "$SHLVL" \
        subshell  "$BASH_SUBSHELL" \
        versinfo  "(${BASH_VERSINFO[*]@Q})" \
        bash      "$BASH" \
        zero      "$0" \
        flags     "$-" \
        shellopts "$SHELLOPTS" \
        bashopts  "$BASHOPTS" \
        command   "${BASH_EXECUTION_STRING-}" \
        brought   "(${__bc_meta[*]@Q})"
    printf -v "$__bc_out" '(%s)' "${*@Q}"
}
```

One array literal, clock first and no verb, written into the caller's local.
Every entry is passed as bash reports it, and `Shell::of` decides what any of
it means. Adding a fact is a word here and a field there.

`brought` is the entry the client writes: the words its join carried, from
`BC_JOIN LABEL DIR word…`, as one nested literal in the shape `versinfo`
takes, landing verbatim on `Shell::brought`. It is an arglist like a
message's. The protocol reserves no word in it, and `key value` pairs read
with `field` are the client's own convention. A fork's reattach announces its
label's words, and a child process re-derives them at its own join.

None of this changes while a shell lives. A subshell has a `$BASHPID` of its
own and joins as a shell of its own, and `set` refuses `-i`, `-c` and `-s`, so
`Invocation` is settled at startup. `Options` is a snapshot, since a subject
may `set -e` at any point.

## Why a shell states what it is

A walk cannot be read without it. Bash writes `$0` into `BASH_SOURCE` for code
it was given rather than read from a file, and `main` there for anything
defined at an interactive prompt — words a script can also produce. Telling
those apart is a property of the shell, and the shell is what knows it:
`Invocation::from_a_file` is `command.is_none() && !standard_input`.

An interactive shell joins by typing its own way in, loading the pieces and
saying the init, because bash reads `BASH_ENV` for non-interactive shells
only. The mechanism is indifferent to this: the same `BC_JOIN` runs however
the shell got there, under either orchestration. See
[scoping.md](scoping.md) and [stack.md](stack.md#bashs-own-words).

## What the account leaves out

Who forked whom. A fork inherits its parent's pipe descriptor and takes its
own on its first word; its descent from a particular shell is not reported,
because bash does not track it either. What bash does know, `$SHLVL` and
`$BASH_SUBSHELL`, is reported as bash states it.

## See also

- [wire.md](wire.md#the-fifos) — how the account travels
- [rigs.md](rigs.md) — where a shell enters a reaction
- [stack.md](stack.md) — the walk that cannot be read without the shell

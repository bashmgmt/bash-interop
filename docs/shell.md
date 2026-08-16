# The shell — what one is

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

**A shell's account of itself opens it**, and it is the shell saying so rather
than anything inferred from the shape of what it went on to write. The account
travels with the announcement — on the control fifo, in frames, before the
shell's pipe is even opened — so the run knows everything about a shell before
it releases it. It is not a `Message` and cannot become one: it is what *makes*
a shell, and a `Message` presupposes one.

The account is built by one prelude function — read it as a checklist of
everything a shell states about itself (anchored; these are the shipped
bytes):

<!-- quote: src/rig/wire/prelude.bash anchor=account -->
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

One array literal, the clock first and no verb, written into the caller's
local. Everything in it is passed as bash reports it, and what any of it means
is decided in `Shell::of`. Adding a fact is a word here and a field there.

`brought` is the one entry the client writes: the words its join carried
(`BC_JOIN LABEL DIR word…`), one nested literal — the shape `versinfo` takes —
landing verbatim on `Shell::brought`. An arglist like a message's: the
protocol reserves no word in it, and `key value` pairs read with `field` are
the client's own convention. A fork's reattach announces its label's words; a
child process re-derives them at its own join.

None of it can change while a shell lives: a subshell has a `$BASHPID` of its
own and joins as a shell of its own, and `set` refuses `-i`, `-c` and `-s`, so
`Invocation` is settled at startup. `Options` is a snapshot — a subject may
`set -e` at any point.

## Why a shell has to say what it is

A walk cannot be read without it. Bash writes `$0` into `BASH_SOURCE` for code
it was given rather than read from a file, and `main` there for anything defined
at an interactive prompt — words a script can also produce. Which is which is a
property of the shell, and the shell is the only thing that knows:
`Invocation::from_a_file` is `command.is_none() && !standard_input`.

An interactive shell can join *only* by typing its own way in — loading
the pieces and saying the init — because bash reads `BASH_ENV` for
non-interactive shells alone. Nothing about the mechanism cares: the same
`BC_JOIN` runs however the shell got there, under either orchestration.
See [scoping.md](scoping.md) and [stack.md](stack.md#bashs-own-words).

## What is not here

Who forked whom. A fork inherits its parent's pipe descriptor and takes
its own on its first word; that it descends from a particular shell is not
reported — no ancestry is tracked, because bash itself does not know it.
What bash *does* know — `$SHLVL`, `$BASH_SUBSHELL` — is reported as bash
states it, nothing more.

## See also

- [wire.md](wire.md#the-fifos) — how the account travels
- [rigs.md](rigs.md) — where a shell enters a reaction
- [stack.md](stack.md) — the walk that cannot be read without the shell

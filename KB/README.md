# Bash interop

Two layers. **Values** (`src/bash/value/`) read and write bash's own quoted
forms — `@Q`, `@A`, `declare -p`. **The rig** (`src/bash/rig/`) runs a bash
program with instrumentation injected, receives messages from every shell in
the resulting process tree, and answers questions those shells ask.

```
KB/mb_resolver/bash/                              src/bash/
  values.md         quoted forms, BashVal, codecs   value/
  wire.md           the bash, the pipe, the frame   rig/wire/
  rig.md            Rig, ExitStatus, run            rig/mod.rs, rig/run.rs
  tree.md           shells and the process forest   rig/tree.rs
  measurements.md   numbers, limits, proofs
  bashcap.md        the reference tool              bashcap/
```

Every module under `src/bash/rig/` is private; `mod.rs` carries the trait,
`ExitStatus`, and one re-export list that is the API:

```rust
pub use run::{run, run_in};
pub use tree::{forest, shells, Shell, ShellNode};
pub use wire::{field, prelude, Answer, Line, Micros, Pid};
pub use crate::failure::{Doing, Failure};
```

**The library ships no accumulator and no rig implementation.** What a run
produces is the client's, expressed as its `Session` — see
[rig.md](rig.md#what-a-session-is-for).

## Three moments

| moment | started by | effect |
|---|---|---|
| setup | `run`, once | two bash files are laid into the workspace, and `BASH_ENV` reaches every shell with them |
| say | the subject | `BC_INSTR say …` ships an arglist and returns |
| ask | the subject | `BC_INSTR ask …` ships one, blocks, and runs what comes back |

`BC_INSTR` is the only name client code calls; its leading word selects the
operation.

The contract towards the subject's shell: no trap installed, no builtin
shadowed, no variable exported, no global set outside `__BC__*`, no shell
option changed, and no `eval`. A client's own traps, `IFS` and options are
therefore its own, and `tests/proofs.rs` asserts this against the generated
prelude.

## A message is an arglist

In both directions, of any width including zero, and the rig reads no position
of one. A leading discriminator — `TIMEIT`, `__BASHCAP__` — is the sender's choice,
and a decoder opts into it:

```rust
let words = line.behind("TIMEIT")?;   // None: some other tool's message
```

The protocol reserves no word in the payload. Its own — the `SAY`/`ASK` kind
and the `at=`/`parent=`/`shlvl=` context — sit in front of the client's
arglist and are shifted off before a rig sees one, the way bash `shift`s past
its own arguments. `Line::words` is the client's alone.

An **answer** is an arglist too, and it is a command the shell runs:
`["return", "1"]`, `["source", path]`, `["declare", "-g", "x=1"]`,
`["exit", "9"]`, or any word the prelude defined.

## Building on it

Declare a session type and implement `Rig::open`; add `bash` if the subject
needs a word of your own, and override `hear`, `answer` or `end` if you care
about them. Then call `run`. Provenance, ordering, the process forest,
subshell capture, concurrent-writer integrity and the control channel come
with the transport.

`tests/examples/` is four worked rigs against the public API alone.

## Reading order

README → rig.md → wire.md, then whichever concern applies. Changing the
transport: wire.md → measurements.md. Writing a tool: bashcap.md.

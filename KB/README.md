# Bash interop

Three layers. **Values** (`src/bash/value/`) read and write bash's own quoted
forms — `@Q`, `@A`, `declare -p`. **The stack** (`src/bash/stack/`) is the
frame walk every instrument shares, both halves: the bash that ships bash's
five arrays, and the Rust that puts them back together. **The rig**
(`src/bash/rig/`) is a session with instrumentation in it: it receives messages
from every shell that joined and answers the questions those shells ask.

`stack` and `rig` are siblings; neither knows the other. A tool composes them.

```
KB/mb_resolver/bash/                              src/bash/
  values.md         quoted forms, BashVal, codecs   value/
  wire.md           the bash, the pipe, the frame   rig/wire/
  rig.md            Rig, Master, Slave, Serving     rig/{mod,master,slave,serving}.rs
  tree.md           shells and the process forest   rig/tree.rs
  stack.md          the call stack, both halves     stack/
  measurements.md   numbers, limits, proofs
  scoping.md        where a name binds              every *.bash we ship
  vendoring.md      shipping instrumented bash      assets/
  bashcap.md        the reference tool              bashcap/
  bashprof.md       a call tree that travels        bashprof/
```

Every module under `src/bash/rig/` is private; `mod.rs` carries `Rig`,
`Workspace` and one re-export list that is the API:

```rust
pub use master::{Master, Run};
pub use slave::{Served, Slave};
pub use status::ExitStatus;
pub use tree::{forest, shells, Shell, ShellNode};
pub use wire::{field, Answer, Kind, Line, Micros, Pid, Sent};
pub use crate::failure::{Doing, Failure};
```

**A rig is the reaction; who started what is a second question.** `Master` runs
a command line of its own and owns its process group; `Slave` hands its address
to a bash script that started the server and serves while that script holds the
handle. Both are traits extending `Rig` with one provided method, so a rig
declares which orchestrations it supports by implementing them. A session lasts
as long as anyone who could still speak, and nothing inside a rig ends one.

**The library ships no accumulator and no rig implementation.** What a run
produces is the client's, expressed as its `Session` — see
[rig.md](rig.md#what-a-session-is-for).

Every instrument that reports a walk is composed by `stack::with(&[…])`, which
puts `stack.bash` first: `__bc_stack` has to be defined before anything calls
it, and that rule lives there rather than at each tool.

## Three moments

| moment | started by | effect |
|---|---|---|
| setup | the role method, once | two bash files are laid into the workspace `Rig::workspace` chose, and either `BASH_ENV` carries them to a subject the run starts or the address goes to a script that joins |
| say | the subject | `BC_INSTR say …` ships an arglist and returns |
| ask | the subject | `BC_INSTR ask …` ships one, blocks, and runs what comes back |

`BC_INSTR` is the only name client code calls; its leading word selects the
operation.

The contract towards the subject's shell: no trap installed, no builtin
shadowed, no variable exported, no name set outside `__BC_*`, no `set -o`
change, and no `eval`. A client's own traps, `IFS` and options are therefore
its own. The one option the protocol does turn on is `expand_aliases`, which
the guards require — see [wire.md](wire.md#error-flow-is-ours-not-set--es).

The one name outside `__BC_*` the protocol touches is `LC_ALL`, taken `local`
for the length of one wide frame so that framing counts the bytes `PIPE_BUF`
counts. It is restored before the send returns, and the subject runs nothing
of its own in between — asserted in `proofs/transparency.rs`.

**Error flow is taken from `set -e` deliberately.** Every command that can
fail in the protocol's bash is guarded with `|| __BC_BAIL` or `|| __BC_THROW`,
so it behaves the same however the subject set its shell, and a fault of ours
is reported at the subject's call site with status 125 rather than killing the
script mid-message. That is a requirement of BC-related bash, not a style
preference.

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

Provenance, ordering, the process forest,
subshell capture, concurrent-writer integrity and the control channel come
with the transport.

Declare a session type and implement `Rig::open`; add `bash` if the subject
needs a word of your own; then implement `Master`, `Slave`, or both.

`tests/examples/` is worked rigs against the public API alone, meant to be read
top to bottom, and `tests/joining.rs` is the same from bash's side: a fixture
script that starts a session, asks it questions and closes it. A corner case of one building block belongs beside it, in
`src/<module>/tests/`.

## Reading order

README → rig.md → wire.md, then whichever concern applies. Changing the
transport: wire.md → measurements.md. Writing a tool: stack.md → bashcap.md.
Reading a run back as structure: tree.md → bashprof.md.

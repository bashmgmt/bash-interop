# Bash interop

Four layers. **Values** (`src/bash/value/`) read and write bash's own quoted
forms — `@Q`, `@A`, `declare -p`. **The shell** (`src/bash/shell.rs`) is which
bash a shell is, how it was given its code and what it has switched on — a
shell's own account of itself, which it gives once when it joins. **The stack**
(`src/bash/stack/`) is the frame walk every instrument shares, both halves: the
bash that ships bash's five arrays, and the Rust that puts them back together.
**The rig** (`src/bash/rig/`) is a session with instrumentation in it: every
shell that joins gets a pipe of its own and a task of its own, and the rig
hears what each says and answers what it asks.

`stack` and `rig` are siblings; neither knows the other. Both stand on `shell`,
because a walk cannot be read without knowing the shell it was taken in: bash
writes `$0` into `BASH_SOURCE` for code it was given rather than read, and no
walk can say on its own which word that is. A tool composes them.

```
KB/mb_resolver/bash/                              src/bash/
  onboarding.md     start here: the words, the surface as code, one rig
  values.md         quoted forms, BashVal, codecs   value/
  shell.md          what a shell is                 shell.rs
  wire.md           the bash, the fifos, the lines  rig/wire/
  rig.md            Rig, Reacting, Driving, Serving rig/{mod,attend,session,watch,driving,serving}.rs
  stack.md          the call stack, both halves     stack/
  measurements.md   numbers, limits, proofs
  scoping.md        where a name binds              every *.bash we ship
  vendoring.md      shipping instrumented bash      assets/
  bashcap.md        the reference tool              bashcap/
  bashprof.md       a call tree that travels        bashprof/
```

Every module under `src/bash/rig/` is private; `mod.rs` carries `Rig` and
`Reacting` and one re-export list that is the rest of the API:

```rust
pub use attended::{heard, Attended, Kept, Layout, Reaching, Said, Setup, Workspace};
pub use driving::{Driving, ExitStatus, Run, Whole};
pub use serving::{Served, Serving};
pub use wire::{field, Answer, Message, Micros, Pid, Stamp, Verb};
pub use crate::bash::shell::Shell;
pub use crate::failure::{Doing, Failure};
pub const JOINING: &str = include_str!("joining.txt");
```

**A rig is a description; the reaction is per shell.** `Rig::joined` builds one
the moment a shell announces itself, so which bash it is, how it was started
and what it had switched on are members from construction rather than something
looked up per message. Each reaction runs as a task of its own on a
single-threaded runtime, so what one shell's reaction awaits holds up nothing
but that shell. The library ships two reactions — `Vec<Message>` keeps every
message, `()` keeps nothing — and no rig implementation.

**Who started what is a second question.** `Driving` runs a command line of
its own and owns its process group; `Serving` hands its address to a bash
script that started the server and serves while that script holds the handle.
Both are traits extending `Rig` with one provided `async fn`, so a rig declares
which orchestrations it supports by implementing them. A session lasts as long
as anyone who could still speak, and nothing inside a rig ends one.

**The address is one file, and the core exports it and decides nothing else.**
The address is the prelude's path — the file a shell sources to join. A driven
subject finds it as `BC_SESSION` in its environment; a served client is handed
it once and puts it there itself (`BC_START`). Whether `BASH_ENV` also names
it — so every non-interactive bash in the tree joins as it starts — is the
rig's answer, given through `Driving::environment`; `Reaching` spells the two
usual ones. `JOINING` is every way a script joins, in bash, and both binaries
print it under `--help`.

Every instrument that reports a walk is composed by `stack::with_walk(&[…])`,
which puts `stack.bash` first: `__bc_stack` has to be defined before anything
calls it, and that rule lives there rather than at each tool.

## Three moments

| moment | started by | effect |
|---|---|---|
| join | `rig.bash`, at source — `BC_JOIN <LABEL>` | the shell makes its pipe, announces it on the control fifo with its account, and blocks until the run has opened it. A fork does the same on its first word |
| say | the subject | `BC_INSTR <LABEL> say …` ships an arglist and returns |
| ask | the subject | `BC_INSTR <LABEL> ask …` ships one, blocks, and runs what comes back |

`BC_INSTR` and `BC_JOIN` are the only names client code calls. The label is a
bash-side lookup key, so one process can hold several sessions; Rust is never
told it.

The contract towards the subject's shell: no trap installed, no builtin
shadowed, no variable exported, no name set outside `BC_*`/`__BC_*`, no `set -o`
change, and no `eval`. A client's own traps, `IFS`, locale and options are
therefore its own. The one option the protocol does turn on is
`expand_aliases`, which the guards require — see [wire.md](wire.md#error-flow).

**Error flow is taken from `set -e` deliberately.** Every command that can
fail in the protocol's bash is guarded with `|| __BC_BAIL` or `|| __BC_THROW`,
so it behaves the same however the subject set its shell, and a fault of ours
is reported at the subject's call site with status 125 rather than killing the
script mid-message.

## A message is an arglist

In both directions, of any width including zero, and the rig reads no position
of one. A leading discriminator — `TIMETHIS`, `__BASHCAP__` — is the sender's
choice, and a decoder opts into it:

```rust
let words = message.behind("TIMETHIS")?;   // None: some other tool's message
```

The protocol reserves no word in the payload. Its own — the `SAY`/`ASK` verb
and the `at=` clock — sit in front of the client's arglist and are shifted off
before a rig sees one. `Message::words` is the client's alone. What a shell
*is* is not on a message at all: it is said once, on joining, and reached
through the shell a reaction was built with.

An **answer** is an arglist too, and it is a command the shell runs:
`["return", "1"]`, `["source", path]`, `["declare", "-g", "x=1"]`,
`["exit", "9"]`, or any word the rig's bash defined.

## Building on it

Implement `Rig` — `setup`, `joined` — and a `Reacting` for what it builds,
unless `Vec<Message>` or `()` is what you want; then implement `Driving`
(`environment`: how the shells are reached), `Serving`, or both. `Setup::bash`
states `BC_JOIN <LABEL>`. What several shells share is yours, held by the rig
and handed to each reaction it builds — an `Rc<RefCell<_>>` is enough, and its
borrow is never held across an `.await`.

Nothing on either trait defaults, so an `impl` block is the whole contract.
What a default used to decide is a named value instead: `Answer::unknown()` is
the `return 127` for a word a rig has no answer for, `Answer::ok()` the quiet
yes.

`tests/examples/` is worked rigs against the public API alone, meant to be read
top to bottom, and `tests/joined/` is the same from bash's side: a fixture
script starts a server, drives its own session and closes it. A corner case of
one building block belongs beside it, in `src/<module>/tests/`.

## Reading order

onboarding.md → README → rig.md → wire.md, then whichever concern applies. Changing the
transport: wire.md → measurements.md. Writing a tool: stack.md → bashcap.md.

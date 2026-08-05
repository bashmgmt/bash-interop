# Bash interop

Two layers, and everything in this crate that talks to bash uses both.

**Values** (`src/bash/value/`) decode and emit bash's own quoted forms. Bash
has one serialisation format — `@Q`, `@A`, `declare -p` — and this layer is
the Rust side of it. Nothing above it invents a format.

**The rig** (`src/bash/rig/`, with its bash in `bash/`) is a driver: it runs a
bash program with instrumentation injected, receives messages from every shell
in the resulting process tree, and answers questions those shells ask. It is
bidirectional, arglist-based, and knows nothing about what the messages mean.

```
KB/mb_resolver/bash/                  src/bash/
  values.md       @Q, @A, BashVal/Schema           value/
  wire.md         pipes, framing, message, reply   rig/wire/
  source.md       BashSrc and the prelude          rig/source/
  capture.md      Capture, and the process forest  rig/capture/
  run.md          the Rig trait, listen/converse   rig/run/, rig/listen.rs
  design.md       the decisions and measurements
  bashcap.md      the reference tool, end to end   utilprog/bashcap/
  managebash.md   the other consumer               mb/
```

The rig's modules are private; `src/bash/rig/mod.rs` carries one re-export
list and that list is the API. Planning lives in
[`KB/.plans/`](../../.plans/INDEX.md).

The documents mirror the source directories one for one. `values.md` is the
layer beneath; `design.md` collects the reasoning that would otherwise be
scattered; the last two are complete worked consumers.

## Three moments, and no others

| moment | who starts it | what happens |
|---|---|---|
| **setup** | the operator, once | the prelude is written and reaches every shell via `BASH_ENV` |
| **say** | the subject | `BC_INSTR say …` ships a message and returns |
| **ask** | the subject | `BC_INSTR ask …` ships one, blocks, and continues with what comes back |

`BC_INSTR` is the only name client code ever calls, and its leading word
selects which of the two it wants. Nothing else enters a running shell: the
rig installs **no traps**, shadows
**no builtin**, exports **no variable**, mutates **no global shell state**, and
contains **no `eval`** — asserted against the generated text by
`tests/proofs.rs::the_prelude_is_non_invasive_and_self_reliant`.

## One way out

A run yields an `ExitStatus` or a `RigError`. Nothing is carried alongside,
nothing is dropped, and nothing half-succeeds: the first thing that cannot be
read or written ends the run, and the subject's process group is killed on the
way out. See [run.md](run.md#one-error-one-way-out).

## A message is an arglist

In both directions. `BC_INSTR ask a b c` delivers exactly `["a", "b", "c"]`,
and `BC_INSTR say a b c` puts exactly those three words on the wire. The rig
reads no position of either and attaches no meaning to any word.

A leading discriminator — `DSL`, `TIMEIT`, `__BASHCAP__` — is a word the
*sender* chose, and the decoder opts into in one line:

```rust
let words = record.behind("TIMEIT")?;   // None: not this tool's record
```

The **answer** is an arglist too, and it is a command the shell runs. That is
the whole of continuing: `["return", "1"]`, `["source", path]`,
`["declare", "-g", "x=1"]`, `["exit", "9"]`, or any word the prelude defined.
There are no variants, because a bash command array can already reach anything
the shell knows.

Two words are reserved, and both are the transport describing itself rather
than a tool describing its payload: `__ORIGIN__`, which opens each shell's
stream, and `__ASK__`, which labels a question so it is visible in the
capture. `__ASK__` is stripped before an ask reaches an answer.

## Adding a tool

One [`Rig`](run.md#a-rig-is-two-functions) — `setup` says how a run starts,
`answer` says what a shell that asked runs next — and one
[`FromRecord`](wire.md#typed-decoding) for the Rust side. Provenance, global
ordering, the process forest, subshell capture, concurrent-writer integrity and
the control channel are inherited.

A tool that only reports needs no Rust on the bash side at all: its script
says what it has to say through `BC_INSTR say`.

`tests/examples/` builds six of them, smallest first. `make examples`.

## Reading order

New to this: **README → run.md → wire.md**, then whichever concern you need.
Changing the transport: **wire.md → design.md**. Writing a tool:
**source.md → bashcap.md**, with `tests/examples/own_tool.rs` open.

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
KB/mb_resolver/bash/
  values.md       @Q, @A, BashVal/Schema, the two codecs
  wire.md         the pipes, the framing, the message and control protocol
  codegen.md      BashSrc, Asset, Codegen — producing the injected bash
  capture.md      Capture and its views: order, shells, the process forest
  instrument.md   Instrument and Dispatch — a bash contribution as a value
  run.md          Rig, Outcome, exit status and signals, capture_into
  design.md       the decisions, and the measurements behind them
  bashcap.md      the reference tool, end to end
```

The documents mirror the source directories one for one. `values.md` is the
layer beneath; `design.md` collects the reasoning that would otherwise be
scattered; `bashcap.md` is a complete worked consumer.

## Three moments, and no others

| moment | who starts it | what happens |
|---|---|---|
| **setup** | the operator, once | the prelude is written and reaches every shell via `BASH_ENV` |
| **speak** | the subject | it calls a function that ships a message; one-way, cheap |
| **ask** | the subject | it calls `BC_INSTR`, blocks, and continues with what comes back |

Nothing else enters a running shell. The rig installs **no traps**, shadows
**no builtin**, exports **no variable**, mutates **no global shell state**, and
contains **no `eval`** — asserted against the generated text by
`src/bash/rig/tests/mod.rs::the_prelude_is_non_invasive_and_self_reliant`.

## A message is an arglist

In both directions. `BC_INSTR a b c` delivers exactly `["a", "b", "c"]`; a
spoken message is exactly the bash array the instrument built. The rig reads
no position of either and attaches no meaning to any word.

A leading discriminator — `DSL`, `TIMEIT`, `__BASHCAP__` — is a convention a
tool opts into in one line:

```rust
let words = record.behind("TIMEIT")?;   // None: not this tool's record
```

Two words are reserved, and both are the transport describing itself rather
than a tool describing its payload: `__ORIGIN__`, which opens each shell's
stream, and `__ASK__`, which labels a question so it is visible in the
capture. `__ASK__` is stripped before an ask reaches an answer.

## Adding a tool

One [`Instrument`](instrument.md) for the bash side, one
[`FromRecord`](wire.md#typed-decoding) for the Rust side. Provenance, global
ordering, the process forest, subshell capture, concurrent-writer integrity
and the control channel are inherited.

`tests/example_tests/` builds six of them, smallest first. `make examples`.

## Reading order

New to this: **README → run.md → wire.md**, then whichever concern you need.
Changing the transport: **wire.md → design.md**. Writing a tool:
**instrument.md → bashcap.md**, with `tests/example_tests/own_tool.rs` open.

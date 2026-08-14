# The bash instrumentation toolkit — design

What `src/bash/` is, what each layer is allowed to know, and the decisions the
shape follows from. Per-directory reference is [`../bash/`](../bash/README.md);
this document is the level above it.

## What it is for

Run a bash program, hear every shell in its process tree, and answer the
questions those shells ask — without changing how the program behaves when
nothing is listening.

That last clause is the whole difficulty. A subject script has its own traps,
its own `IFS`, its own shell options and its own exit status, and a tool that
disturbs any of them is measuring something other than the program. So the
design is organised around what the instrumentation may *not* do, and every
capability is built from what is left.

## The layers

```
value ─┬─ shell ─┬─ stack        bash's five parallel arrays, read back
       │         └─ rig          the session: a wire, a workspace, reactions
       └─────────────┘
```

| | knows about | never knows about |
|---|---|---|
| `value` | bash's quoted forms — `@Q`, `@A`, `declare -p` | everything else |
| `shell` | `value` | how a shell was reached, what it went on to say |
| `stack` | `value`, `shell` | the wire, the rig, any tool |
| `rig` | `value`, `shell` | the stack, any tool |

**`stack` and `rig` are siblings and neither calls the other.** A tool composes
them: the frame walk goes into the bash a rig injects, through `stack::with`.
That is what `bashcap` and `bashprof` are, and a third tool would be the same
composition with different words.

**Both stand on `shell`**, because a walk cannot be read without knowing the
shell it was taken in. Bash writes `$0` into `BASH_SOURCE` for code it was
*given* rather than read from a file, and `main` there for anything defined at
an interactive prompt — words a script can also produce. Which is which is a
property of the shell, and the shell is the only thing that knows.

## Six decisions

### 1. A message is an arglist, in both directions

Not a schema, not a struct, not a serialisation format. `BC_INSTR say a b c`
ships three words; a rig gets three words. Any width, zero included, and the
protocol reads no position of one.

This is what lets several tools share one wire: a leading discriminator —
`TIME_CPS`, `__BASHCAP__` — is the sender's own choice, and a decoder opts in
with `line.behind(TAG)`, getting `None` for somebody else's message. There is
no registry and nothing to coordinate.

An **answer** is an arglist too, and it is a command the shell runs. That one
choice covers every case that would otherwise need a protocol: `["return",
"1"]` refuses, `["declare", "-g", "x=1"]` sets a variable in the asking shell's
own scope, `["source", path]` runs bash of any length the rig wrote to a file,
`["exit", "9"]` ends the subject. Expressiveness is bash's, so there is no
answer type to extend.

### 2. Values travel as bash's own quoted forms

`${x[*]@Q}` and `"(${x[*]@Q})"` on the way out, `parse_array` on the way in;
`declare -a x="$msg"` and `emit_array` the other way. Both sides speak the
notation bash already has, so word boundaries, newlines, tabs and bytes bash
cannot display survive without a length prefix, an escape scheme, or a
dependency on either side's idea of encoding.

`value` therefore stands on nothing and is usable on its own — see
[values.md](../bash/values.md).

### 3. Instrumentation reaches shells through `BASH_ENV`, not argv

A command line reaches one process. `BASH_ENV` reaches every non-interactive
bash in the tree the subject creates, which is what makes `bashcap
run_bash_env --into out make test` work: every recipe shell `make` starts joins
by itself.

So the run lays two files into a workspace — the protocol's bash and the rig's
— and points `BASH_ENV` at the first. Nothing is templated into either: the
prelude finds its own workspace from `${BASH_SOURCE[0]%/*}`, which means the
shipped file is real bash that can be read and checked directly.

The command line is then free to be exactly what the caller wrote, program
included: `&["env", "TARGET=staging", "bash", "x.bash"]` needs no support from
the run.

### 4. A rig is a description; a reaction is per shell

```rust
trait Rig     { type Attending: Reacting;  fn bash(&self);  fn joined(&self, at, shell); }
trait Reacting { type Kept;  fn hear(&mut self);  fn answer(&mut self);  fn finish(self); }
```

A shell's first message is its account of itself: which bash, how it was given
its code, where it sits, what it had switched on. None of that can change while
the shell lives — a subshell gets its own `$BASHPID` and joins as a shell of its
own, and `set` refuses `-i`, `-c` and `-s`. So it is said once, and the reaction
built from it holds it as a **member from construction** rather than looking it
up per message.

Owning a reaction is the proof that its shell announced itself; there is no
other way to construct one. A message from a pid that never joined therefore
cannot reach a reaction at all — it is a fault, and the run says so.

What comes back is one entry per shell, `Attended { shell, kept }`, and the
provenance is the *shape*: no second list to cross-reference, nothing that could
disagree with it. `heard` flattens it back to arrival order when a reading wants
the run whole.

What several shells share — a sink, a merged view — belongs to the rig, which
hands each reaction a share. The core names no sharing discipline and has no
opinion on one.

### 5. Who started the shells is a second question

`Master` runs a command line and owns its process group. `Slave` hands its
address to a bash script that started the server and serves while that script
holds the handle. Both are traits extending `Rig` with one provided method, so
a rig declares which orchestrations it supports by implementing them, and its
reaction is the same code either way.

Both tools expose the pair as two symmetric verbs taking one shared options
type — `run_bash_env` and `serve` — so the command line says the same thing the
traits do:

```
bashprof run_bash_env --into build.times -- make test
bashprof serve        --into build.times      # started by BC_JOIN, from a script
```

One sentence covers both ends: **a session lasts as long as anyone who could
still speak.** `Until` is a descriptor — a pidfd, or the handle an initiator
holds — and it is only ever *watched*. Signalling and reaping belong to whoever
started the thing being watched, which is never the serving loop. That is what
lets one loop serve both.

Nothing inside a rig ends a session. A rig reacts; a `Failure` from it means it
could not do its work, not that it is finished.

### 6. The subject keeps everything of its own

| | |
|---|---|
| no trap installed | a client's `trap … EXIT` fires as it would unwrapped |
| no builtin shadowed | `printf`, `read`, `exec` mean what they mean |
| no variable exported | nothing leaks into a child that did not join |
| no name outside `__BC_*` | a subject's globals cannot collide with ours |
| no `set -o` change | `errexit`, `nounset`, `pipefail` are the subject's |
| no `eval` | nothing of the subject's is re-parsed |
| its own exit status | a wrapped script is indistinguishable from an unwrapped one |

Two exceptions, both deliberate and both measured. `expand_aliases` is turned
on and stays on, because the error guards must be aliases — `return` has to act
in the frame that failed. And `LC_ALL` is taken `local` for the length of one
wide frame, so framing counts the bytes `PIPE_BUF` counts; it is restored before
the send returns and the subject runs nothing of its own in between.

Because the protocol may not use `set -e`, every command in it that can fail is
followed by `|| __BC_BAIL` or `|| __BC_THROW`. A fault of ours is then reported
at the *subject's* call site with status 125 — what `env` and `timeout` return
when the wrapper rather than the payload failed — rather than killing the script
mid-message.

## What the transport gives every tool

Provenance, ordering, the process forest, subshell capture, concurrent-writer
integrity and a control channel — none of which a tool implements again:

- **Every shell opens the pipe itself, by name.** Nothing is inherited, so no
  descriptor has to survive a fork and a client's own use of an fd cannot
  collide. `$BASHPID != $__BC__owner` detects a fork, so a `( … )` or `$( … )`
  rejoins with its own descriptor and its own sequence counter.
- **One frame is one atomic write** under `PIPE_BUF`, so concurrent writers
  cannot interleave; anything wider is chunked under a shared `(pid, seq)` key
  and rejoined as bytes.
- **Both clocks on every message**: the sending shell's `$EPOCHREALTIME` and the
  run's own. A span is the interval between two of them, which is why nothing is
  timed in bash.
- **A run-wide arrival counter**, because a per-shell fold keeps its own order
  and nothing else.

## The tools are compositions, not special cases

| | its bash | its reading |
|---|---|---|
| `bashcap` | the walk, plus `BASHCAP`'s effect | one JSON object per snapshot, streamed |
| `bashprof` | the walk, plus `BASHPROF_TIME_CPS`'s effect | three passes: records, tree, timings |

Neither is privileged. Both ship the words a call site says as a file that is
*both* injected and vendored, so a client's copy and the tool's cannot drift —
the words name a hook, and only the hook exists twice. A script with the words
and no tool runs unprofiled; the same script under the tool measures itself. See
[vendoring.md](../bash/vendoring.md).

## What is deliberately absent

| | why |
|---|---|
| a session-wide accumulator in the library | what a run produces is the client's; `Vec<Line>` and `()` are the only two shipped |
| a timer, an interval, a heartbeat | serving ends when nobody who could speak is left, and that is a descriptor |
| a closing word or reserved payload word | the handle says when it is over, so nothing in the loop intercepts a message |
| a poisoned or degraded mode | an answer that says no is a command returning non-zero, like any other |
| parallelism in the serving loop | one pipe, one reader; the cost is bash's `printf`, not ours |
| a schema or IDL | an arglist has no shape to agree on |

## See also

- [`../bash/README.md`](../bash/README.md) — the layer-by-layer reference
- [`../bash/rig.md`](../bash/rig.md) — `Rig`, `Reacting`, and the two roles
- [`../bash/wire.md`](../bash/wire.md) — the protocol, frame by frame
- [`../bash/measurements.md`](../bash/measurements.md) — every number above
- [`../bash/scoping.md`](../bash/scoping.md) — where a name binds in the shipped bash

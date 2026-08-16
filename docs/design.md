# The bash instrumentation toolkit — design

What `src/` is, what each layer is allowed to know, and the decisions the
shape follows from. The chapter-by-chapter reference is the rest of
[this book](README.md); this document is the level above it.

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
bash-strings ─┬─ shell ─┬─ stack   bash's five parallel arrays, read back
              │         └─ rig     the session: a workspace, a pipe and a task per shell, reactions
              └─────────────┘
```

| | knows about | never knows about |
|---|---|---|
| `bash-strings` (its own crate) | bash's quoted forms — `@Q`, `@A`, `declare -p` | everything else |
| `shell` | `bash-strings` | how a shell was reached, what it went on to say |
| `stack` | `bash-strings`, `shell` | the wire, the rig, any tool |
| `rig` | `bash-strings`, `shell` | the stack, any tool |

**`stack` and `rig` are siblings and neither calls the other.** A tool composes
them: the frame walk goes into the bash a rig injects, through `stack::with_walk`.
That is what `bashcap` and `bashprof` are, and a third tool would be the same
composition with different words.

**Both stand on `shell`**, because a walk cannot be read without knowing the
shell it was taken in. Bash writes `$0` into `BASH_SOURCE` for code it was
*given* rather than read from a file, and `main` there for anything defined at
an interactive prompt — words a script can also produce. Which is which is a
property of the shell, and the shell is the only thing that knows.

## Six decisions

### 1. A message is an arglist, in both directions

Not a schema, not a struct, not a serialisation format. `BC_INSTR L say a b c`
ships three words; a rig gets three words. Any width, zero included, and the
protocol reads no position of one.

This is what lets several tools share one wire: a leading discriminator —
`TIMETHIS`, `__BASHCAP__` — is the sender's own choice, and a decoder opts in
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
`bash-strings/docs/values.md`.

### 3. One coordinate, owned; the core decides nothing else

The workspace directory is the session's address and its only coordinate:
every fifo and file is `<dir>/…`, modelled in one place — `Layout`, a
validated directory and accessors for the constant names. The session
**owns** the directory it serves: `<dir>/lock` is `flock`ed before anything
in it is touched and held until its fifos are gone. Ownership is what makes
three promises cheap: a second session on the same directory is refused
whole; a killed predecessor's leavings are swept at the next open, because
the kernel released the dead lock; and the join fifo's presence is a
truthful liveness signal. A prescribed directory must exist; making it is
the host's job, in both roles.

The session lays two files: the generic prelude (shipped verbatim, reading
neither its own location nor the environment) and the rig's bash —
**definitions only**, `Rig::bash(&Layout)`, inert to source. Initiation is a
line of client code: `BC_JOIN <label> <dir> [word…]`, zero, one or many
labels, the label being client vocabulary — the write-time-stable name the
words speak, bound to a run-time coordinate at the join; Rust is never told
it. The words after the dir ride the announcement and land on
`Shell::brought`. The rig states its standard initiation as data,
`Rig::joining(&Layout)`, which the core never runs: it is written into a
provisioned startup file, or said by a client's own line. The one file that
may initiate, `<dir>/bash_env.bash`, is written only by
`Layout::bash_env(provision)` — the two sources, then the joining line iff
`Provision::Joining` — the choice stated first, by whoever provisions.

A driven run starts the command line; its workspace is a temporary directory
of the run's own — or, `run_at`, one the caller made and keeps — so nothing
external can prescribe or collide with it. **How the shells reach the
session is the run's question, stated at the run**: `run` and `run_at` take
an environment closure — fallible, because provisioning writes a file —
whose return is the subject's whole environment delta: the core exports
nothing. `Layout::bash_env(Provision::Joining(…))` is the usual pair: the
join of every non-interactive bash in the tree — what makes `bashcap run
--into out make test` work, every recipe shell `make` starts joining by
itself. `BC_SESSION` is the tools' own convention — the workspace as a
variable, `BASHCAP_INIT "$BC_SESSION"` where a by-hand script says —
spelled in their binaries, not in the core. The tools' `--reach` is their
vocabulary over these spellings: `bash-env` provisions a joining file,
`by-hand` a definitions file, initiation the scripts' own.

A serving run requires the workspace from outside — `--at`, existing, no
fallback — and answers to nobody: nothing is written back, a serving
application is a complete standalone program, and the client feeds the same
directory to start, probe, load and initiate. Liveness is the workspace's
to show — the join fifo is present exactly while a session serves — so
liveness is one file test, and the one boundary is a server killed
outright, whose stale fifo stands until its directory is next opened or
removed.

The upshot across both roles: the join is always one line, a provisioned
file's or the client's own, and `JOINING` — one text, printed by both
binaries under `--help` — is every way a script writes it. One limit is
stated rather than papered over: `BASH_ENV` is a single variable, so two
driven runs nested through it shadow each other for the inner subtree; the
escape is `--reach by-hand`, and it is the client's.

The command line is free to be exactly what the caller wrote, program
included: `&["env", "TARGET=staging", "bash", "x.bash"]` needs no support from
the run.

### 4. A rig is a description; a reaction is per shell, and a task

```rust
trait Rig      { type Reaction: Reacting;  bash(&Layout) -> String;  joining(&Layout) -> String;
                 async joined(&Layout, Arc<Shell>) }
trait Reacting { type Kept;  async hear(Message);  async answer(Message) -> Answer;  async finish() -> Kept }
```

Every shell has a pipe of its own, so which shell said something is which pipe
it came out of, and every pipe has a task of its own: read a line, react,
maybe answer, until end of input. A shell announces itself with its account:
which bash, how it was given its code, where it sits, what it had switched on
— on the control fifo, before its pipe is opened, so the run knows everything
about a shell before it releases it. None of that can change while the shell
lives — a subshell gets its own `$BASHPID` and joins as a shell of its own, and
`set` refuses `-i`, `-c` and `-s`. So it is said once, and the reaction built
from it holds it as a **member from construction**.

Owning a reaction is the proof that its shell announced itself; a message
reaches it only down that shell's own pipe.

The session is **single-threaded and concurrent**: one `current_thread`
runtime, `spawn_local` per shell, no `Send` bound anywhere. What one shell's
reaction awaits — a slow answer, a 100 KB reply, a file opened at `joined` —
holds up nothing but that shell. `Rc<RefCell<_>>` is a share; the borrow is
never held across an `.await`.

What comes back is one entry per shell, `Attended { shell, kept, parted }`, and
the provenance is the *shape*. `heard` flattens it back into the order it was
said — the sending shells' own clocks — when a reading wants the run whole.

**Neither trait has a default body.** A default is a decision an implementor did
not make and cannot see; `Answer::unknown()` names the refusal (`return
127`, bash's own "command not found") and puts it where it applies.

What several shells share — a sink, a merged view — belongs to the rig, which
hands each reaction a share. The core names no sharing discipline and has no
opinion on one.

### 5. Who started the shells is a second question

`Driving` runs a command line and owns its process group. `Serving` lays the
session in the workspace the client prescribed and serves while that client
holds the handle. Both are traits extending `Rig` with one provided `async fn`, so
a rig declares which orchestrations it supports by implementing them, and its
reaction is the same code either way.

Both tools expose the pair as two symmetric verbs taking one shared options
type — `run` and `serve` — so the command line says the same thing the traits
do:

```
bashprof run   [--reach bash-env|by-hand] --into build.times -- make test
bashprof serve --at "$PWD/prof.d" --into build.times   # started by a client's coproc
                                       # (mkdir; probe join; source the laid files; init)
```

One sentence covers both ends: **a session lasts as long as anyone who could
still speak.** `Watch` is a descriptor — a pidfd, or the handle an initiator
holds — and it is only ever *watched*. Signalling and reaping belong to whoever
started the thing being watched, which is never the session. That is what lets
one session serve both. Under `Driving` the group is killed before the session
closes, so every task reads what its shell wrote up to the kill.

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

One exception, deliberate: `expand_aliases` is turned on and stays on, because
the error guards must be aliases — `return` has to act in the frame that
failed. `IFS` is taken `local` inside two of the protocol's own functions so
`[*]` joins with a space, and the subject's — unset included — is back on
return.

Because the protocol may not use `set -e`, every command in it that can fail is
followed by `|| __BC_BAIL` or `|| __BC_THROW`. A fault of ours is then reported
at the *subject's* call site with status 125 — what `env` and `timeout` return
when the wrapper rather than the payload failed — rather than killing the script
mid-message.

## What the transport gives every tool

Provenance, ordering, subshell capture, lifetimes and a control channel — none
of which a tool implements again:

- **Every shell has a pipe of its own**, made by the shell, announced with its
  account, opened by the run: the blocking open is the rendezvous, and end of
  input is the goodbye. A `( … )` or `$( … )` that speaks takes a pipe of its
  own on its first word.
- **A line is a message.** One writer per pipe, so nothing interleaves and no
  write need be atomic; the pipe has no frame. The control fifo has many
  writers and one: an announcement is frames of at most `PIPE_BUF` bytes,
  keyed by the shell's token and put back together in bytes.
- **Both clocks on every message**: the sending shell's `$EPOCHREALTIME` and the
  run's own. A span is the interval between two of them, which is why nothing is
  timed in bash; the sender's clock is what orders a run.
- **A label per session**, in bash, so one process can hold several; Rust is
  never told it.

## The tools are compositions, not special cases

| | its bash | its reading |
|---|---|---|
| `bashcap` | the walk, plus `BASHCAP`'s effect | one JSON object per snapshot, streamed |
| `bashprof` | the walk, plus `BASHPROF_TIMETHIS`'s effect | three passes: records, tree, timings |

Neither is privileged. Both ship the words a call site says as a file that is
*both* injected and vendored, so a client's copy and the tool's cannot drift —
the words name a hook, and only the hook exists twice. A script with the words
and no tool runs unprofiled; the same script under the tool measures itself. See
[vendoring.md](vendoring.md).

## What is deliberately absent

| | why |
|---|---|
| a session-wide accumulator in the library | what a run produces is the client's; `Vec<Message>` and `()` are the only two shipped |
| a timer, an interval, a heartbeat | serving ends when nobody who could speak is left, and that is a descriptor; tokio's `time` feature is not enabled |
| a closing word or reserved payload word | the handle says when it is over, so nothing in the loop intercepts a message |
| a way in the core prefers | the core exports nothing and runs no initiation; every environment is the run's closure, every join a stated line, and `--reach` a tool's vocabulary |
| a poisoned or degraded mode | an answer that says no is a command returning non-zero, like any other |
| parallelism | concurrency is a task per shell on one thread; the cost is bash's `printf`, not ours, and a `Send` bound would tax every implementor for nothing |
| a fork tree | a fork inherits and then takes its own pipe; that it descends from a shell is not reported |
| a schema or IDL | an arglist has no shape to agree on |

## See also

- [`rigs.md`](rigs.md) — `Rig`, `Reacting`, and the two roles
- [`wire.md`](wire.md) — the protocol, line by line
- [`measurements.md`](measurements.md) — every number above
- [`scoping.md`](scoping.md) — where a name binds in the shipped bash

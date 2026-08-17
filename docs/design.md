# Design

What `src/` is, what each layer knows, and where the shape comes from. The
chapter-by-chapter reference is the rest of [this book](README.md); this
document sits above it.

## What it is for

Run a bash program, hear every shell in its process tree, and answer the
questions those shells ask, while the program behaves as it does when nothing
is listening.

That last clause is the hard part. A subject script has its own
traps, its own `IFS`, its own shell options and its own exit status, and a
tool that disturbs any of them measures something other than the program. The
design is organised around what the instrumentation may not touch, and every
capability is built from what remains.

## The layers

```
bash-strings           the quoted forms: @Q, @A, declare -p, Cursor
    └── shell          a shell's account of itself
         ├── stack     bash's five parallel arrays, read back
         └── rig       the session: a workspace, a pipe and a task per shell
```

| | knows about | never knows about |
|---|---|---|
| `bash-strings` (its own crate) | bash's quoted forms — `@Q`, `@A`, `declare -p` | everything else |
| `shell` | `bash-strings` | how a shell was reached, what it went on to say |
| `stack` | `bash-strings`, `shell` | the wire, the rig, any tool |
| `rig` | `bash-strings`, `shell` | the stack, any tool |

`stack` and `rig` are siblings, and neither calls the other. A tool composes
them: the frame walk goes into the bash a rig injects, through
`stack::with_walk`. `bashcap` and `bashprof` are that composition, and a third
tool would be the same one with different words.

Both stand on `shell`, because a walk cannot be read without knowing the shell
it was taken in. Bash writes `$0` into `BASH_SOURCE` for code it was given
rather than read from a file, and `main` there for anything defined at an
interactive prompt — words a script can also produce. Telling those apart is a
property of the shell, and the shell is what knows it.

## Messages are arglists

`BC_INSTR L say a b c` ships three words, and a rig receives three words. Any
width, zero included, and the protocol reads no position of one.

Several tools can therefore share one wire. The sender picks its own leading
discriminator — `TIMETHIS`, `__BASHCAP__` — and a decoder opts in with
`line.behind(TAG)`, receiving `None` for somebody else's message. There is no
registry and nothing to coordinate.

An answer is an arglist too, and the shell that asked runs it as a command in
its own frame — bash parses the reply as an array literal and invokes it, with
no `eval` involved. `["return", "1"]` refuses, `["declare", "-g", "x=1"]` sets
a variable in the asking shell, `["source", path]` runs bash of any length the
rig wrote to a file, `["exit", "9"]` ends the subject, and `["echo", value]`
hands a value back through a command substitution the script already wrote.

Two consequences follow, and together they are why there is no second protocol
here. Bash supplies the expressiveness, so no answer type has to grow. And
since every shell sources `<dir>/rig.bash` on the way in, a reply can call a
function the rig defined and pass it arguments computed in Rust, which makes
the rig's own bash the vocabulary an answer selects from.

## Values travel as bash's own quoted forms

`${x[*]@Q}` and `"(${x[*]@Q})"` on the way out, `parse_array` on the way in;
`declare -a x="$msg"` and `emit_array` the other way. Both sides speak the
notation bash already has, so word boundaries, newlines, tabs and bytes bash
cannot display survive without a length prefix, an escape scheme, or a
dependency on either side's idea of encoding.

That layer stands on nothing else and is usable on its own; see
[bash-strings: values](https://bashmgmt.github.io/bash-strings/values.html).

## One coordinate, owned

The workspace directory is the session's address and its only coordinate.
Every fifo and file is `<dir>/…`, modelled in one place by `Layout`, a
validated directory with accessors for the constant names.

The session owns the directory it serves. `<dir>/lock` is `flock`ed before
anything in the directory is touched and held until the fifos are gone. That
ownership makes three promises cheap: a second session on the same directory
is refused whole, a killed predecessor's leavings are swept at the next open
because the kernel released the dead lock, and the join fifo's presence is a
truthful liveness signal. A prescribed directory must already exist; making it
is the host's job in both roles.

### The two laid files

The session lays the generic prelude, shipped verbatim, reading neither its
own location nor the environment; and the rig's bash, `Rig::bash(&Layout)`.
Both hold definitions only and are inert to source.

Initiation is a line of client code: `BC_JOIN <label> <dir> [word…]`, with
zero, one or many labels. The label is client vocabulary, a write-time-stable
name the words speak, bound to a run-time coordinate at the join; Rust is
never told it. Words after the directory ride the announcement and land on
`Shell::brought`.

A standard initiation line is data the wrapper supplies, and the tools export
theirs as a function beside their rig. The core never runs it. It is written
into a provisioned startup file, or said by a client's own line. `<dir>/bash_env.bash`
is the one file that may initiate, and `Layout::bash_env(provision)` is the
only thing that writes it: the two sources, then the joining line when
`Provision::Joining` was asked for.

### Driven runs

A driven run starts the command line and owns a workspace of its own, either
a temporary directory or, with `run_at`, one the caller made and keeps.
Nothing external prescribes or collides with it.

How the shells reach the session is stated at the run. `run` and `run_at` take
an environment closure — fallible, because provisioning writes a file — and
what it returns becomes the subject's entire environment delta. The core
exports nothing on its own.

`Layout::bash_env(Provision::Joining(…))` is the usual pair, and it joins
every non-interactive bash in the tree. This is what makes `bashcap run --into
out make test` work, with every recipe shell `make` starts joining by itself.

Each tool decides for itself how to carry the workspace in a named variable,
and spells that name in its own binary — `BASHCAP_INIT "$BASHCAP_SESSION"`
where a by-hand script says so. `--reach` is the tools' vocabulary over these
spellings: `bash-env` provisions a joining file, `by-hand` a definitions file
with initiation left to the scripts.

`BASH_ENV` is a single variable, so two driven runs nested through it shadow
each other for the inner subtree. The way around it is a definitions-only
provision, which is the tools' `--reach by-hand`, and the choice is the
client's.

The command line is whatever the caller wrote, program included: `&["env",
"TARGET=staging", "bash", "x.bash"]` needs no support from the run.

### Served runs

A serving run takes its workspace from outside — `--at`, existing, with no
fallback — and answers to nobody. Nothing is written back, a serving
application is a complete standalone program, and the client feeds the same
directory to start, probe, load and initiate.

The workspace shows whether anything is live. Its join fifo is present exactly
while a session serves, so one file test answers the question. The boundary
case is a server killed outright, whose stale fifo stands until its directory
is next opened or removed.

Across both roles the join is one line, either a provisioned file's or the
client's own, and each tool prints every way a script writes it under
`--help`, in its own words.

## A rig describes; a reaction is per shell, and a task

```rust
// abridged — rigs.md quotes the real declarations
trait Rig      { type Reaction: Reacting;  bash(&Layout) -> String;
                 async joined(&Layout, Arc<Shell>) }
trait Reacting { type Kept;  async hear(Message);  async answer(Message) -> Answer;  async finish() -> Kept }
```

Every shell has a pipe of its own, so which shell said something is which pipe
it came out of, and every pipe has a task of its own: read a line, react,
maybe answer, until end of input.

A shell announces itself with its account — which bash, how it was given its
code, where it sits, what it had switched on — on the control fifo, before its
pipe is opened, so the run knows all of that before releasing the shell. None
of it changes while the shell lives: a subshell gets its own `$BASHPID` and
joins as a shell of its own, and `set` refuses `-i`, `-c` and `-s`. It is said
once, and the reaction built from it holds it as a member from construction.
Holding a reaction is therefore proof that its shell announced itself, and a
message reaches it only down that shell's own pipe.

The session is single-threaded and concurrent: one `current_thread` runtime,
`spawn_local` per shell, and no `Send` bound anywhere. What one shell's
reaction awaits — a slow answer, a 100 KB reply, a file opened at `joined` —
holds up that shell alone. `Rc<RefCell<_>>` is how a share is passed, and the
borrow is never held across an `.await`.

What comes back is one entry per shell, `Attended { shell, kept, parted }`,
where the shape carries the provenance. `heard` flattens it into the order it
was said, by the sending shells' own clocks, when a reading wants the run
whole.

Neither trait has a default body, so an implementor decides every case in
view. `Answer::unknown()` names the refusal — `return 127`, bash's own command
not found — and puts it where it applies.

What several shells share, such as a sink or a merged view, belongs to the
rig, which hands each reaction a share. The core names no sharing discipline.

## Who started the shells is a separate question

`Driving` runs a command line and owns its process group. `Serving` lays the
session in the workspace the client prescribed and serves while that client
holds the handle. Both are traits extending `Rig` with one provided `async
fn`, so a rig declares which orchestrations it supports by implementing them,
and its reaction is the same code either way. A program built on the core
exposes the pair as two symmetric verbs; the tools spell them `run` and
`serve`, with each verb's role table in its own book.

A session lasts as long as anyone who could still speak. `Watch` is a
descriptor — a pidfd, or the handle an initiator holds — and it is only
watched. Signalling and reaping belong to whoever started the thing being
watched, which is never the session, and that is what lets one session serve
both roles. Under `Driving` the group is killed before the session closes, so
every task reads what its shell wrote up to the kill.

Nothing inside a rig ends a session. A rig reacts, and a `Failure` from it
reports that it could not do its work.

## The subject keeps everything of its own

| | |
|---|---|
| no trap installed | a client's `trap … EXIT` fires as it would unwrapped |
| no builtin shadowed | `printf`, `read`, `exec` mean what they mean |
| no variable exported | nothing leaks into a child that did not join |
| no name outside `BC_*`/`__BC_*` | a subject's globals cannot collide with ours |
| no `set -o` change | `errexit`, `nounset`, `pipefail` stay as the subject set them |
| no `eval` | nothing of the subject's is re-parsed |
| its own exit status | a wrapped script is indistinguishable from an unwrapped one |

One exception: `expand_aliases` is turned on and stays on, because the error
guards are aliases — `return` has to act in the frame that failed. `IFS` is
taken `local` inside two of the protocol's own functions so `[*]` joins with a
space, and the subject's, unset included, is back on return.

The protocol may not use `set -e`, so every command in it that can fail is
followed by `|| __BC_BAIL` or `|| __BC_THROW`. A fault of ours is reported at
the subject's call site with status 125, which is what `env` and `timeout`
return when the wrapper rather than the payload failed, and the script carries
on rather than dying mid-message.

## What the transport gives every tool

Provenance, ordering, subshell capture, lifetimes and a control channel, none
of which a tool implements again.

Every shell has a pipe of its own, made by the shell, announced with its
account, and opened by the run. The blocking open is the rendezvous and end of
input is the goodbye. A `( … )` or `$( … )` that speaks takes a pipe of its
own on its first word.

A line is a message. One writer per pipe, so nothing interleaves and no write
need be atomic, and the pipe carries no framing. The control fifo has many
writers and one reader, so an announcement is sent as frames of at most
`PIPE_BUF` bytes, keyed by the shell's token and reassembled in bytes.

Every message carries both clocks, the sending shell's `$EPOCHREALTIME` and
the run's own. A span is the interval between two of them, which is why
nothing is timed in bash, and the sender's clock is what orders a run.

A label belongs to a session in bash, so one process can hold several. Rust is
never told it.

## The tools are compositions

| | its bash | its reading |
|---|---|---|
| `bashcap` | the walk, plus `BASHCAP`'s effect | one JSON object per snapshot, streamed |
| `bashprof` | the walk, plus `BASHPROF_TIMETHIS`'s effect | three passes: records, tree, timings |

Neither ships a file to a client. The words arrive with the session's own
bash, as everything else does. A committed call site makes its tool a
dependency of the script that says it: outside a session the word is a missing
command, loudly, in the same way an unjoined label reports 125.

## Not provided

| | what stands in its place |
|---|---|
| a session-wide accumulator in the library | what a run produces belongs to the client; `Vec<Message>` and `()` are the two shipped |
| a timer, an interval, a heartbeat | serving ends on a descriptor, so tokio's `time` feature is not enabled |
| a closing word or reserved payload word | the handle says when it is over, and nothing in the loop intercepts a message |
| a way in that the core prefers | every environment comes from the run's closure, every join from a stated line, and `--reach` is a tool's own vocabulary |
| a poisoned or degraded mode | an answer that says no is a command returning non-zero, like any other |
| parallelism | a task per shell on one thread; the cost sits in bash's `printf`, and a `Send` bound would tax every implementor |
| a fork tree | a fork inherits and then takes its own pipe; its descent is not reported |
| a schema or IDL | an arglist has no shape to agree on |

## See also

- [`rigs.md`](rigs.md) — `Rig`, `Reacting`, and the two roles
- [`wire.md`](wire.md) — the protocol, line by line
- [`measurements.md`](measurements.md) — every number above
- [`scoping.md`](scoping.md) — where a name binds in the shipped bash

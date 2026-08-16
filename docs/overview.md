# Overview

You have a bash program — a build script, a `make test` run, a deploy — and
you want to hear what happens inside it while it runs: which functions ran,
what a variable held at some moment, how long a step took. You do not want
to change how the program behaves while you listen, and you may want to
*answer* it too: let the running script ask a question and act on the reply.

That is what this crate does. This chapter walks the whole model once, at
reading pace; every later chapter is a close-up of one part.

## The two sides

There are always two sides, and they meet in the filesystem:

- **The bash side** is the program under instrumentation — the **subject**
  — and every bash process in its tree that takes part. A process that
  takes part is called a **shell**, and processes count separately: a
  subshell `( … )`, a command substitution `$( … )`, a `bash -c`, a child
  script — each is a shell of its own, because each is its own process
  with its own state.

- **The Rust side** is the **session**: it owns a directory (the
  **workspace**), listens there, and runs one small event loop. For every
  shell that joins, it builds a **reaction** — your code — and from then
  on that shell and that reaction talk over a pipe of their own.

Nothing else connects them. There is no daemon, no socket, no environment
protocol; a shell finds a session because somebody told it the workspace
directory, and everything after that happens in files under that directory.

## What a shell says, and what it can be told

Once joined, a script speaks through exactly one word, in two moods:

```bash
BC_INSTR DEPLOY say REC compiled "$target"    # ship these words; carry on
BC_INSTR DEPLOY ask which-target              # ship, block, run the reply
```

A `say` is fire-and-forget: the words travel to the Rust side as one
**message** and the script continues immediately. An `ask` blocks: the
message travels, your reaction decides an **answer**, and the answer comes
back — not as data, but as *a command the shell runs*. `["declare", "-g",
"target=staging"]` sets a variable in the asking shell; `["source",
path]` runs a file of any length; `["return", "3"]` refuses with a status.
The `ask`'s own exit status is the answer's, so plain `if BC_INSTR … ask …`
logic works.

A message carries an **arglist** — the words exactly as the caller wrote
them, any number of them, boundaries preserved — plus the verb and two
clocks (the shell's own `$EPOCHREALTIME`, and the session's clock at the
read). There is no schema: the first word is whatever convention your rig
and your scripts agree on, which is what lets several tools share one
session without coordinating.

The word between `BC_INSTR` and the verb — `DEPLOY` above — is the
**label**. It is pure bash-side vocabulary: a lookup key that binds a name
your scripts use to the workspace they joined, so one process can even hold
several sessions at once. The Rust side is never told the label; it only
ever sees which pipe a message came out of.

## Joining: definitions, then initiation

How does a shell come to be joined? Two steps, deliberately separate:

1. **Loading** brings the *definitions* in: `source
   <dir>/prelude.bash` (the protocol's words — `BC_JOIN`, `BC_INSTR`),
   then `source <dir>/rig.bash` (your rig's words). Both files are laid by
   the session and both are inert — sourcing them defines functions and
   changes nothing else.

2. **Initiation** opens the channel: one line, `BC_JOIN LABEL <dir>`,
   usually wrapped in an init function the rig defined. This is the moment
   the shell announces itself and gets its pipe.

Who says that `BC_JOIN` line? **Client code, always** — your script, at a
place it chooses — with exactly one stated exception: a run may
*provision* a startup file (`<dir>/bash_env.bash`, pointed to by
`BASH_ENV`) and declare whether that file initiates or only defines. That
is how a driven run reaches programs that have never heard of the session:
bash sources `BASH_ENV` in every non-interactive shell as it starts, so
the whole process tree joins with zero cooperation. The full menu of ways
in — each as a complete script — is [joining.md](joining.md).

## The workspace

Everything meets in one directory. After a session opens it looks like
this:

```
<dir>/
├── lock            flock()ed by the session for its whole life
├── prelude.bash    laid: the protocol's words       (definitions only)
├── rig.bash        laid: your rig's words           (definitions only)
├── bash_env.bash   provisioned on request: the startup file for BASH_ENV
│
├── join            fifo: every shell announces itself here, once
├── up.<token>      fifo, one per shell: its messages, one per line
└── rep.<token>     fifo, one per shell: answers to its asks
```

The directory is the session's **address** — the one coordinate anybody
needs — and the session **owns** it: the `lock` is taken (`flock`) before
anything else is touched and released only after the fifos are gone. That
buys three things worth naming now. A second session on the same directory
is refused instead of corrupting the first. A session killed outright
leaves fifos behind, but the *next* open sweeps them safely, because the
kernel released the dead session's lock. And the `join` fifo exists
exactly while a session serves — so "is something serving at `<dir>`?" is
one file test: `[[ -p <dir>/join ]]`.

## One shell, start to finish

Follow a single shell through its life:

```
 the shell (bash)                              the session (Rust)
 ───────────────                               ─────────────────
 sources prelude.bash, rig.bash                waits on the join fifo
 BC_JOIN LABEL <dir>
   1. writes its announcement ──── join ────►  reads the announcement,
      (its "account": which bash,              builds Shell from it,
       how started, options,                   awaits your Rig::joined
       the words the join brought)             → your Reaction exists
   2. blocks opening up.<token> ◄─ open ─────  opens the pipe: the shell
      …unblocked: it is joined                 is admitted; a task starts

 BC_INSTR L say words…  ───────── up.<token> ► task reads a line
                                               → your hear(message)
 BC_INSTR L ask words…  ───────── up.<token> ► → your answer(message)
   blocks reading rep    ◄─────── rep.<token>  writes the answer command
   runs the answer; its status is the ask's

 exits (or just stops talking)                 pipe reaches end of input
                                               → your finish() runs
                                               → Attended { shell, kept }
```

Three things in this picture do a lot of quiet work. The blocking open in
step 2 is a **rendezvous**: the shell cannot proceed until the session has
its pipe open, so even a shell that says one thing and exits within
microseconds loses nothing. The **account** travels *with* the
announcement, so by the time your reaction is built, everything knowable
about the shell — which bash, how it was invoked, what options it had on,
the extra words its join carried — is already in your hands as `Shell`,
and none of it can change while the shell lives. And because every shell
has its own pipe and its own task, provenance is structural: which shell
said something is simply which pipe it arrived on, and a slow reaction
delays nobody but its own shell.

## How it ends

A session lasts exactly as long as **anyone who could still speak**. The
thing it watches is a file descriptor — under a driven run, a pidfd on the
subject; under a served one, a handle the initiating script holds — and
the session only ever *observes* it. When the watch fires, a driven run
kills the process group it started and reaps it; a served run kills
nothing, because it started nothing. Then the session closes: every task
reads what its pipe still holds, every reaction finishes, the fifos are
removed, the lock is released last. Nothing inside a rig ends a session —
a `Failure` from your code means "I could not do my work", and the session
still closes cleanly on the way out.

## What the subject keeps

The listening must not change the program. The shipped bash installs no
trap, shadows no builtin, exports no variable, takes no name outside
`BC_*`/`__BC_*`, changes no `set -o` option, never uses `eval`, and leaves
the subject's exit status its own. The one deliberate exception: it turns
`expand_aliases` on (its error guards must be aliases, because `return`
has to act in the frame that failed). Each of these claims has a
wire-level proof behind it — the table in
[measurements.md](measurements.md#what-the-proofs-establish) lists them
one by one.

## Vocabulary, on one card

The terms above, and the Rust name each maps to:

| term | what it names |
|---|---|
| **subject** | the bash program under instrumentation: the command line a driven run starts, or the script that started a server |
| **shell** | one bash process that joined; `Shell` |
| **session** | one run: a workspace, a control fifo, a pipe and a task per shell, until the watch fires |
| **workspace** | the session's directory and address, locked for its life; modelled by `Layout` |
| **label** | the bash-side key binding a name to a joined workspace; Rust never sees it |
| **rig** | your description: definitions, standard initiation, how a reaction is built; `Rig` |
| **reaction** | your per-shell counterpart, run as a task of its own; `Reacting` |
| **message / answer** | one arglist a shell shipped / one command a blocked shell runs; `Message`, `Answer` |
| **account** | what a shell says of itself when announcing; becomes `Shell` |
| **kept** | what a reaction leaves behind; `Reacting::Kept`, landing in `Attended::kept` |
| **driving / serving** | who started the shells: Rust owns a command line, or a bash script holds the handle; `Driving`, `Serving` |
| **provision** | what a `bash_env.bash` does about the channel — joins, or only defines; `Provision` |
| **watch** | the descriptor a session ends on; observed, never signalled |

## The two tools, and reading on

`bashcap` (a full shell snapshot at every call site) and `bashprof` (a
timed call tree) are built from nothing but this crate's public surface —
each is a rig plus a reading, each with its own book in its own
repository. A third tool would be the same composition with different
words.

From here: [design.md](design.md) states the decisions this shape follows
from. [rigs.md](rigs.md) is the API you implement. [driving.md](driving.md)
and [serving.md](serving.md) are the two orchestrations,
[joining.md](joining.md) every way in, [wire.md](wire.md) the protocol
underneath. The full Rust surface is rustdoc's: `cargo doc --no-deps
--open`.

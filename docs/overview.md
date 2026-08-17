# Overview

You have a bash program — a build script, a `make test` run, a deploy — and
you want to hear what happens inside it while it runs: which functions ran,
what a variable held at some moment, how long a step took. You want the
program to behave the same while you listen, and you may want to answer it,
letting the running script ask a question and act on the reply.

This chapter walks the whole model once. Every later chapter is a close-up of
one part.

## The two sides

There are two sides, and they meet in the filesystem.

The bash side is the program under instrumentation, called the subject, and
every bash process in its tree that takes part. A process that takes part is
a shell, and processes count separately: a subshell `( … )`, a command
substitution `$( … )`, a `bash -c`, a child script — each is its own process
with its own state, so each is its own shell.

The Rust side is the session. It owns a directory, the workspace, listens
there, and runs one small event loop. For every shell that joins it builds a
reaction, which is your code, and from then on that shell and that reaction
talk over a pipe of their own.

Nothing else connects the two. There is no daemon, no socket, no environment
protocol. A shell finds a session because somebody told it the workspace
directory, and everything after that happens in files under that directory.

## What a shell says, and what it can be told

Once joined, a script speaks through one word in two moods:

```bash
BC_INSTR DEPLOY say REC compiled "$target"    # ship these words; carry on
BC_INSTR DEPLOY ask which-target              # ship, block, run the reply
```

`say` ships the words as one message and the script continues immediately,
since nothing is waiting on it. `ask` blocks until your reaction replies.

A message carries an arglist — the words exactly as the caller wrote them, any
number of them, boundaries preserved — plus the verb and two clocks, the
shell's own `$EPOCHREALTIME` and the session's clock at the read. There is no
schema. The first word is whatever convention your rig and your scripts agree
on, which lets several tools share one session without coordinating.

The word between `BC_INSTR` and the verb, `DEPLOY` above, is the label. It is
bash-side vocabulary: a lookup key binding a name your scripts use to the
workspace they joined, so one process can hold several sessions at once. The
Rust side is never told the label. It sees which pipe a message came out of.

### The reply is a command

What comes back from an `ask` is also a list of words, and the asking shell
parses it with bash's own array syntax and then invokes it, in the frame that
asked:

```bash
local -a __bc_answer="$__bc_line"    # the reply, read as a bash array literal
"${__bc_answer[@]}"                  # and invoked, right here
```

No `eval` takes part. Bash reads an array literal — the notation `declare -p`
prints — and calls the result.

Replying with a command rather than a value is where the generality comes
from, because running one command in the caller's frame already spans what a
richer protocol would need types for:

| the reply | what the asking shell does |
|---|---|
| `["echo", "/usr/lib"]` | prints it, so `x=$(BC_INSTR … ask …)` captures a value |
| `["declare", "-g", "target=staging"]` | sets a variable in its own process |
| `["return", "3"]` | returns 3, so `if BC_INSTR … ask …` branches on the reply |
| `["source", "/tmp/x.bash"]` | runs a file of any length the rig just wrote |
| `["exit", "9"]` | ends the subject |

The `ask` exits with the status of whatever ran, so a reply that says no
arrives as an ordinary shell failure the script can test.

The command need not be a builtin. `<dir>/rig.bash` holds bash your rig wrote,
and every shell sources it on the way in, so a reply may call a function you
defined there and pass it arguments your Rust code computed. The rig supplies
the vocabulary; the reply picks a word from it. That is the whole control
channel, and it needs no `eval`, no reserved words and no second protocol.

## Joining: definitions, then initiation

A shell comes to be joined in two steps.

Loading brings the definitions in. `source <dir>/prelude.bash` defines the
protocol's words, `BC_JOIN` and `BC_INSTR`; `source <dir>/rig.bash` defines
your rig's. The session lays both files, and both are inert — sourcing them
defines functions and changes nothing else.

Initiation opens the channel. One line, `BC_JOIN LABEL <dir>`, usually
wrapped in an init function the rig defined. At this line the shell announces
itself and gets its pipe.

Client code says that line, at a place it chooses. The one exception is that
a run may provision a startup file, `<dir>/bash_env.bash`, pointed to by
`BASH_ENV`, and declare whether that file initiates or only defines. This is
how a driven run reaches programs that have never heard of the session: bash
sources `BASH_ENV` in every non-interactive shell as it starts, so the whole
process tree joins without cooperating. [joining.md](joining.md) gives every
way in, each as a complete script.

## The workspace

After a session opens, its directory looks like this:

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

The directory is the session's address, the one coordinate anybody needs, and
the session owns it. The `lock` is taken with `flock` before anything else is
touched and released after the fifos are gone.

Three consequences follow. A second session on the same directory is refused
rather than corrupting the first. A session killed outright leaves its fifos
behind, and the next open sweeps them safely, because the kernel released the
dead session's lock. And the `join` fifo exists exactly while a session
serves, so `[[ -p <dir>/join ]]` answers whether one is up.

## One shell, start to finish

```
 the shell (bash)                              the session (Rust)
 ───────────────                               ─────────────────
 sources prelude.bash, rig.bash                waits on the join fifo
 BC_JOIN LABEL <dir>
   1. writes its announcement ──── join ────►  reads the announcement,
      (its account: which bash,                builds Shell from it,
       how started, options,                   awaits your Rig::joined
       the words the join brought)             → your Reaction exists
   2. blocks opening up.<token> ◄─ open ─────  opens the pipe: the shell
      …unblocked: it is joined                 is admitted; a task starts

 BC_INSTR L say words…  ───────── up.<token> ► task reads a line
                                               → your hear(message)
 BC_INSTR L ask words…  ───────── up.<token> ► → your answer(message)
   blocks reading rep    ◄─────── rep.<token>  writes the answer command
   runs the answer; the ask exits with it

 exits (or just stops talking)                 pipe reaches end of input
                                               → your finish() runs
                                               → Attended { shell, kept }
```

The blocking open in step 2 is a rendezvous. The shell cannot proceed until
the session has its pipe open, so a shell that says one thing and exits
within microseconds still gets heard.

The account travels with the announcement. By the time your reaction is
built, what is knowable about the shell — which bash, how it was invoked,
what options it had on, the extra words its join carried — is in your hands
as `Shell`, and none of it changes while the shell lives.

Each shell has its own pipe and its own task. Which shell said something is
which pipe it arrived on, and a slow reaction delays only its own shell.

## How it ends

A session lasts as long as anyone who could still speak. What it watches is a
file descriptor: under a driven run a pidfd on the subject, under a served one
a handle the initiating script holds. The session only observes it. When the
watch fires, a driven run kills the process group it started and reaps it; a
served run kills nothing, having started nothing. Then the session closes.
Every task reads what its pipe still holds, every reaction finishes, the fifos
are removed, and the lock is released last.

Nothing inside a rig ends a session. A `Failure` from your code reports that
your code could not do its work, and the session still closes cleanly.

## What the subject keeps

The shipped bash installs no trap, shadows no builtin, exports no variable,
takes no name outside `BC_*` and `__BC_*`, changes no `set -o` option, never
uses `eval`, and leaves the subject's exit status alone. It turns
`expand_aliases` on, because its error guards are aliases: `return` has to act
in the frame that failed. Each claim has a wire-level proof behind it, listed
one by one in
[measurements.md](measurements.md#what-the-proofs-establish).

## Vocabulary

| term | what it names |
|---|---|
| subject | the bash program under instrumentation: the command line a driven run starts, or the script that started a server |
| shell | one bash process that joined; `Shell` |
| session | one run: a workspace, a control fifo, a pipe and a task per shell, until the watch fires |
| workspace | the session's directory and address, locked for its life; modelled by `Layout` |
| label | the bash-side key binding a name to a joined workspace; Rust never sees it |
| rig | your description: definitions, and how a reaction is built; `Rig` |
| reaction | your per-shell counterpart, run as a task of its own; `Reacting` |
| message / answer | one arglist a shell shipped / one command a blocked shell runs; `Message`, `Answer` |
| account | what a shell says of itself when announcing; becomes `Shell` |
| kept | what a reaction leaves behind; `Reacting::Kept`, landing in `Attended::kept` |
| driving / serving | who started the shells: Rust owns a command line, or a bash script holds the handle; `Driving`, `Serving` |
| provision | what a `bash_env.bash` does about the channel: joins, or only defines; `Provision` |
| watch | the descriptor a session ends on; observed, never signalled |

## The two tools, and reading on

`bashcap`, a full shell snapshot at every call site, and `bashprof`, a timed
call tree, are built from this crate's public surface. Each is a rig plus a
reading, each with its own book in its own repository, and a third tool would
be the same composition with different words.

From here: [design.md](design.md) states the decisions this shape follows
from. [rigs.md](rigs.md) is the API you implement. [driving.md](driving.md)
and [serving.md](serving.md) are the two orchestrations,
[joining.md](joining.md) every way in, and [wire.md](wire.md) the protocol
underneath. The full Rust surface is in rustdoc: `cargo doc --no-deps --open`.

# Overview

A **rig** describes an instrumented run of bash: the definitions the subject's
shells get, the standard way a shell joins, and how a **reaction** is built
once one is there. A **session** carries it out: a workspace directory the
session owns, a control fifo announcements arrive on, and a pipe and a task
per shell, until the watch fires. Everything a shell says is one **message**
— an arglist, a verb, two clocks — and everything it is told is one
**answer** — a command it runs.

Two words in bash, and a script never says anything else of the protocol's:

```bash
BC_INSTR LABEL say a b c   # ship the arglist and return
BC_INSTR LABEL ask a b c   # ship it, block, run the answer; its status is the answer's
```

Both name a **label** — the write-time-stable name a client's words speak,
bound to a workspace by `BC_JOIN LABEL <dir> [word…]`, which is the channel
initiation. Who says that join line is the whole question of joining, and the
answer is: **client code, always** — except where a run explicitly provisions
a startup file that says it, the one auto-initiation there is. See
[joining.md](joining.md).

## Vocabulary

| term | what it names |
|---|---|
| **subject** | the bash program under instrumentation: the command line a driven run starts, or the script that started a server |
| **shell** | one bash process that joined. A `( … )`, a `$( … )`, a `bash -c` are each a shell of its own; `Shell` is what one is |
| **session** | one run: a workspace, a control fifo, a pipe and a task per shell, until the *watch* fires |
| **workspace** | the directory holding the session's files and fifos — the session's one coordinate and its address, locked (`<dir>/lock`) for the session's life. `Layout`; a prescribed one exists before the session does |
| **label** | the word after `BC_INSTR` and `BC_JOIN`: a bash-side lookup key, so one process can hold several sessions; Rust is never told it |
| **rig** | a description: definitions, standard initiation, and how a reaction is built. `Rig` |
| **reaction** | what one shell talks to, for as long as it can speak. `Reacting`, made per shell by `Rig::joined`, run as a task of its own |
| **message** | one arglist a shell shipped, with the verb (`say`/`ask`) and two clocks. `Message` |
| **answer** | one command a blocked shell is told to run. `Answer` |
| **account** | what a shell says of itself when it announces: which bash, how it was started, what it had on, and the words its join brought. It becomes `Shell` |
| **kept** | what a reaction leaves behind when its shell is gone. `Reacting::Kept`; `Attended::kept` |
| **driving / serving** | who started the shells: Rust ran a command line and owns it, or a bash script started the server and holds the handle. `Driving`, `Serving` |
| **environment** | the run's, whole: `run(argv, environment)` takes a fallible closure over the settled `Layout`, and its return is everything the subject's environment gets |
| **provision** | what a `bash_env.bash` startup file does about the channel — `Joining` or `Definitions` — stated by whoever writes it. `Provision` |
| **watch** | the descriptor a session ends on — the subject's pidfd, or the handle a client holds. Observed, never signalled |

## The shape

```
 subject's process tree                     workspace <dir>/                   one current-thread runtime
 ────────────────────────                   ──────────────────                 ──────────────────────────
 bash ─ source <dir>/bash_env.bash ────►    bash_env.bash ── provisioned: sources the two below,
   │    (only where provisioned)                              then its stated joining, or not
   │                                        prelude.bash  ── generic: BC_JOIN, BC_INSTR
   │                                        rig.bash      ── Rig::bash — definitions only
   │                                        lock          ── flock()ed while the session lives
   │  BC_JOIN LABEL <dir> ── announce ─►    join          ── frames ──►  Session::serve ── Rig::joined ──┐
   │                ── exec {fd}>up.tok ►   up.<token>    ── lines ───►  attend task ── Reacting::hear    │
   │  BC_INSTR ask  ◄─ read <&rep ────────  rep.<token>   ◄── answer ──            ── Reacting::answer ◄─┘
   ├─ ( subshell )  ── its own token, pipe, task ──────►                           ── Reacting::finish → Attended
   └─ bash child    ── the same
```

Every shell has a pipe of its own, so which shell said something is which
pipe it came out of; every pipe has a task of its own, so what one shell's
reaction awaits holds up nothing but that shell. The session ends when the
watch fires; nothing inside a rig ends it.

## What the subject keeps

No trap installed, no builtin shadowed, no variable exported, no name outside
`BC_*`/`__BC_*`, no `set -o` change, no `eval`, its own exit status. The one
option turned on is `expand_aliases`. A wire-level proof stands behind each
of these — see [measurements.md](measurements.md).

## Guarantees a caller leans on

A message is a bash array literal on one line and arrives whole at any width
(one writer per pipe); a shell that says one thing and exits within
microseconds loses nothing (the blocking open is the rendezvous); `heard`
orders by the senders' clocks; a `Failure` from any reaction ends the run —
the subject is killed under `Driving`, released under `Serving`; a line the
protocol did not write ends the run naming it. What the proofs establish,
one by one, is the table in
[measurements.md](measurements.md#what-the-proofs-establish).

## The two tools

Neither `bashcap` nor `bashprof` is privileged; each is a rig plus a
reading, composed from this crate's public surface, and each carries its own
book in its own repository. A third tool would be the same composition with
different words.

## Reading on

[design.md](design.md) states the decisions this shape follows from;
[rigs.md](rigs.md) is the API; [driving.md](driving.md) and
[serving.md](serving.md) are the two orchestrations; the rest of the book is
the mechanics. The full Rust surface is rustdoc's:
`cargo doc --no-deps --open`.

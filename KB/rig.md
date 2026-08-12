# The rig — what a run is

`src/bash/rig/mod.rs` for the definition, `src/bash/rig/run.rs` for
performing it.

A run is a bash program executed under instrumentation. A **rig** says what
bash that run needs, what state it keeps, and how it reacts; **`run`** does
everything else.

## `Rig`

```rust
pub trait Rig {
    /// The client's state. No bounds, no lifetime: the run stores nothing of
    /// its own in it, and hands it back when the run is over.
    type Session;

    /// What the run needs before there is a shell to talk to.
    fn startup(&self) -> Startup;

    fn open(&self) -> Result<Self::Session, Failure>;

    /// A message nobody is waiting on.
    fn hear(&self, session: &mut Self::Session, said: Line) -> Result<(), Failure>;

    /// A message a shell is blocked on; the run frames what comes back and
    /// writes it to that shell.
    fn answer(&self, session: &mut Self::Session, asked: Line) -> Result<Answer, Failure>;

    /// The subject is gone; release what the session holds.
    fn end(&self, session: &mut Self::Session, status: ExitStatus) -> Result<(), Failure>;
}
```

**`open` is the only required method.** The rest default to: injecting
nothing, keeping nothing, hearing the question and telling the shell the word
is unknown (`return 127`), and doing nothing. A rig that ignores everything is
a session type and one line; a rig that only listens adds `hear`.

`Startup` is what a rig tells the run about the process before it exists:

```rust
#[derive(Default)]
pub struct Startup {
    /// Injected into every shell, after the protocol's own. The only half
    /// descendants see: `BASH_ENV` reaches them, a command line does not.
    pub bash: String,

    /// Added to the environment the subject is started with.
    pub env: Vec<(OsString, OsString)>,
}
```

**The command line is run as it is given, and carries its own program.**
`run(&rig, &["bash", "x.bash"])`, not `&["x.bash"]` — so a run is not bound to
bash at the top, and a caller wanting a launcher writes one into the argv it
passes. `&["make", "test"]` works too, and every bash `make` starts joins the
wire.

A rig has no say in the command line, and needs none: what it would want to
put there — a word, a variable, an option — reaches *every* shell through
`Startup`, which a command line cannot do.

`answer`'s default routes through `hear`, so a rig that does not answer still
keeps what it was asked.

A rig's bash and its decoder are one thing, and `Startup::bash` is where that
pairing is expressed: bashcap's rig hands over bashcap's bash, and neither can
be run without the other.

`Line` arrives **by value**: a session that keeps it does so without cloning,
one that ignores it drops it for free.

`&self` throughout — a rig is a description and is never mutated by running,
which is also why `Session` needs no lifetime. Anything a session might want
to borrow from the rig, a method reads off `&self` directly.

## What a session is for

**Tracking what a run produced is entirely the client's.** The library ships
no accumulator, no collection type, and no rig implementation. A session is
whatever the client says it is:

| | its `Session` | what it overrides |
|---|---|---|
| bashcap | `Capturing { written, sink }` | `startup`; `hear` decodes and writes; `end` flushes |
| `examples/snapshotting.rs` | `Vec<Capture>` | `startup`; `hear` decodes and keeps |
| `examples/answering.rs` | what it has heard | `startup`; `answer` decides from it |
| `bashprof` | `Vec<Line>` | `startup`; `hear` keeps. Every message carries its own provenance *and* the name of the call it was made inside of, so reading is one pass with a map, then two hylic folds — one to nest, one to read the tree as timings |
| `proofs/answering.rs` | `Soak { heard, answered }` | `startup`, `hear`, `answer`; the tally lives in the session because a rig is `&self` |
| `proofs/owning.rs` | `()` | `answer`, which never returns |

That last row is the point: a session is whatever the client says it is,
including nothing.

## `run`

```rust
pub fn run<R: Rig, S: AsRef<OsStr>>(rig: &R, argv: &[S])
    -> Result<Run<R::Session>, Failure>;

/// Where the run lays its own bash, and how long that outlives the run.
pub enum Workspace {
    Temporary,        // the default: made with the run, removed with it
    At(PathBuf),      // one of the caller's, created if absent and left behind
}

/// What a run produced.
pub struct Run<S> {
    pub session: S,
    pub subject: ExitStatus,
    pub failed: Option<Failure>,
}

impl<S> Run<S> {
    /// The session, if nothing went wrong.
    pub fn whole(self) -> Result<(S, ExitStatus), Failure>;
}
```

**Reaching a `Run` means bash got to its own end**, so `subject` is always the
subject's own status. `failed` is narrower than that: what went wrong *closing
up* — a message left half-read, or a session that would not let go. Both
happen after the subject is gone, which is why they travel beside a status
rather than replacing it.

A `Failure` in place of a `Run` means the run never got that far: it could not
be set up, or the rig could not do its work and the subject was killed — and
then how the subject *would* have ended is not something anyone can say. That
is the whole of the distinction, and it is why the status is not an
`Option`.

`whole()` is for callers that have no use for a partial reading.

The two arguments are the two real inputs. The run owns a directory for its
pipes and its prelude, the transport over them, and the subject's process
group. **None of it appears in any signature a rig sees**, and the session is
the only thing that crosses between them. An empty command line is a
`Failure`, not a panic.

`run` hands back the session because that is the client's, and the status
because the run is what called `wait`.

Internally the driver is a struct rather than a pile of locals:

```rust
struct Running<'r, R: Rig> {
    rig: &'r R,
    subject: Subject,
    session: R::Session,
    wire: Wire,
}
```

with `open` / `drive` / `serve` / `finish`. No state beyond what a run *is*.
A temporary workspace drops one frame above the run that read it, after
`finish` has reaped the subject. `Rig::workspace` is what decides: a rig whose
reading outlives the run names a directory it keeps, because the instrument's
own frames name a file in there and a source path is only as readable as the
file it names — see [stack.md](stack.md#where-a-source-path-lands).

**The field order is the drop order.** Leaving through `?` anywhere drops
`Running`, and `subject` first means the shell is stopped before the session it
was feeding is released.

`open` lays down the workspace and the pipe *before* asking the rig to open a
session, so the session is the last thing acquired before the subject exists
and nothing is held over a setup that failed.

## The loop

`Running::drive` and `serve`. There is no interval and no timer.

```rust
for line in self.wire.drain()? {
    let asking = line.pid;

    match line.kind {
        Kind::Say => self.rig.hear(&mut self.session, line)?,
        Kind::Ask => {
            let answer = self.rig.answer(&mut self.session, line)?;

            self.wire.answer(asking, answer)?;
        }
    }
}
```

A shell that asked is blocked until its answer is written, so writing it is
part of serving rather than something the caller does after. There is no catch
here and no state: a rig that returns `Err` leaves through `?` — see [when the
rig fails](#when-the-rig-fails).

The subject's exit is a readable descriptor — a `pidfd` — so one `poll` waits
on the pipe and on the child at once. A readable `pidfd` does not imply an
empty pipe: the pipe is checked first, and drained once more after the loop.

## The run owns its subject

```rust
struct Subject { child: Child, group: libc::pid_t, exit: OwnedFd }
```

Spawned with `process_group(0)`, so the subject and everything it starts are
one group. `finish` **kills the group and then reaps** — in that order,
because while the subject is unreaped its group cannot have been recycled, so
the signal cannot reach anything else. `Drop` does the same if the run left by
any other path; `Child::wait` caches its status, so doing it twice costs
nothing.

Reaping is `wait`, not `try_wait`: a killed child that is merely unreaped
still answers `kill(pid, 0)`, so a zombie would read as a survivor.

A shell that asks after the subject has exited can never be answered, and goes
with the run. A process that means to survive detaches with `setsid`, which
leaves the group.

## When the rig fails

**A rig that returns `Err` ends the run.** The failure leaves through `?`,
which drops `Running` — killing the subject's process group and reaping it —
and `run` yields that reason instead of a `Run`.

The subject is not told. There is nothing to tell it: an answer is a command,
and no command means *the operator broke*. Writing one would ask bash to
interpret a condition that is not its own, and a shell that then carried on
would be running against a rig that has already stopped working.

This is the line the design draws:

| | whose |
|---|---|
| a rig cannot hear, or cannot decide an answer | the run's — kill, and `Err` |
| an answer that returns non-zero | the subject's — `set -e`, `\|\|`, or ignore it |

**Every answer is the same kind of thing.** Saying no is a command returning
non-zero, exactly as saying yes is a command returning zero, and the run only
waits to see what bash makes of it. There is no refusal on the wire and no
category for one — `Answer` has `of` and `status`, and the default `answer`
says the word is unknown with `return 127` like any rig would.

Serving is therefore stateless: no flag, no poisoned mode, no second reading
of a message already handled. `serve` calls the rig and writes what comes
back.

None of this costs the status. `bashcap run` still exits with the subject's
code even when the capture broke, and says on stderr that it did — a wrapper
reporting its own trouble as the subject's would not be transparent.

## Status

```rust
pub enum ExitStatus { Code(u8), Signal(u8) }

impl ExitStatus {
    /// What a shell would report for it: `128 + n` for a signal.
    pub fn shell_code(self) -> i32;
}
```

Both fields are the width the kernel gives them. The conversion from
`std::process::ExitStatus` reads the two fields out of the raw wait status —
`WTERMSIG` is the low seven bits, `WEXITSTATUS` the second byte — so it is
total and invents nothing.

How a run went and how the subject ended are two facts, and the status is only
the second: `Run::failed` carries the first. A caller wanting both takes them
from the `Run`, and one wanting neither separately calls `whole()`.

No signal disposition is changed, so a subject's own handlers and a caller's
`SIGINT` both behave as they would unwrapped.

## One error, one way out

```rust
pub struct Failure { doing: String, cause: Box<dyn Error + Send + Sync> }

pub trait Doing<T> {
    fn doing(self, what: impl FnOnce() -> String) -> Result<T, Failure>;
}
```

`src/failure.rs`, crate-level rather than the rig's. A context and a cause
rather than an enum, since every use is `Display` or `source()`:

```
writing the prelude to /tmp/x/prelude.bash: Permission denied (os error 13)
```

There is no partial result: the first thing that cannot be read or written
ends the run, with `Subject`'s guard killing the subject on the way out. A
subject that exits non-zero, is signalled, or asks something the rig has no
useful answer to is an outcome, not an error.

## See also

- [wire.md](wire.md) — what `drain` hands back, and what an answer is
- [tree.md](tree.md) — views over what a session kept
- [stack.md](stack.md) — the frame walk any instrument can reuse
- [bashcap.md](bashcap.md) — a rig that streams instead of keeping

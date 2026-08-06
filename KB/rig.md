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

    /// The command line actually run, given the one the caller asked for.
    /// Identity by default.
    fn transform_command(&self, argv: Vec<OsString>) -> Vec<OsString>;

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
nothing, running the command line as asked, keeping nothing, hearing the
question and telling the shell the word is unknown (`return 127`), and doing
nothing. A rig that ignores everything is a session type and one line; a rig
that only listens adds `hear`.

The two that inform the run split by kind. `Startup` is **data** — what the
process needs before it exists:

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

`transform_command` is **behaviour** — a rig may put a launcher in front of
the command line, wrap the payload, or replace it outright. It cannot mislay
what the caller asked for by accident, because identity is the default.

**The command line carries its own program.** `run(&rig, &["bash", "x.bash"])`,
not `&["x.bash"]` — so a run is not bound to bash at the top. Instrumentation
travels by `BASH_ENV`, so `&["make", "test"]` works too, and every bash `make`
starts joins the wire.

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
| `proofs/answering.rs` | `Soak { heard, answered }` | `startup`, `hear`, `answer`; the tally lives in the session because a rig is `&self` |
| `proofs/owning.rs` | `()` | `answer`, which never returns |

That last row is the point: a session is whatever the client says it is,
including nothing.

## `run`

```rust
pub fn run<R: Rig, S: AsRef<OsStr>>(rig: &R, argv: &[S])
    -> Result<Run<R::Session>, Failure>;

/// The same, in a directory of your choosing, left behind to read.
pub fn run_in<R: Rig, S: AsRef<OsStr>>(rig: &R, at: &Path, argv: &[S])
    -> Result<Run<R::Session>, Failure>;

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

**`subject` is always the subject's own status**, because the run serves until
it leaves of its own accord — see [when the rig
fails](#when-the-rig-fails). `failed` is what went wrong on this side, if
anything, and the two are independent: a run can fail and still report exactly
how bash ended.

A `Failure` in place of a `Run` means something else entirely: the run never
got that far. It could not be set up, or it lost contact with the subject and
killed it, and then how the subject *would* have ended is not something anyone
can say. That is the whole of the distinction, and it is why the status is not
an `Option`.

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
    session: R::Session,
    subject: Subject,
    wire: Wire,

    /// Set the moment the rig fails, and the run ends in it rather than in a
    /// status.
    failed: Option<Failure>,
}
```

with `open` / `drive` / `serve` / `react` / `finish`. `run` owns the `TempDir`
and calls `run_in`, so the workspace drops one frame above the run that read
it — after `finish` has reaped the subject.

`open` lays down the workspace and the pipe *before* asking the rig to open a
session, so the session a rig hands over is the last thing acquired before the
subject exists. That is what makes `end` reachable for every run that started
one.

## The loop

`Running::drive` and `serve`. There is no interval and no timer.

```rust
for line in self.wire.drain()? {
    let waiting = line.pid;

    if let Some(answer) = self.react(line) {
        self.wire.answer(waiting, answer)?;
    }
}
```

`react` is where the rig is called and where a failure is caught — see [when
the rig fails](#when-the-rig-fails). It returns what to write back, if
anything: only a shell that *asked* is listening, so a `hear` that fails has
nowhere to reply and yields `None`.

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

A rig that returns `Err` **poisons the run**: it is not called again, every
ask from then on is refused with its reason, and `run` ends in that `Failure`
rather than a status.

The run does not stop at once, and that is the point. A shell blocked on an
ask waits forever unless it is told something, and — measured, 0 of 120 — a
subject killed immediately after the refusal is written **never delivers it**.
So the loop serves on until the subject leaves of its own accord, exactly as a
successful run does.

The blocked shell is answered with `__bc_refused`, which reports the reason at
the subject's own call site and returns 125. From the subject's side that is
an ordinary failing command: under `set -e` its script ends there, otherwise
it carries on with a status it can test. Nothing is forced on it.

`hear` has nobody waiting, so nothing can be said at the time — but the run
still ends in the failure, and the next shell to ask carries the reason.

A rig that has already failed is not asked to `end`, and a message left
half-read is treated as a consequence of whatever went wrong before it rather
than as news of its own: the first failure is the one reported.

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
- [bashcap.md](bashcap.md) — a rig that streams instead of keeping

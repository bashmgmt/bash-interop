# The rig — a reaction, and two orchestrations

`src/bash/rig/mod.rs` for the reaction, `master.rs` and `slave.rs` for the two
ways a session comes about, `serving.rs` for what they share.

A **rig** is the reaction inside the protocol it defines: the bash it gives the
subject, the state it keeps, and what it does with what arrives. Who started
what — and therefore who ends it and cleans up — is a second question, and each
of its two answers is a trait that extends `Rig` and carries its own
orchestration.

## `Rig`

```rust
pub trait Rig {
    /// The client's state. No bounds, no lifetime: the session stores nothing
    /// of its own in it, and hands it back when the conversation is over.
    type Session;

    /// The words this rig gives the subject, laid beside the protocol's own
    /// and sourced by it. The same text in either orchestration.
    fn bash(&self) -> String;

    /// Where the session's files go, and how long they outlive it.
    fn workspace(&self) -> Workspace;

    fn open(&self) -> Result<Self::Session, Failure>;

    /// A message nobody is waiting on.
    fn hear(&self, session: &mut Self::Session, said: Line) -> Result<(), Halt>;

    /// A message a shell is blocked on; the session frames what comes back and
    /// writes it to that shell.
    fn answer(&self, session: &mut Self::Session, asked: Line) -> Result<Answer, Failure>;

    /// The conversation is over; release what the session holds.
    fn end(&self, session: &mut Self::Session) -> Result<(), Failure>;
}
```

**`open` is the only required method.** The rest default to: no words of the
subject's own, a workspace thrown away, keeping nothing, telling the shell the
word is unknown (`return 127`), and doing nothing at the end. A rig that
ignores everything is a session type and one line; a rig that only listens adds
`hear`.

`Line` arrives **by value**: a session that keeps it does so without cloning,
one that ignores it drops it for free.

`&self` throughout — a rig is a description and is never mutated by running,
which is also why `Session` needs no lifetime.

A rig's bash and its decoder are one thing, and `Rig::bash` is where that
pairing is expressed: bashcap's rig hands over bashcap's bash, and neither can
be run without the other.

`Rig::workspace` decides how long that bash outlives the conversation. A rig
whose reading resolves frame sources afterwards names a directory it keeps,
because the instrument's own frames name a file in there and a source path is
only as readable as the file it names — see
[stack.md](stack.md#where-a-source-path-lands).

## `Halt`, and why only `hear` may raise it

```rust
pub enum Halt { Done, Failed(Failure) }
impl From<Failure> for Halt { … }
```

`Done` ends the conversation cleanly; `Failed` ends it as a fault. A `Failure`
raised anywhere in a rig's own code becomes `Failed` through `?`, so `Done` is
the only variant a rig ever writes out.

`answer` returns `Result<Answer, Failure>` and cannot halt. A shell that asked
is blocked in `read` on a fifo it holds read-write, so that read sees no end of
input and no timeout; the answer is what unblocks it. Under `Master` the group
is killed afterwards and the difference does not show, and under `Slave`
nothing kills anything. The signature carries the rule so no code has to.

The default `answer` therefore returns `Answer::status(127)` on its own. A rig
that wants questions kept implements `answer` and keeps them.

## `Master` — Rust orchestrates

```rust
pub trait Master: Rig {
    fn run<A: AsRef<OsStr>>(&self, argv: &[A]) -> Result<Run<Self::Session>, Failure>;
}

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

**The command line is run as it is given, and carries its own program.**
`rig.run(&["bash", "x.bash"])`, not `&["x.bash"]` — so a run is not bound to
bash at the top, and a caller wanting a launcher or an environment writes one
into the argv: `&["env", "TARGET=staging", "bash", "x.bash"]`. `&["make",
"test"]` works too, and every bash `make` starts joins the wire.

What a rig wants in *every* shell goes through `Rig::bash` instead, which
`BASH_ENV` carries where a command line cannot reach.

**Reaching a `Run` means bash got to its own end**, so `subject` is always the
subject's own status — `Signal(9)` where the rig cut the run short, since the
group it owned was killed to do so. `failed` is narrower: what went wrong
*closing up*, after the subject is gone, which is why it travels beside a
status rather than replacing it.

A `Failure` in place of a `Run` means the run never got that far: it could not
be set up, or the rig could not do its work and the subject was killed.

### The run owns its subject

```rust
struct Subject { child: Child, group: libc::pid_t }
```

Spawned with `process_group(0)`, so the subject and everything it starts are
one group. `finish` **kills the group and then reaps** — in that order, because
while the subject is unreaped its group cannot have been recycled, so the
signal cannot reach anything else. `Drop` does the same if `run` left by any
other path; `Child::wait` caches its status, so doing it twice costs nothing.

Reaping is `wait`, not `try_wait`: a killed child that is merely unreaped still
answers `kill(pid, 0)`.

`Subject` is a local of `Master::run`, declared after the serving, so leaving
through `?` drops it first and the shell is stopped before what was feeding it
is released. A process that means to survive detaches with `setsid`, which
leaves the group.

## `Slave` — bash orchestrates

```rust
pub trait Slave: Rig {
    fn serve<A>(&self, held: Held, announce: A) -> Result<Served<Self::Session>, Failure>
    where A: FnOnce(&Answer) -> Result<(), Failure>;
}

pub struct Served<S> {
    pub session: S,
    pub closed: Closed,
    pub failed: Option<Failure>,
}

pub enum Closed { Said, Gone }
pub struct Held(OwnedFd);
```

A script that is already running starts the server, takes the address it is
handed, and says when the session is over. Nothing on this side starts a
process or ends one.

`announce` is called once, before anything is served, with the session's
address: `Answer::of("source", [prelude])`. It is one command, `Display`s as
the bash array literal a reply carries, and a client runs it the way the
prelude runs an answer:

```bash
declare -a __join="$line"; "${__join[@]}"
```

What that reaches is the client's decision. Sourcing instruments that shell,
its functions, its subshells and what it sources; exporting `BASH_ENV` to the
same path as well instruments the processes it starts — see
[scoping.md](scoping.md).

`Held` is the handle the initiator holds open for as long as its session lasts;
it hangs up when the last holder has let go. An initiator that closes properly
never reaches it: it closes with a word of its own, which its rig reads as
`Halt::Done`. `Closed` says which of the two happened, and what a tool makes of
`Gone` is the tool's business.

A client that keeps talking after the session closed writes into a fifo whose
reader is gone and takes `SIGPIPE`. Closing last — from a `trap … EXIT` — is
what a client does about it.

## Both have both exits

| | the handle became ready | the rig said `Halt::Done` |
|---|---|---|
| `Master` | bash is finished — reap, report its status | cut short — the group is killed, status `Signal(9)` |
| `Slave` | the initiator vanished — `Closed::Gone` | the client closed — `Closed::Said` |

Which of the two is the ordinary one is a fact about who started what. The
serving loop knows neither, and neither entry point branches on it: `Master`
kills-then-reaps either way, and `Slave` reports what happened.

## `Serving` — what they share

```rust
struct Serving<'r, R: Rig> {
    rig: &'r R,
    session: R::Session,
    wire: Wire,
    prelude: PathBuf,
    _temporary: Option<TempDir>,
}
```

`lay` writes the workspace and the pipe *before* asking the rig to open a
session, so the session is the last thing acquired and nothing is held over a
setup that failed. `prelude` is the session's only address. `finish` reports a
message left half-read before asking the rig to end, since it is the earlier
fault.

There is no interval and no timer.

```rust
fn step(&mut self, until: &Until) -> Result<Ready, Halt> {
    self.deliver()?;

    match wait_for(&self.wire, until)? {
        Ready::Spoke => Ok(Ready::Spoke),
        Ready::Gone => { self.deliver()?; Ok(Ready::Gone) }
    }
}

fn drive(&mut self, until: &Until) -> Result<Closed, Failure> {
    loop {
        match self.step(until) {
            Ok(Ready::Spoke)       => continue,
            Ok(Ready::Gone)        => return Ok(Closed::Gone),
            Err(Halt::Done)        => return Ok(Closed::Said),
            Err(Halt::Failed(why)) => return Err(why),
        }
    }
}
```

`drive` is the one place where the rig's vocabulary becomes the run's: above it
there is no `Halt`, below it no `Closed`.

`deliver` hands every message the pipe holds to the rig one at a time, and
writes the answer to a shell that asked before moving on. A ready handle does
not imply an empty pipe, so `Gone` is reported only after a last delivery.

```rust
struct Until(OwnedFd);

impl Until {
    fn process(pid: libc::pid_t) -> Result<Self, Failure>;   // pidfd
    fn held(handle: OwnedFd) -> Self;                        // POLLHUP
}
```

**`Until` observes; whoever started the thing owns it.** `Until` never signals
and never reaps, which is what lets one loop serve both orchestrations. It is
private: neither constructor is reachable outside the role method that needs
it, so the two cannot be mixed.

`poll` asks for `POLLIN` on both descriptors, which is what a pidfd reports
when its process exits. A handle reports `POLLHUP` or `POLLERR`, which `poll`
delivers whether or not they were asked for.

## What a session is for

**Tracking what a run produced is entirely the client's.** The library ships no
accumulator, no collection type, and no rig implementation.

| | its `Session` | what it overrides |
|---|---|---|
| bashcap | `Capturing { written, sink }` | `bash`; `hear` decodes and writes; `end` flushes |
| `examples/snapshotting.rs` | `Vec<Capture>` | `bash`; `hear` decodes and keeps |
| `examples/answering.rs` | what it has heard | `bash`; `answer` decides from it |
| `bashprof` | `Vec<Line>` | `bash`; `hear` keeps. Every message carries its own provenance *and* the name of the call it was made inside of, so reading is one pass with a map, then two hylic folds — one to nest, one to read the tree as timings |
| `proofs/answering.rs` | `Soak { heard, answered }` | `bash`, `hear`, `answer` |
| `proofs/serving.rs` | `Vec<Line>` | `bash`; `hear` keeps, and reads one word as the end |
| `proofs/owning.rs` | `()` | `answer`, which never returns |

## When the rig fails

**`Halt::Failed` ends the conversation.** Under `Master` it leaves through `?`,
dropping `Subject` — killing the group and reaping it — and `run` yields that
reason instead of a `Run`.

The subject is not told. An answer is a command, and no command means *the
operator broke*; a shell that carried on would be running against a rig that
has already stopped working.

| | whose |
|---|---|
| a rig cannot hear, or cannot decide an answer | the run's — kill, and `Err` |
| an answer that returns non-zero | the subject's — `set -e`, `\|\|`, or ignore it |

**Every answer is the same kind of thing.** Saying no is a command returning
non-zero, exactly as saying yes is a command returning zero. `Answer` has `of`
and `status`, and the default `answer` says the word is unknown with `return
127` like any rig would.

Serving is therefore stateless: no flag, no poisoned mode, no second reading of
a message already handled.

Under `Slave` a shell that was blocked when the fault happened stays blocked:
the session owns nothing it could kill.

None of this costs the status. `bashcap run` still exits with the subject's
code even when the capture broke, and says on stderr that it did.

## Status

```rust
pub enum ExitStatus { Code(u8), Signal(u8) }

impl ExitStatus {
    /// What a shell would report for it: `128 + n` for a signal.
    pub fn shell_code(self) -> i32;
}
```

Both fields are the width the kernel gives them. The conversion from
`std::process::ExitStatus` reads them out of the raw wait status — `WTERMSIG`
is the low seven bits, `WEXITSTATUS` the second byte — so it is total.

How a run went and how the subject ended are two facts, and the status is only
the second: `Run::failed` carries the first.

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

The first thing that cannot be read or written ends the conversation. A subject
that exits non-zero, is signalled, or asks something the rig has no useful
answer to is an outcome.

## See also

- [wire.md](wire.md) — what `deliver` hands back, and what an answer is
- [tree.md](tree.md) — views over what a session kept
- [stack.md](stack.md) — the frame walk any instrument can reuse
- [vendoring.md](vendoring.md) — the words a client ships, and the hooks behind them
- [bashcap.md](bashcap.md) — a rig that streams instead of keeping

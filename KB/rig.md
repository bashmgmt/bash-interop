# The rig — a reaction per shell, and two orchestrations

`src/bash/rig/mod.rs` for the two traits, `driving.rs` and `serving.rs` for the
two ways a session comes about, `session.rs`, `attend.rs` and `watch.rs` for
what they share.

```
rig/  mod.rs       the doc; `Rig`, `Reacting`, the two templates; the re-export list; `JOINING`
      joining.txt  how a script joins, in every way there is — `JOINING`'s text
      attended.rs  `Layout`, `Attended`, `Kept`, `Said`, `heard`
      session.rs   `Session` — open, serve, announced, close
      attend.rs    `attend` — one shell's task, start to finish
      watch.rs     `Watch` — what a session ends on
      driving.rs   `Driving`, `Run`, `Whole`, `Subject`, `ExitStatus`
      serving.rs   `Serving`, `Served`
      wire/        the protocol
```

A **rig** is a description: the bash it gives the subject, where the session's
files go, and how to build a reaction once a shell is there. The reaction is
`Reacting`, and it is made **per shell**, at the moment that shell announces
itself; it then runs as a task of its own. Who started what — and therefore who
ends it and cleans up — is a second question, and each of its two answers is a
trait that extends `Rig` and carries its own orchestration.

## `Rig` and `Reacting`

```rust
pub trait Rig {
    type Reaction: Reacting;

    /// The rig's own bash. The invocation sources it with the session's
    /// workspace as `$1`; joining — which labels, with which words, or none —
    /// is this text's own business: `BC_JOIN <LABEL> "$1" [word…]`.
    fn bash(&self) -> String;

    /// A shell has joined, and everything about it is known. Awaited in the
    /// accept loop, so a slow `joined` delays the next join and nothing else.
    async fn joined(&self, at: &Layout, shell: Arc<Shell>) -> Result<Self::Reaction, Failure>;
}

pub trait Reacting: Sized + 'static {
    type Kept: 'static;

    async fn hear(&mut self, said: Message) -> Result<(), Failure>;
    async fn answer(&mut self, asked: Message) -> Result<Answer, Failure>;
    async fn finish(self) -> Result<Self::Kept, Failure>;
}
```

**No method has a default body.** An `impl` block is the whole contract, and a
reaction that drops what it hears or refuses every question says so where a
reader will look for it. Two shipped reactions are the templates to copy from,
and a rig that wants either whole names it as its `Reaction` and writes
nothing:

| | `hear` | `answer` | `finish` |
|---|---|---|---|
| `Vec<Message>` | push | `hear`, then `Answer::unknown()` | `Ok(self)` |
| `()` | drop it | `Answer::unknown()` | `Ok(())` |

`&self` on `Rig` throughout — a rig is a description and is never mutated by
running. `&mut self` on `Reacting` — a reaction is the thing that changes.
`Message` arrives **by value**.

### The session is single-threaded, and every shell is a task

One `current_thread` tokio runtime, a `LocalSet` inside `run`/`serve`, one
`spawn_local` per shell, and no `Send` bound anywhere. `'static` on `Reacting`
is what a task needs: a reaction owns what it holds. Awaiting inside `hear`,
`answer` or `finish` yields to the other shells' tasks; synchronous work in one
blocks them for its duration, as it always did.

What shells share is the caller's own, handed in through `Rig::joined`:

```rust
type Sink = Rc<RefCell<BufWriter<File>>>;

struct BashCap { into: PathBuf, sink: Sink, tracing: Tracing }
struct Capturing { shell: Arc<Shell>, into: PathBuf, sink: Sink, written: usize }
```

An `Rc<RefCell<_>>` is a share on one thread. The one rule that comes with
async: a `RefCell` borrow is not held across an `.await` — the borrow is fine,
another task borrowing while this one is parked is the panic. Every reaction in
the crate borrows, writes, and returns without awaiting.

`Rc<Shell>` would be honest too; `Arc<Shell>` stays because a run's *result* is
data and may travel.

### Facts are members, not parameters

Which bash a shell is, how it was started and what it had switched on are
settled before its first message and cannot change while it lives. They arrive
once, at `joined`, and a reaction that needs them keeps them:

```rust
struct Seen { shell: Arc<Shell>, captures: Vec<Capture> }

async fn joined(&self, _at: &Layout, shell: Arc<Shell>) -> Result<Seen, Failure> {
    Ok(Seen { shell, captures: Vec::new() })
}
```

Owning a reaction is the whole proof that its shell announced itself. A
message reaches a reaction only down that shell's own pipe, so no path through
the session can hold a message whose shell is unknown.

`Layout { dir, address }` is where the session's files ended up — the
workspace, and the address: `<dir>/session.bash`, the generated invocation a
shell sources to join, held as text because it crosses into bash and onto the
announce line (validated whole at open). `Layout::bash_env()` is that address
spelled as the `("BASH_ENV", <address>)` pair.

**Nothing in a rig ends a session.** A rig reacts; when the conversation is
over is decided by whoever started the shells. A `Failure` is the only thing a
rig can raise, and it means the rig could not do its work.

## What a run hands back

```rust
pub struct Attended<K> {
    pub shell: Arc<Shell>,
    pub kept: K,
    /// When nobody could write on its pipe any more. `None` for a shell the
    /// session outlived.
    pub parted: Option<Micros>,
}

pub type Kept<R> = <<R as Rig>::Reaction as Reacting>::Kept;
```

One entry per shell, in the order they joined, each carrying what its own
reaction left behind. The provenance is the shape rather than a field.

Where a caller wants the run flat again:

```rust
pub struct Said<'a> { pub shell: &'a Arc<Shell>, pub message: &'a Message }

pub fn heard<K: AsRef<[Message]>>(shells: &[Attended<K>]) -> Vec<Said<'_>>;
```

Many pipes have no arrival order between them, so `heard` sorts on
`Stamp::sent_at` — the sending shell's own clock — stably over join order and
each shell's own order. A `Kept` of the caller's own joins in by implementing
`AsRef<[Message]>`. The core has no accumulator beyond this: what needs to be
whole across shells is a resource the rig owns and hands shares of.

## `Driving` — Rust orchestrates

```rust
pub trait Driving: Rig {
    /// A workspace of the run's own, gone when the run ends.
    async fn run<A, E>(&self, argv: &[A], environment: E) -> Result<Run<Kept<Self>>, Failure>
    where A: AsRef<OsStr>, E: FnOnce(&Layout) -> Vec<(OsString, OsString)>;

    /// The caller's directory instead — created if missing, left behind: a
    /// reading taken later may follow source paths into it.
    async fn run_at<A, E>(&self, at: &Path, argv: &[A], environment: E) -> Result<Run<Kept<Self>>, Failure>
    where A: AsRef<OsStr>, E: FnOnce(&Layout) -> Vec<(OsString, OsString)>;
}
// the role opt-in is an empty impl block; the closure's return is the
// subject's WHOLE environment delta — the core exports nothing

pub struct Run<K> { pub shells: Vec<Attended<K>>, pub subject: ExitStatus, pub failed: Option<Failure> }
pub struct Whole<K> { pub shells: Vec<Attended<K>>, pub subject: ExitStatus }   // Run::whole()

```

**The command line is run as it is given, and carries its own program.**
`rig.run(&["bash", "x.bash"], |at| …)`, and a caller wanting a launcher
writes one into the argv: `&["env", "TARGET=staging", "bash", "x.bash"]`. The
subject's environment is exactly the closure's return, built from the settled
`Layout`: `vec![at.bc_session(), at.bash_env()]` is the usual pair — the
handle a script sources, and the join of every non-interactive bash in the
tree; `vec![at.bc_session()]` leaves joining to the scripts; `vec![]` and any
variable of the caller's own are equally legitimate sentences.
`tests/proofs/starting.rs` proves the strong side: a variable the closure did
not return is absent.

**Reaching a `Run` means bash got to its own end**, so `subject` is always the
subject's own status. `failed` is what went wrong closing up. A `Failure` in
place of a `Run` means the run never got that far.

The run spawns the subject with `process_group(0)`, watches its pidfd, and when
it fires **kills the group and then reaps** — in that order, because while the
subject is unreaped its group cannot have been recycled. Only then does the
session close, so every task reads what its shell wrote up to the kill and
sees end of input. `Drop` does the same if `run` left by any other path.

## `Serving` — bash orchestrates

```rust
pub trait Serving: Rig {
    async fn serve<A>(&self, at: &Path, held: OwnedFd, announce: A)
        -> Result<Served<Kept<Self>>, Failure>
    where A: FnOnce(&str) -> Result<(), Failure>;

    async fn serve_coprocess(&self, at: &Path) -> Result<Served<Kept<Self>>, Failure>;
}

pub struct Served<K> { pub shells: Vec<Attended<K>>, pub failed: Option<Failure> }
```

A script that is already running names the workspace, starts the server, and
lets go when it is done. Nothing on this side starts a process or ends one.
`at` is required with no fallback: the client prescribed the directory, so it
knows the address — `<at>/session.bash` — before the server has done anything.
The directory is created if missing and left behind; a reading taken later may
follow source paths into it, and removing it is the client's, like everything
else it initiated.

What remains of the announcement is the rendezvous: `coproc` returns before
the server has parsed its argv or written a file, so the client must not
source before the session is laid. `announce` is called once, after
`Session::open` returned and before anything is served, with the address as
one line — a value the client's own choice already fixed. The blocking read on
the client's side is the barrier, and end of input is how a server that died
before ready (unwritable `--at`, bad flag) is told apart from a slow one. The
client puts the line where a driven shell would find it and sources it:

```bash
export BC_SESSION="$line"; source "$BC_SESSION"
```

A `Failure` while serving still sees the session out — every shell released
or finished, the fifos gone — before it is returned.

`held` is a descriptor the initiator keeps open for as long as it wants the
session. Serving ends when the last holder has let go, deliberately or by
dying. A descendant that inherited the handle keeps the session open, since it
can still speak. A shell that keeps talking after the session closed writes
into a fifo whose reader is gone and takes `SIGPIPE`.

### The coprocess convention

**The client holds the server's standard input, and the server writes the
address on its standard output.**

```bash
source lib/joining.bash

BC_START bashprof serve --at prof.d --into build.times   # start it, sync, join
BC_INSTR BASHPROF say STEP compile
BC_LEAVE                                     # release, wait, return its status
```

`assets/joining.bash` is the bash half; `Serving::serve_coprocess` is the Rust
half. `coproc` takes a literal NAME, so the server's fds live in `BC_SERVER`
and there is one server per shell; `BC_START` reads the address into
`BC_SESSION`, exports it, and sources it. `BC_LEAVE` returns the server's
status, so a client under `set -e` stops on a server that failed, and by the
time it returns whatever the server writes is written.

`JOINING` (`rig/joining.txt`) is the whole list — driven and already joined,
by hand, started as a coprocess, only if there is a session, the vendored words
and their polyfill — and both binaries print it under `run --help` and
`serve --help`.

`__fixtures/joined/build.bash` starts the shipped `bashprof serve` and is
driven from `tests/cli.rs`; `merging.bash` starts `tests/joined/merging.rs`, a
program rather than a harness because a script that starts its own server has
to have something to start.

## One exit

| | the descriptor | the session's extent | who cleans up |
|---|---|---|---|
| `Driving` | a pidfd on the subject the run started | its process group | the run: kill, reap, report the status |
| `Serving` | the handle the initiator holds | whoever inherited it | nobody — not our process |

## `Session` — what they share

```rust
struct Session<'r, R: Rig> {
    rig: &'r R,
    layout: Layout,
    control: Control,                                    // <dir>/join, held read-write
    attending: JoinSet<Result<Attendance<Kept<R>>, Failure>>,
    closing: watch::Sender<bool>,
    joined: usize,                                       // the next shell's nth
    done: Vec<Attended<Kept<R>>>,
    _temporary: Option<TempDir>,
}
```

One future borrows the rig; every shell is a task that borrows nothing.

```rust
async fn serve(&mut self, watch: &Watch) -> Result<(), Failure> {
    loop {
        tokio::select! {
            biased;
            announced = self.control.next()         => self.announced(announced?).await?,
            Some(done) = self.attending.join_next() => …,   // a task's Failure ends the run here
            fired = watch.fired()                   => return fired,
        }
    }
}
```

`Control::next` yields `Announced { token, account }`: the shell's account
came with its announcement, reassembled from frames on the control fifo (see
[wire.md](wire.md#the-control-fifo)). `announced` makes the shell's reply
pipe, opens its pipe — which is what releases the shell from its blocking open
— builds the `Shell`, awaits `rig.joined`, and spawns the task. Nothing in the
accept loop awaits a shell.

`attend` is the task: `select!` on the next line and on `closing`; a `SAY`
goes to `hear`, an `ASK` to `answer` and the answer down the reply pipe; end
of input is the shell parting; `closing` means the watch fired, so what the
pipe already holds is read and the reaction finished. A line the shell left
half-written is reported beside what it kept, not instead of it.

`close` releases every announced pipe not yet opened, unlinks the control
fifo, signals `closing`, collects every task in join order, and hands back
`(Vec<Attended<Kept>>, Option<Failure>)`.

`Watch` is an `AsyncFd` over a pidfd or the held handle. It observes; whoever
started the thing owns it.

## When the rig fails

**A `Failure` from `hear`, `answer` or `joined` ends the conversation.** It
reaches the serve loop on the task's next turn, and under `Driving` the
`Subject` — which lives inside the orchestration's inner block — is dropped
first, killing the group. The subject is not told: an answer is a command, and
no command means *the operator broke*.

| | whose |
|---|---|
| a rig cannot hear, or cannot decide an answer | the run's — kill, and `Err` |
| an answer that returns non-zero | the subject's — `set -e`, `\|\|`, or ignore it |
| a line the protocol did not write, or one left half-written | the run's while it is served; `failed` if found at close |

**In both roles, every path after `Session::open` sees the session out**:
`run`, `run_at` and `serve` have one exit — whatever failed (a reaction, the
watch, the announcement, the spawn) is held while `close` runs, then returned.
Close releases a shell announced and not yet opened, removes the fifo of an
announcement that never finished (its token names it), unlinks the control
fifo, and collects every task; a shell still attached takes `SIGPIPE` at its
next word. Under `Serving` the session owns nothing it could kill.

## Status

```rust
pub enum ExitStatus { Code(u8), Signal(u8) }   // shell_code(): 128 + n for a signal
```

How a run went and how the subject ended are two facts, and the status is only
the second: `Run::failed` carries the first. No signal disposition is changed.

## One error, one way out

```rust
pub struct Failure { doing: String, cause: Box<dyn Error + Send + Sync> }

pub trait Doing<T> {
    fn doing(self, what: impl FnOnce() -> String) -> Result<T, Failure>;
}
```

`src/failure.rs`, crate-level. A context and a cause rather than an enum, since
every use is `Display` or `source()`.

## See also

- [wire.md](wire.md) — the fifos, the lines, and what an answer is
- [shell.md](shell.md) — what a shell is
- [stack.md](stack.md) — the frame walk any instrument can reuse
- [vendoring.md](vendoring.md) — the words a client ships, and the hooks behind them
- [bashcap.md](bashcap.md) — a rig that streams instead of keeping

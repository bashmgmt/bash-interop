# The rig — a reaction per shell, and two orchestrations

`src/rig/mod.rs` for the two traits, `driving.rs` and `serving.rs` for the
two ways a session comes about, `session.rs`, `attend.rs` and `watch.rs` for
what they share.

```
rig/  mod.rs       the doc; `Rig`, `Reacting`, the two templates; the re-export list; `JOINING`
      joining.txt  how a script joins, in every way there is — `JOINING`'s text
      attended.rs  `Layout`, `Provision`, `Attended`, `Kept`, `Said`, `heard`
      session.rs   `Session` — open, serve, announced, close
      attend.rs    `attend` — one shell's task, start to finish
      watch.rs     `Watch` — what a session ends on
      driving.rs   `Driving`, `Run`, `Whole`, `Subject`, `ExitStatus`
      serving.rs   `Serving`, `Served`
      wire/        the protocol
```

A **rig** is a description: the definitions it gives the subject, its
standard initiation as data, and how to build a reaction once a shell is
there. The reaction is
`Reacting`, and it is made **per shell**, at the moment that shell announces
itself; it then runs as a task of its own. Who started what — and therefore who
ends it and cleans up — is a second question, and each of its two answers is a
trait that extends `Rig` and carries its own orchestration.

## `Rig` and `Reacting`

<!-- quote: src/rig/mod.rs anchor=rig-trait -->
```rust
#[expect(async_fn_in_trait, reason = "single-threaded by design: no Send bound")]
pub trait Rig {
    /// What reacts to one shell.
    type Reaction: Reacting;

    /// The rig's own bash: **definitions only**. Its words, and at most a
    /// channel-init function; sourcing it has no effect on a shell beyond
    /// names coming into being, so it is inert, re-sourceable, and free of
    /// the coordinate unless its author bakes one in.
    fn bash(&self, at: &Layout) -> String;

    /// The rig's standard initiation, as a line of bash ending in a newline
    /// — `BASHPROF_INIT '<dir>'`, or a raw `BC_JOIN <LABEL> <dir> [word…]`.
    /// Data: the core never runs it. [`Layout::bash_env`] writes it into the
    /// provisioned file under [`Provision::Joining`]; every other shell's
    /// initiation is its own code.
    fn joining(&self, at: &Layout) -> String;

    /// A shell has joined, and everything about it is known. Awaited in the
    /// accept loop, so a slow `joined` delays the next join and nothing else.
    async fn joined(&self, at: &Layout, shell: Arc<Shell>) -> Result<Self::Reaction, Failure>;
}
```

<!-- quote: src/rig/mod.rs anchor=reacting-trait -->
```rust
#[expect(async_fn_in_trait, reason = "single-threaded by design: no Send bound")]
pub trait Reacting: Sized + 'static {
    /// What is left when the shell can no longer speak. `Self` where nothing
    /// is released at the end.
    type Kept: 'static;

    /// A `Failure` from this or [`answer`](Reacting::answer) ends the
    /// conversation: under [`Driving`] the subject is killed and the run
    /// yields that reason.
    async fn hear(&mut self, said: Message) -> Result<(), Failure>;

    /// An answer is a command, and every answer is the same kind of thing.
    /// Saying no is a command that returns non-zero — [`Answer::unknown`] for
    /// a word this rig has no answer for.
    async fn answer(&mut self, asked: Message) -> Result<Answer, Failure>;

    /// The conversation is over; release what this held.
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
blocks them for its duration.

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

`Layout` is the workspace: the one coordinate — held as text, validated at
construction, because it crosses into bash — and the model of the files in
it. The constant names (`bash_env.bash`, `prelude.bash`, `rig.bash`, `join`,
`up.<tok>`, `rep.<tok>`, `lock`) live nowhere else; `Layout::text()` is what
a rig splices into its joining through `emit_scalar`, and
`Layout::bash_env(provision)` is the one owner of the provisioned startup
file. The session holds `<dir>/lock` `flock`ed from before it touches the
directory until after its fifos are gone: an occupied workspace is refused,
a killed predecessor's fifos are swept at the next open, and the kernel
releases the hold on any death.

What the provisioned file does about the channel is its writer's stated
choice:

<!-- quote: src/rig/attended.rs anchor=provision -->
```rust
/// What the provisioned file does about the channel — the first thing a
/// [`Layout::bash_env`] caller states.
#[derive(Copy, Clone, Debug)]
pub enum Provision<'a> {
    /// The file ends with this line, [`super::Rig::joining`]'s
    /// usually: subjects with no prior knowledge join as their shells start.
    Joining(&'a str),

    /// Definitions only: the client code initiates its own channel, and the
    /// file carries no coordinate — the caller states one beside this pair
    /// if its scripts need it.
    Definitions,
}
```


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
    _lock: Lock,                                         // released last; the kernel's on any death
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
- `bashcap/docs/` — a rig that streams instead of keeping

# The rig

This chapter is the API: the two traits you write, `Rig` and `Reacting`, the
values you are handed, `Layout` and `Shell`, and what a finished run gives
back. It ends with the session machinery underneath, so the guarantees above
it can be checked rather than assumed.

Where the code lives, for reading along:

```
src/rig/
      mod.rs       `Rig`, `Reacting`, the two shipped reactions
      attended.rs  `Layout`, `Provision`, `Attended`, `Kept`, `Said`, `heard`
      driving.rs   `Driving`, `Run`, `Whole`, `ExitStatus`
      serving.rs   `Serving`, `Served`
      session.rs   `Session` — open, serve, announced, close
      attend.rs    `attend` — one shell's task, start to finish
      watch.rs     `Watch` — what a session ends on
      wire/        the protocol (its own chapter: wire.md)
```

## A rig at a glance

A rig that gives the subject one word, keeps what each shell says, and answers
one question. This is a compressed sketch; the compiled original, kept honest
by the doctest gate, is the module doc of `rig`.

```rust
struct Deploying;                                        // the description
struct Told { shell: Arc<Shell>, heard: Vec<Message> }   // one shell's reaction

impl Rig for Deploying {
    type Reaction = Told;

    // definitions only: a word scripts can call; nothing joins here
    fn bash(&self, _at: &Layout) -> String {
        "STAGE() { BC_INSTR DEPLOY say STAGE \"$@\"; }\n".to_string()
    }

    // a shell joined: build its reaction from what it said of itself
    async fn joined(&self, _at: &Layout, shell: Arc<Shell>) -> Result<Told, Failure> {
        Ok(Told { shell, heard: Vec::new() })
    }
}

impl Reacting for Told {
    type Kept = Self;
    async fn hear(&mut self, said: Message) -> Result<(), Failure> {
        self.heard.push(said);
        Ok(())
    }
    async fn answer(&mut self, asked: Message) -> Result<Answer, Failure> {
        Ok(match asked.words.first().map(String::as_str) {
            Some("target") => Answer::of("declare", ["-g", "target=staging"]),
            _ => Answer::unknown(),
        })
    }
    async fn finish(self) -> Result<Self, Failure> { Ok(self) }
}

impl Driving for Deploying {}     // opt in to the Rust-orchestrated role

// the standard initiation, as data — the run's closure hands it to
// bash_env; run only where a client or a provisioned file says so
fn deploy_join(at: &Layout) -> String {
    format!("BC_JOIN DEPLOY {}\n", bash_strings::emit_scalar(at.text()))
}
```

Three shapes make up the arrangement. The rig is a single value describing
all of it. The reaction is a second type, built fresh for each shell. The
roles — `Driving` here — are empty impls you opt into, and the trait brings
the orchestration with it.

## `Rig`

As it stands in `src/rig/mod.rs`; the doc comments are the contract.

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

    /// A shell has joined, and everything about it is known. Awaited in the
    /// accept loop, so a slow `joined` delays the next join and nothing else.
    async fn joined(&self, at: &Layout, shell: Arc<Shell>) -> Result<Self::Reaction, Failure>;
}
```

`bash()` takes `&Layout` because baking the coordinate in is a freedom. Most
rigs ignore the parameter and return the same bytes for every session; a rig
that wants the workspace inside a definition can have it. The text becomes
`<dir>/rig.bash`, laid by the session, and sourcing that file is safe because
this method promises definitions only.

The initiation line lives with the wrapper, the code that owns the run and its
environment closure. A provisioned `bash_env.bash` is the one place allowed to
automate initiation, and it takes the line as plain data,
`Provision::Joining(&line)`. The tools each export theirs as a function beside
their rig, such as `bashprof::joining(at)`, and a by-hand script types the
same line. The core takes a string and has no method for it, so the wrapper
states which line initiates a rig, at the point where the run is made.

`joined()` is async because it runs in the session's accept loop, between a
shell announcing itself and its pipe opening, where you may need to open a
file, allocate a resource, or consult something. A slow `joined` delays the
next join and never a shell already admitted.

`&self` throughout, because a rig is a description and running it changes
nothing about it. Everything that changes lives in the reactions.

## `Reacting`

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

`hear` receives a `say`, where nobody is waiting, so there is nothing to
produce but success or a `Failure`. `answer` receives an `ask`, where a shell
is blocked, and the `Answer` you return is written back as the command that
shell runs. `finish` consumes the reaction when its shell can no longer speak,
and what it returns becomes the shell's entry in the run's result.

`&mut self` because the reaction is what changes. `Message` arrives by value
because it is yours from then on. `'static` because each reaction runs as a
task of its own and must own what it holds.

No method has a default body, so an implementor decides every case in view.
The two common whole behaviours ship as types instead: name one as your
`Reaction` and write nothing.

| shipped reaction | `hear` | `answer` | `finish` |
|---|---|---|---|
| `Vec<Message>` | push | `hear` it, then `Answer::unknown()` | `Ok(self)` |
| `()` | drop it | `Answer::unknown()` | `Ok(())` |

`Answer::unknown()` is `return 127`, bash's own command-not-found, so a script
asking a question no rig answers sees an ordinary, testable failure status.

### Sharing between shells

Several shells writing into one place — an output file, a merged view — share
a resource the rig owns, and `joined` hands each reaction a share. From
bashcap:

```rust
type Sink = Rc<RefCell<BufWriter<File>>>;

struct BashCap   { into: PathBuf, sink: Sink, tracing: Tracing }   // the rig owns it
struct Capturing { shell: Arc<Shell>, into: PathBuf, sink: Sink, written: usize }
```

`Rc<RefCell<_>>` fits because the session is single-threaded: one
`current_thread` runtime, one `spawn_local` task per shell, no `Send` bound
anywhere. The rule async adds is to never hold the `RefCell` borrow across an
`.await`; the borrow itself is fine, and the panic case is another task
borrowing while yours is parked. Every reaction in this crate borrows, writes
and returns without awaiting.

Awaiting inside `hear`, `answer` or `finish` yields to the other shells'
tasks, and synchronous work blocks them for its duration, on the usual terms
of a cooperative loop.

### Facts are members, not parameters

Which bash a shell is, how it was started, what options it had on, and which
words its join brought are settled before its first message and cannot change
while it lives. A subshell that differs is a new shell with its own
`$BASHPID`, and `set` refuses `-i`, `-c` and `-s`. So it arrives once, at
`joined`, as `Arc<Shell>`, and a reaction that needs it keeps it as a member:

```rust
struct Seen { shell: Arc<Shell>, captures: Vec<Capture> }

async fn joined(&self, _at: &Layout, shell: Arc<Shell>) -> Result<Seen, Failure> {
    Ok(Seen { shell, captures: Vec::new() })
}
```

Owning a reaction is the evidence that its shell announced itself, and a
message reaches a reaction only down that shell's own pipe, so no path through
the session holds a message whose shell is unknown.

## `Layout` and `Provision`

`joined`, `bash` and the environment closure all receive `&Layout`. It is the
workspace: one validated coordinate, held as text because it crosses into
bash, plus the model of the files inside. The constant names — `prelude.bash`,
`rig.bash`, `bash_env.bash`, `join`, `up.<tok>`, `rep.<tok>`, `lock` — exist
nowhere else in the codebase.

What you call on it:

- `at.text()` — the directory as text, ready for `bash_strings::emit_scalar`
  when a joining line spells it;
- `at.path()` — the same as a `&Path`, for Rust's own file work;
- `at.bash_env(provision)` — the one owner of the provisioned startup file: it
  writes the file and yields the `("BASH_ENV", <file>)` pair for the
  environment closure to return.

The last one takes the choice its caller states first:

<!-- quote: src/rig/attended.rs anchor=provision -->
```rust
/// What the provisioned file does about the channel — the first thing a
/// [`Layout::bash_env`] caller states.
#[derive(Copy, Clone, Debug)]
pub enum Provision<'a> {
    /// The file ends with this line — supplied by the provisioner, usually
    /// the rig's standard initiation: subjects with no prior knowledge join
    /// as their shells start.
    Joining(&'a str),

    /// Definitions only: the client code initiates its own channel, and the
    /// file carries no coordinate — the caller states one beside this pair
    /// if its scripts need it.
    Definitions,
}
```

The two arms carry different information, which is why this is an enum.
`Joining` needs the line to write, the wrapper's own. `Definitions` leaves the
file without a coordinate, so a caller whose scripts must find the workspace
states a variable for it beside this pair; the tools spell theirs
`BASHPROF_SESSION` and `BASHCAP_SESSION`. [joining.md](joining.md) shows both
arms as whole scripts, including what happens to a shell that has the words
and never initiates.

Behind the same type sits the ownership story, told once here and assumed
elsewhere. The session `flock`s `<dir>/lock` before touching anything and
releases it after its fifos are gone. An occupied workspace is refused whole.
A predecessor killed outright leaves stale fifos that the next open sweeps
safely, the kernel having released the dead lock. The `join` fifo therefore
exists exactly while a session serves, which is what makes `[[ -p <dir>/join
]]` a truthful liveness probe.

## What a run hands back

One entry per shell, in join order:

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

Which shell produced something is which entry you are holding, so there is no
field to match up. `parted` is an `Option` because it records a genuine
either/or: `Some(when)` for a shell that finished while the session watched,
`None` for a shell still alive when the session ended, such as a served client
outliving the handle.

When a reading wants the run flat again — every message from every shell, in
the order things were said:

```rust
pub struct Said<'a> { pub shell: &'a Arc<Shell>, pub message: &'a Message }

pub fn heard<K: AsRef<[Message]>>(shells: &[Attended<K>]) -> Vec<Said<'_>>;
```

Separate pipes have no arrival order between them, so `heard` sorts by
`Stamp::sent_at`, the sending shells' own clocks, stably over join order. Your
own `Kept` joins in by implementing `AsRef<[Message]>`. The core ships no
session-wide collector, since what needs to be whole across shells is a
resource the rig owns, in the `Sink` pattern above.

## Under the floor: the session

Nothing below is API. Seeing the loop once makes the guarantees above
concrete.

Both orchestrations drive the same `Session`, sketched from
`src/rig/session.rs`; rustdoc is authoritative.

```rust
struct Session<'r, R: Rig> {
    rig: &'r R,
    layout: Layout,
    control: Control,          // the join fifo, held open read-write
    attending: JoinSet<…>,     // one task per admitted shell
    closing: watch::Sender<bool>,
    joined: usize,             // the next shell's `nth`
    done: Vec<Attended<…>>,
    _lock: Lock,               // released last; the kernel's on any death
    _temporary: Option<TempDir>,
}
```

The serve loop is three arms:

```rust
loop {
    tokio::select! {
        biased;
        announced = self.control.next()         => self.announced(announced?).await?,
        Some(done) = self.attending.join_next() => …,   // a task's Failure ends the run here
        fired = watch.fired()                   => return fired,
    }
}
```

In the first arm a shell announced itself on the `join` fifo. Its account —
which bash, how invoked, what options, the join's extra words — arrived with
the announcement, reassembled from frames, which [wire.md](wire.md) explains.
`announced` makes the shell's reply fifo, opens its pipe, builds `Shell`,
awaits your `joined`, and spawns the task. Opening the pipe is what releases
the shell from its blocking rendezvous. Nothing in this loop awaits an
admitted shell.

In the second arm some shell's task finished. Its `Attended` is collected, and
a `Failure` it carried ends the run here.

In the third the watch fired, because the subject exited under driving or the
handle was released under serving. The loop returns, and `close` does the one
ending there is: signal every task to drain what its pipe still holds, finish
every reaction, release any shell announced but not admitted, remove the
fifos, unlink `join` last, and hand back `(Vec<Attended<…>>,
Option<Failure>)`.

## When a rig fails

A `Failure` from `joined`, `hear` or `answer` reports that the operator broke,
not the subject.

| happening | whose problem, and what follows |
|---|---|
| a rig cannot hear, or cannot decide an answer | the run's: under `Driving` the subject's process group is killed, and the run returns that `Failure` |
| an answer that returns non-zero | the subject's, entirely ordinary: `set -e`, `\|\|`, or ignoring it are its own choices |
| a line on a pipe the protocol did not write, or one left half-written | the run's while serving; reported in `failed` if found at close |

The subject is not told when the operator breaks. An answer is a command, and
no command arriving means the asking shell blocks until the kill or the close
reaches it. In both roles every path after `Session::open` sees the session
out: whatever failed is held while `close` runs, then returned.

Two small types complete the error picture:

```rust
pub enum ExitStatus { Code(u8), Signal(u8) }   // shell_code(): 128 + n for a signal
```

How the run went and how the subject ended are different facts. `Run::failed`
carries the first and `ExitStatus` the second, and no signal disposition is
changed. Then the crate's one error:

```rust
pub struct Failure { doing: String, cause: Box<dyn Error + Send + Sync> }

pub trait Doing<T> {
    fn doing(self, what: impl FnOnce() -> String) -> Result<T, Failure>;
}
```

A context and a cause rather than an enum, because every consumer either
displays it or walks `source()`.

## See also

- [driving.md](driving.md) / [serving.md](serving.md) — the two roles' own
  chapters
- [wire.md](wire.md) — the fifos, the lines, and what an answer is on the
  wire
- [shell.md](shell.md) — everything `Shell` knows and how it knows it
- [stack.md](stack.md) — the frame walk any instrument can reuse
- [bashcap's book](https://bashmgmt.github.io/bashcap/) — a real rig that streams instead of keeping

# The rig — what you implement

This chapter is the API: the two traits you write (`Rig` and `Reacting`),
the values you are handed (`Layout`, `Shell`), and what a finished run gives
back. It ends with a look under the floor — the session machinery all of it
runs on — so that nothing later in the book has to be taken on faith.

Where the code lives, for when you want to read along:

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

## A rig in one look

Before the contracts, the whole thing at a glance — a rig that gives the
subject one word, keeps what each shell says, and answers one question.
(This is a compressed sketch; the compiled original, kept honest by the
doctest gate, is the module doc of `rig` — `cargo doc --open`.)

```rust
struct Deploying;                                        // the description
struct Told { shell: Arc<Shell>, heard: Vec<Message> }   // one shell's reaction

impl Rig for Deploying {
    type Reaction = Told;

    // definitions only: a word scripts can call; nothing joins here
    fn bash(&self, _at: &Layout) -> String {
        "STAGE() { BC_INSTR DEPLOY say STAGE \"$@\"; }\n".to_string()
    }

    // the standard initiation, as data — run only where someone says so
    fn joining(&self, at: &Layout) -> String {
        format!("BC_JOIN DEPLOY {}\n", bash_strings::emit_scalar(at.text()))
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
```

Three shapes to hold onto: the **rig** is one value describing the whole
arrangement; the **reaction** is a second type, built fresh *per shell*;
and the roles (`Driving` here) are empty opt-in impls — the orchestration
comes with the trait. Now the contracts, one at a time.

## `Rig` — the description

As it stands in `src/rig/mod.rs`; read the doc comments as the contract:

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
    /// — the rig's own `<WORD>_INIT '<dir>'`, or a raw
    /// `BC_JOIN <LABEL> <dir> [word…]`.
    /// Data: the core never runs it. [`Layout::bash_env`] writes it into the
    /// provisioned file under [`Provision::Joining`]; every other shell's
    /// initiation is its own code.
    fn joining(&self, at: &Layout) -> String;

    /// A shell has joined, and everything about it is known. Awaited in the
    /// accept loop, so a slow `joined` delays the next join and nothing else.
    async fn joined(&self, at: &Layout, shell: Arc<Shell>) -> Result<Self::Reaction, Failure>;
}
```

Questions this quote tends to raise, answered in order:

**Why does `bash()` take `&Layout` if its text is supposed to be
location-free?** Because *may bake* is a freedom, not a requirement. Most
rigs ignore the parameter (`_at`) and return the same bytes for every
session; a rig that wants the workspace inside a definition can have it.
The text becomes `<dir>/rig.bash`, laid by the session — and sourcing that
file is safe precisely because this method promises definitions only.

**What is `joining()` for, if the core never runs it?** It is the rig
stating its standard way in *as a value*, so that the one place allowed to
automate initiation — a provisioned `bash_env.bash` — has a line to write.
A by-hand script does not call this method; it types the same line itself
(or the init function the rig's `bash()` defined). One spelling, stated by
the rig, used by whoever chooses to.

**Why is `joined()` async, and what does a slow one cost?** It runs in the
session's accept loop, between "a shell announced" and "its pipe opens".
You may need to open a file, allocate a resource, or consult something —
that can await. The cost is the one the doc comment states: a slow
`joined` delays the *next* join, never a shell already admitted.

**Why `&self` everywhere?** A rig is a description; running it changes
nothing about it. Everything that changes lives in the reactions.

## `Reacting` — one shell's counterpart

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

Reading it slowly: `hear` receives a `say` — nobody is waiting, so there is
nothing to produce but success or a `Failure`. `answer` receives an `ask` —
a shell is blocked on it, and the `Answer` you return is written back as
the command that shell runs. `finish` consumes the reaction when its shell
can no longer speak, and what it returns becomes the shell's entry in the
run's result. `&mut self` because the reaction is the thing that changes;
`Message` arrives by value because it is now yours; `'static` because each
reaction runs as a task of its own and must own what it holds.

**No method has a default body**, deliberately. A default is a decision an
implementor did not make and cannot see in their own code. Instead, the two
common whole behaviours are shipped as types — name one as your `Reaction`
and write nothing:

| shipped reaction | `hear` | `answer` | `finish` |
|---|---|---|---|
| `Vec<Message>` | push | `hear` it, then `Answer::unknown()` | `Ok(self)` |
| `()` | drop it | `Answer::unknown()` | `Ok(())` |

`Answer::unknown()` is `return 127` — bash's own "command not found" — so a
script asking a question no rig answers sees an ordinary, testable failure
status, not a hang and not a crash.

### Sharing between shells

Each reaction is its own task, so how do several shells write into one
place — one output file, one merged view? That place is *the rig's*, and
`joined` hands each reaction a share. From bashcap:

```rust
type Sink = Rc<RefCell<BufWriter<File>>>;

struct BashCap   { into: PathBuf, sink: Sink, tracing: Tracing }   // the rig owns it
struct Capturing { shell: Arc<Shell>, into: PathBuf, sink: Sink, written: usize }
```

`Rc<RefCell<_>>`, not `Arc<Mutex<_>>`: the session is single-threaded — one
`current_thread` runtime, one `spawn_local` task per shell, no `Send` bound
anywhere — so a single-threaded share is the honest one. The one rule async
adds: never hold the `RefCell` borrow across an `.await`. The borrow itself
is fine; the panic case is another task borrowing while yours is parked.
Every reaction in this crate borrows, writes, and returns without awaiting.

Awaiting inside `hear`/`answer`/`finish` yields to the other shells' tasks;
synchronous work blocks them for its duration — the usual terms of any
cooperative loop.

### Facts are members, not parameters

Which bash a shell is, how it was started, what options it had on, which
words its join brought — all of that is settled before its first message
and cannot change while it lives (a subshell that differs is a *new* shell
with its own `$BASHPID`; `set` refuses `-i`, `-c`, `-s`). So it arrives
once, at `joined`, as `Arc<Shell>`, and a reaction that needs it keeps it
as a member:

```rust
struct Seen { shell: Arc<Shell>, captures: Vec<Capture> }

async fn joined(&self, _at: &Layout, shell: Arc<Shell>) -> Result<Seen, Failure> {
    Ok(Seen { shell, captures: Vec::new() })
}
```

This is also a proof obligation discharged by construction: owning a
reaction *is* the evidence that its shell announced itself, and a message
can only reach a reaction down that shell's own pipe — no path through the
session holds a message whose shell is unknown.

## `Layout` and `Provision` — the workspace, in your hands

`joined`, `bash`, `joining` and the environment closure all receive
`&Layout`. It is the workspace: one validated coordinate (held as text,
because it crosses into bash), plus the model of the files inside — the
constant names (`prelude.bash`, `rig.bash`, `bash_env.bash`, `join`,
`up.<tok>`, `rep.<tok>`, `lock`) exist nowhere else in the codebase.

What you actually call on it:

- `at.text()` — the directory as text, ready for
  `bash_strings::emit_scalar` when a joining line spells it;
- `at.path()` — the same, as a `&Path` for Rust's own file work;
- `at.bash_env(provision)` — the one owner of the provisioned startup
  file: writes it, and yields the `("BASH_ENV", <file>)` pair for the
  environment closure to return.

That last one takes the choice its caller must state first:

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

Why an enum and not a boolean: the two arms carry different information.
`Joining` needs the line to write (usually `&rig.joining(at)`), and
`Definitions` needs a warning attached — the file then carries no
coordinate at all, so if your scripts must find the workspace, you state a
variable for it (`BASHPROF_SESSION`, `BASHCAP_SESSION` — each tool its
own name) *beside* this pair. [joining.md](joining.md) shows both arms as whole scripts, including
what happens to a shell that has the words and never initiates.

Behind the same type sits the ownership story, told once here and assumed
everywhere else: the session `flock`s `<dir>/lock` before touching
anything and releases it after its fifos are gone. An occupied workspace
is refused whole; a predecessor killed outright leaves stale fifos that
the *next* open sweeps safely (the kernel released the dead lock); and the
`join` fifo therefore exists exactly while a session serves, which is what
makes the liveness probe — `[[ -p <dir>/join ]]`, one file test — truthful.

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

Provenance is the shape: *which shell produced this* is not a field to
match up, it is which entry you are holding. `parted` deserves a pause —
it is an `Option` because it records a genuine either/or: `Some(when)` for
a shell that finished while the session watched, `None` for a shell still
alive when the session ended (a served client that outlives the handle,
say). It is never "unknown".

When a reading wants the run flat again — every message from every shell,
in the order things were actually said:

```rust
pub struct Said<'a> { pub shell: &'a Arc<Shell>, pub message: &'a Message }

pub fn heard<K: AsRef<[Message]>>(shells: &[Attended<K>]) -> Vec<Said<'_>>;
```

Separate pipes have no arrival order *between* them, so `heard` sorts by
`Stamp::sent_at` — the sending shells' own clocks — stably over join order.
Your own `Kept` joins in by implementing `AsRef<[Message]>`. And that is
the whole accumulator story: the core ships no session-wide collector,
because what needs to be whole across shells is a resource your rig owns
(the `Sink` pattern above).

## Under the floor: the session

Nothing below is API — you never touch these types — but seeing the loop
once makes the guarantees above concrete.

Both orchestrations drive the same `Session` (a sketch of
`src/rig/session.rs`; abridged, rustdoc is authoritative):

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

Arm one: a shell announced itself on the `join` fifo. Its **account** —
which bash, how invoked, what options, the join's extra words — arrived
*with* the announcement (reassembled from frames; [wire.md](wire.md)
explains why frames exist). `announced` makes the shell's reply fifo,
opens its pipe — that open is what releases the shell from its blocking
rendezvous — builds `Shell`, awaits your `joined`, and spawns the task.
Nothing in this loop ever awaits an admitted shell.

Arm two: some shell's task finished. Its `Attended` is collected; a
`Failure` it carried ends the run here.

Arm three: the watch fired — the subject exited (driving) or the handle
was released (serving). The loop returns, and `close` does the one ending
there is: signal every task to drain what its pipe still holds, finish
every reaction, release any shell announced but not yet admitted, remove
the fifos, unlink `join` last, and hand back
`(Vec<Attended<…>>, Option<Failure>)`.

## When a rig fails

A `Failure` from `joined`, `hear` or `answer` means *the operator broke* —
not the subject. What happens next, and whose problem each case is:

| happening | whose problem, and what follows |
|---|---|
| a rig cannot hear, or cannot decide an answer | the run's: under `Driving` the subject's process group is killed, and the run returns that `Failure` |
| an answer that returns non-zero | the subject's, entirely ordinary: `set -e`, `\|\|`, or ignoring it are its own choices |
| a line on a pipe the protocol did not write, or one left half-written | the run's while serving; reported in `failed` if found at close |

The subject is *not told* when the operator breaks — an answer is a
command, and no command arriving means the asking shell blocks until the
kill or the close reaches it. And in both roles, **every path after
`Session::open` sees the session out**: whatever failed is held while
`close` runs, then returned. There is exactly one exit.

Two small types complete the error picture:

```rust
pub enum ExitStatus { Code(u8), Signal(u8) }   // shell_code(): 128 + n for a signal
```

How the *run* went and how the *subject* ended are different facts —
`Run::failed` carries the first, `ExitStatus` only the second, and no
signal disposition is ever changed. And the crate's one error:

```rust
pub struct Failure { doing: String, cause: Box<dyn Error + Send + Sync> }

pub trait Doing<T> {
    fn doing(self, what: impl FnOnce() -> String) -> Result<T, Failure>;
}
```

A context and a cause rather than an enum, because every consumer either
displays it or walks `source()` — nothing matches on variants.

## See also

- [driving.md](driving.md) / [serving.md](serving.md) — the two roles' own
  chapters
- [wire.md](wire.md) — the fifos, the lines, and what an answer is on the
  wire
- [shell.md](shell.md) — everything `Shell` knows and how it knows it
- [stack.md](stack.md) — the frame walk any instrument can reuse
- `bashcap/docs/` — a real rig that streams instead of keeping

# The rig — a reaction, and two orchestrations

`src/bash/rig/mod.rs` for the reaction, `master.rs` and `slave.rs` for the two
ways a session comes about, `serving.rs` for what they share.

A **rig** is a description: the bash it gives the subject, where the session's
files go, and how to build a reaction once a shell is there. The reaction is
`Reacting`, and it is made **per shell**, at the moment that shell announces
itself. Who started what — and therefore who ends it and cleans up — is a
second question, and each of its two answers is a trait that extends `Rig` and
carries its own orchestration.

## `Rig` and `Reacting`

```rust
pub trait Rig {
    /// What reacts to one shell.
    type Attending: Reacting;

    /// The words this rig gives the subject, laid beside the protocol's own
    /// and sourced by it. The same text in either orchestration.
    fn bash(&self) -> String;

    /// Where the session's files go, and how long they outlive it.
    fn workspace(&self) -> Workspace;

    /// A shell has joined, and everything about it is known. This is where it
    /// enters, and the last time it is a parameter.
    fn joined(&self, at: &Laid, shell: Arc<Shell>) -> Result<Self::Attending, Failure>;
}

pub trait Reacting: Sized {
    /// What is left when the shell can no longer speak.
    type Kept;

    fn hear(&mut self, said: Line) -> Result<(), Failure>;
    fn answer(&mut self, asked: Line) -> Result<Answer, Failure>;
    fn finish(self) -> Result<Self::Kept, Failure>;
}
```

**`joined` and `finish` are the only required methods.** `Rig` defaults to no
words of the subject's own and a workspace thrown away; `Reacting` defaults to
keeping nothing, and to hearing a question and telling the shell the word is
unknown (`return 127`).

Two shipped implementations cover the common cases: `Vec<Line>` keeps every
message, `()` keeps nothing. A rig that wants either needs no type of its own.

`&self` on `Rig` throughout — a rig is a description and is never mutated by
running. `&mut self` on `Reacting` — a reaction is the thing that changes.

`Line` arrives **by value**: a reaction that keeps it does so without cloning,
one that ignores it drops it for free.

### Facts are members, not parameters

Which bash a shell is, how it was started and what it had switched on are
settled before its first message and cannot change while it lives. They arrive
once, at `joined`, and a reaction that needs them keeps them:

```rust
struct Seen { shell: Arc<Shell>, captures: Vec<Capture> }

fn joined(&self, _at: &Laid, shell: Arc<Shell>) -> Result<Seen, Failure> {
    Ok(Seen { shell, captures: Vec::new() })
}
```

Owning a reaction is the whole proof that its shell announced itself: there is
no other way to construct one, so no path through the session can hold a
message whose shell is unknown. A message from a pid that never joined is a
fault and ends the run.

`Laid { dir, prelude }` is where the session's files ended up — the workspace,
and the address a shell sources to join. It is handed over for the same reason:
a reaction that resolves frame sources afterwards, or hands the address to
something it starts, knows where they are without being told twice.

### What is shared is the caller's

A reaction owns its state, so nothing is threaded through it. Where several
shells write into one thing — a file, a merged view — that thing belongs to the
rig, which hands each reaction a share:

```rust
type Sink = Rc<RefCell<BufWriter<File>>>;

struct BashCap { into: PathBuf, sink: Sink, tracing: Tracing }
struct Capturing { shell: Arc<Shell>, into: PathBuf, sink: Sink, written: usize }
```

The core names no sharing discipline and has no opinion on one. Serving is
single-threaded, so `Rc<RefCell<_>>` is enough; a rig that wants something else
uses something else.

A rig's bash and its decoder are one thing, and `Rig::bash` is where that
pairing is expressed: bashcap's rig hands over bashcap's bash, and neither can
be run without the other.

`Rig::workspace` decides how long that bash outlives the conversation. A rig
whose reading resolves frame sources afterwards names a directory it keeps,
because the instrument's own frames name a file in there and a source path is
only as readable as the file it names — see
[stack.md](stack.md#where-a-source-path-lands).

**Nothing in a rig ends a session.** A rig reacts; when the conversation is
over is decided by whoever started the shells, which is the role's business and
not the reaction's. A `Failure` is the only thing a rig can raise, and it means
the rig could not do its work rather than that it is finished.

## What a run hands back

```rust
pub struct Attended<K> { pub shell: Arc<Shell>, pub kept: K }

/// What one shell's reaction leaves behind, for a given rig.
pub type Kept<R> = <<R as Rig>::Attending as Reacting>::Kept;
```

One entry per shell, in the order they joined, each carrying what its own
reaction left behind. The provenance is the shape rather than a field: there is
no list of shells to cross-reference and nothing that could disagree with one.

Where a caller wants the run flat again:

```rust
pub struct Said<'a> { pub shell: &'a Arc<Shell>, pub line: &'a Line }

pub fn heard<K: AsRef<[Line]>>(shells: &[Attended<K>]) -> Vec<Said<'_>>;
```

`Sent::nth` counts messages over the whole run, so merging the per-shell
foldings back into arrival order is a sort on one field. A `Said` is a message
and the shell that sent it, which is what any later reading of a walk needs —
see [stack.md](stack.md#bashs-own-words). A `Kept` of the caller's own joins in
by implementing `AsRef<[Line]>`.

## `Master` — Rust orchestrates

```rust
pub trait Master: Rig {
    fn run<A: AsRef<OsStr>>(&self, argv: &[A]) -> Result<Run<Kept<Self>>, Failure>;
}

pub struct Run<K> {
    pub shells: Vec<Attended<K>>,
    pub subject: ExitStatus,
    pub failed: Option<Failure>,
}

impl<K> Run<K> {
    /// The run with its closing-up discharged.
    pub fn whole(self) -> Result<Whole<K>, Failure>;
}

pub struct Whole<K> { pub shells: Vec<Attended<K>>, pub subject: ExitStatus }
```

**The command line is run as it is given, and carries its own program.**
`rig.run(&["bash", "x.bash"])`, not `&["x.bash"]` — so a run is not bound to
bash at the top, and a caller wanting a launcher or an environment writes one
into the argv: `&["env", "TARGET=staging", "bash", "x.bash"]`. `&["make",
"test"]` works too, and every bash `make` starts joins the wire.

What a rig wants in *every* shell goes through `Rig::bash` instead, which
`BASH_ENV` carries where a command line cannot reach.

**Reaching a `Run` means bash got to its own end**, so `subject` is always the
subject's own status. `failed` is narrower: what went wrong *closing up*, after
the subject is gone, which is why it travels beside a status rather than
replacing it.

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
is released. The kill on the way out is also what collects anything that
outlived the leader; a process that means to survive detaches with `setsid`,
which leaves the group.

## `Slave` — bash orchestrates

```rust
pub trait Slave: Rig {
    fn serve<A>(&self, held: OwnedFd, announce: A) -> Result<Served<Kept<Self>>, Failure>
    where A: FnOnce(&Answer) -> Result<(), Failure>;

    fn serve_coprocess(&self) -> Result<Served<Kept<Self>>, Failure>;
}

pub struct Served<K> {
    pub shells: Vec<Attended<K>>,
    pub failed: Option<Failure>,
}
```

A script that is already running starts the server, takes the address it is
handed, and lets go when it is done. Nothing on this side starts a process or
ends one.

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

**Joining is one mechanism, not a property of either role.** How a shell learns
the address differs — a driven run puts it in `BASH_ENV`, a client that started
the server sources what it was handed — but a shell that wants in for its own
reasons sources either, and an interactive one has no other way, bash reading
`BASH_ENV` for non-interactive shells alone. Whatever brought it there, the
first thing it says is its account of itself; see [tree.md](tree.md).

`held` is a descriptor the initiator keeps open for as long as it wants the
session. Serving ends when the last holder has let go, and a client releases it
either deliberately or by dying:

```bash
exec {BASHPROF[1]}>&-      # release
wait "$BASHPROF_PID"       # the reading is on disk
```

One mechanism covers both, which is why there is no closing word, no reserved
payload word, and no interception in the serving loop. A descendant that
inherited the handle keeps the session open — correctly, since it can still
speak, and by the same rule `Master` applies to its process group.

A client that keeps talking after the session ended writes into a fifo whose
reader is gone and takes `SIGPIPE`. Releasing last — from a `trap … EXIT` — is
what a client does about it.

### The coprocess convention

Both halves of the usual arrangement are shipped, so a client writes neither.
It is one sentence: **the client holds the server's standard input, and the
server writes the address on its standard output.**

```bash
source lib/joining.bash

BC_JOIN bashprof serve --into build.times   # start it, take the address, run it
BC_INSTR say STEP compile
BC_LEAVE                                    # release, wait, return its status
```

`assets/joining.bash` is the bash half; `Slave::serve_coprocess` is the Rust
half, and a server that wants a channel of its own calls `serve` directly.
Unlike a tool's words, `joining.bash` is only ever vendored — it runs before
there is anything to inject, and it is what brings the protocol into a shell.

Both shipped tools expose this as a subcommand, so a script that wants what one
of them already does starts that rather than writing a server:

```bash
BC_JOIN bashprof serve --into build.times
BC_JOIN bashcap  serve --into capture.jsonl --trace-calls
```

`coproc` takes a literal NAME, so the session's fds live in `BC_SESSION` and
there is one per shell — the count the protocol already keeps in `__BC__owner`.
`BC_LEAVE` returns the server's status, so a client under `set -e` stops on a
server that failed, and by the time it returns whatever the server writes is
written.

`__fixtures/joined/` holds two scripts that use it. `build.bash` starts the
shipped `bashprof serve` and is driven from `tests/cli.rs`; `merging.bash`
starts `tests/joined/merging.rs`, which is a program rather than a harness
because a script that starts its own server has to have something to start —
and because that rig answers questions, which no shipped tool does.

## One exit

| | the descriptor | the session's extent | who cleans up |
|---|---|---|---|
| `Master` | a pidfd on the subject the run started | its process group | the run: kill, reap, report the status |
| `Slave` | the handle the initiator holds | whoever inherited it | nobody — not our process |

**A session lasts as long as anyone who could still speak.** One sentence for
both, and the loop below is the whole of it.

## `Serving` — what they share

```rust
struct Serving<'r, R: Rig> {
    rig: &'r R,
    laid: Laid,
    wire: Wire,
    shells: Vec<Attending<R::Attending>>,
    newest: HashMap<Pid, usize>,
    _temporary: Option<TempDir>,
}

struct Attending<A> { shell: Arc<Shell>, reacting: A }
```

`lay` writes the workspace and the pipe; nothing of the rig's exists yet,
because a reaction is built when a shell turns up rather than in advance.
`address()` is the session's only address. `newest` is which shell a later
message from a pid belongs to — a pid reused across a long run opens a new
shell rather than reopening the first.

`finish` hands back `(Vec<Attended<Kept>>, Option<Failure>)` — each role names
the fields of its own result — and reports a message left half-read before
asking any reaction to finish, since it is the earlier fault.

There is no interval and no timer.

```rust
fn drive(&mut self, until: &Until) -> Result<(), Failure> {
    while let Ready::Spoke = wait_for(&self.wire, until)? {
        self.deliver()?;
    }

    self.deliver()
}
```

`deliver` takes everything the pipe holds and places each item: an account of
itself builds a `Shell` and asks the rig for a reaction; anything else goes to
the reaction of the newest shell carrying that pid, and the answer to a shell
that asked is written before moving on. `wait_for` polls the pipe first, so a
message already waiting is read before the end is noticed, and the delivery
behind the loop takes what arrived with it.

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

## What a reaction is for

**Tracking what a run produced is entirely the client's.** The library ships no
accumulator beyond `Vec<Line>` and `()`, and no rig implementation.

| | its `Attending` → `Kept` | what it overrides |
|---|---|---|
| bashcap | `Capturing { shell, into, sink, written }` → `usize` | `bash`; `hear` decodes and writes; `finish` flushes and reports how many |
| bashprof | `Vec<Line>` → itself | `bash` alone. Every message carries the name of the call it was made inside of, so reading is one pass with a map, then two hylic folds — one to nest, one to read the tree as timings |
| `examples/snapshotting.rs` | `Seen { shell, captures }` → `Vec<Capture>` | `bash`; `hear` decodes against its own shell and keeps |
| `examples/answering.rs` | `Conversation { shell, dir, heard }` → `Vec<Line>` | `bash`; `answer` decides from what that shell said, and writes its bash into `Laid::dir` |
| `examples/streaming.rs` | `Writing { shell, into, sink, written }` → `usize` | `hear` writes, `finish` flushes — one file, every shell |
| `joined/merging.rs` | `Merges { shell, into, heard }` → `()` | the merge is a shared list; pushing as messages arrive *is* the merge |
| `proofs/answering.rs` | `Soak { steps, heard, answered }` → itself | `bash`, `hear`, `answer`; `AsRef<[Line]>` so `heard` reaches it |
| `proofs/owning.rs` | `Boom` → `()` | `answer`, which never returns |

## When the rig fails

**A `Failure` from `hear` or `answer` ends the conversation.** Under `Master` it
leaves through `?`,
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
- [tree.md](tree.md) — what a shell is, and who started whom
- [stack.md](stack.md) — the frame walk any instrument can reuse
- [vendoring.md](vendoring.md) — the words a client ships, and the hooks behind them
- [bashcap.md](bashcap.md) — a rig that streams instead of keeping

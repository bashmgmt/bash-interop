# Bash instrumentation — onboarding

What `src/bash/rig/` is, in the order a newcomer needs it: the words, the
shape, the whole public surface as code, one complete rig, what the session
does underneath, and the two tools built on it. Everything named here exists
under `mb_resolver::bash::rig` unless a path says otherwise. The layer-by-layer
reference is [README.md](README.md); the design above it is
[architecture/bash-instrumentation.md](../architecture/bash-instrumentation.md).

## What it is

Run a bash program, hear every shell in its process tree, and answer the
questions those shells ask — without changing how the program behaves when
nothing is listening. A bash script says things with one word:

```bash
BC_INSTR DEPLOY say REC compiled "$target"     # ship an arglist and carry on
BC_INSTR DEPLOY ask which target               # ship one, block, run what comes back
```

and a Rust program hears them, one *reaction* per shell, and hands back what
each shell said or what the reaction made of it.

## The words

| term | what it names |
|---|---|
| **subject** | the bash program under instrumentation: the command line a driven run starts, or the script that started a server |
| **shell** | one bash process that joined. A `( … )`, a `$( … )`, a `bash -c` are each a shell of their own; `Shell` is what one is |
| **session** | one run: a workspace, a control fifo, a pipe and a task per shell, until the *watch* fires |
| **address** | the session's one entry point — `<dir>/session.bash`, the generated invocation a shell sources to join. `BC_SESSION` in a driven subject's environment; a serving client chose `<dir>` itself. Its dirname is the workspace |
| **label** | the word after `BC_INSTR` and `BC_JOIN`. A bash-side lookup key so one process can hold several sessions; Rust is never told it |
| **rig** | a description: the label its words speak under, the bash the subject gets, how a reaction is built once a shell is there. `Rig` |
| **reaction** | what one shell talks to, for as long as it can speak. `Reacting`, made per shell by `Rig::joined`, run as a task of its own |
| **message** | one arglist a shell shipped, with the verb (`say`/`ask`) and two clocks. `Message` |
| **answer** | one command a blocked shell is told to run. `Answer` |
| **account** | what a shell says of itself when it announces: which bash, how it was started, what it had on. It becomes `Shell`, and a reaction is built from it |
| **kept** | what a reaction leaves behind when its shell is gone. `Reacting::Kept`; `Attended::kept` |
| **driving / serving** | who started the shells: Rust ran a command line and owns it, or a bash script started the server and holds the handle. `Driving`, `Serving` |
| **reaching** | how a driven subject's shells find the address: `BASH_ENV`, or by hand. The run's choice, not the rig's — `Reached { rig, reaching }`; `Driving::environment` for a rig with an environment of its own |
| **invocation** | the one generated bash file, `session.bash`: source the prelude, `BC_JOIN <label> '<dir>'`, source the rig's bash. Self-contained, and it is the address |
| **workspace** | the directory holding the session's files and fifos — the session's one coordinate, passed at every join. `Layout::dir`; a serving client prescribes it (`--at`) |
| **watch** | the descriptor a session ends on — the subject's pidfd, or the handle a client holds. Observed, never signalled |

## The shape

```
 subject's process tree                     workspace <dir>/                    one current-thread runtime
 ────────────────────────                   ──────────────────                  ──────────────────────────
 bash ── source "$BC_SESSION" ─────────►    session.bash  ── the address: source prelude,
   │                                        prelude.bash     BC_JOIN LABEL '<dir>', source rig
   │                                        rig.bash      ── Setup::bash — words and effects
   │  BC_JOIN LABEL DIR ── announce ───►    join          ── frames ──►  Session::serve  ── Rig::joined ──┐
   │                 ── exec {fd}>up.tok ►  up.<token>    ── lines ───►  attend task ── Reacting::hear     │
   │  BC_INSTR ask   ◄─ read <&rep ───────  rep.<token>   ◄── answer ──             ── Reacting::answer ◄─┘
   ├─ ( subshell )   ── its own token, pipe, task ─────►                             ── Reacting::finish → Attended
   └─ bash child     ── the same
```

Every shell has a pipe of its own, so which shell said something is which pipe
it came out of; every pipe has a task of its own, so what one shell's reaction
awaits holds up nothing but that shell. The session ends when the watch fires;
nothing inside a rig ends it.

## The bash side

Two words, and a script never says anything else of the protocol's:

```bash
BC_JOIN LABEL DIR          # once, from the invocation, at source — the script never writes this
BC_INSTR LABEL say a b c   # ship the arglist and return
BC_INSTR LABEL ask a b c   # ship it, block, run the answer; its status is the answer's
```

Both name the label first, and the join binds it to the session's one
coordinate — the workspace, which everything after the join reads from the
label-keyed LUTs. A script that wants a second label spells the coordinate it
means: `BC_JOIN OTHER "${BC_SESSION%/*}"` — the address names the workspace. A label nobody joined is an error by absence:
`BC_INSTR: label NOPE is not joined at build.bash:42` on stderr, status 125.

How a script joins — `JOINING`, printed by `bashprof run --help` and `bashcap
run --help`:

```bash
# A shell under a driven run finds the address in its environment; under
# --reach bash-env it is already joined, otherwise it joins where it likes:
source "$BC_SESSION"

# A script that starts the tool itself, as a coprocess:
source lib/joining.bash
BC_START bashprof serve --at prof.d --into build.times   # start, sync, join
BC_LEAVE                                       # let go, wait, return its status

# Only if there is a session:
[[ -n ${BC_SESSION-} ]] && source "$BC_SESSION"

# The words a client vendors, and the polyfill for when nothing is listening —
# after the join, so a session's own hooks win:
source lib/bashprof.bash
declare -F __bp_begin >/dev/null || { __bp_begin() { :; }; __bp_end() { :; }; }
```

What the subject keeps: no trap installed, no builtin shadowed, no variable
exported, no name outside `BC_*`/`__BC_*`, no `set -o` change, no `eval`, its
own exit status. The one option turned on is `expand_aliases`.

## The Rust side — the whole surface

Everything a caller touches, as declared. Read the comments as the contract.

```rust
use mb_resolver::bash::rig::*;

/// A description. `&self` throughout: nothing about it changes by running.
/// No method has a default body.
pub trait Rig {
    /// What talks to one shell.
    type Reaction: Reacting;

    /// The bash the subject gets and where the session's files go.
    fn setup(&self) -> Setup;

    /// A shell has joined and everything about it is known. Build its reaction.
    /// Awaited in the accept loop: a slow `joined` delays the next join, nothing else.
    async fn joined(&self, at: &Layout, shell: Arc<Shell>) -> Result<Self::Reaction, Failure>;
}

pub struct Setup {
    /// The name the rig's words speak under: `BC_INSTR <label> …`. The session
    /// writes the join; a label that will not name a file is refused at open.
    pub label: String,
    /// The rig's own bash — words and effects, no join line. Sourced after the
    /// join; `stack::with_walk(&[…])` composes it where a walk is reported.
    pub bash: String,
}

/// Where the session's files ended up. `address` is `<dir>/session.bash` —
/// text, because it crosses into bash; validated whole at open.
pub struct Layout { pub dir: PathBuf, pub address: String }
impl Layout {
    /// The address, spelled for `BASH_ENV`: `("BASH_ENV", <address>)`.
    pub fn bash_env(&self) -> (OsString, OsString);
}

/// One shell's reaction, for as long as that shell can speak. A task of its
/// own: owns what it holds (`'static`), never sent to another thread (no `Send`).
/// Awaiting inside a method yields to the other shells' tasks. No default bodies.
pub trait Reacting: Sized + 'static {
    /// What is left when the shell is gone. `Self` where nothing is released.
    type Kept: 'static;

    /// A message nobody is waiting on. A `Failure` here ends the run.
    async fn hear(&mut self, said: Message) -> Result<(), Failure>;

    /// A message the shell is blocked on. The task writes the answer back.
    /// Saying no is `Answer::unknown()`; a `Failure` ends the run.
    async fn answer(&mut self, asked: Message) -> Result<Answer, Failure>;

    /// The shell can no longer speak; release what this held.
    async fn finish(self) -> Result<Self::Kept, Failure>;
}

impl Reacting for Vec<Message> { … }   // keeps every message, answers `unknown()`; Kept = Self
impl Reacting for ()           { … }   // keeps nothing, answers `unknown()`;      Kept = ()

/// Rust orchestrates: the run starts the command line in a workspace of its
/// own, exports the address into it, watches its pidfd, kills the group at
/// the end. Not `Send`; awaited from a current-thread runtime.
pub trait Driving: Rig {
    /// What the subject's environment gets beyond `BC_SESSION=<address>`.
    /// Nothing is a legitimate answer: the shells then join by hand.
    fn environment(&self, at: &Layout) -> Vec<(OsString, OsString)>;

    /// Provided. The command line is run as given and carries its own program.
    async fn run<A: AsRef<OsStr>>(&self, argv: &[A]) -> Result<Run<Kept<Self>>, Failure>;
}

/// The two usual answers, and the enum `--reach` parses. The core consults
/// neither: `Reached` carries them.
pub enum Reaching { BashEnv, ByHand }

/// A rig driven with one of them. How a driven subject's shells find the
/// session is the run's question, so it is stated where the run is made —
/// a rig that serves never carries it. A rig with an environment of its own
/// implements `Driving` directly instead.
pub struct Reached<R> { pub rig: R, pub reaching: Reaching }
impl<R: Rig> Rig for Reached<R>     { /* delegates */ }
impl<R: Rig> Driving for Reached<R> { /* [at.bash_env()] | [] */ }

/// Bash orchestrates: a running script named the workspace, started the
/// server, and holds a handle.
pub trait Serving: Rig {
    /// Provided. `at` is the workspace the client prescribed — required, no
    /// fallback, created if missing, left behind — so the client knows the
    /// address before this runs. `announce` is handed it once, after the
    /// session is laid: the client's blocking read is what says it is ready.
    /// Serving ends when the last holder of `held` lets go. A `Failure` still
    /// sees the session out.
    async fn serve<A>(&self, at: &Path, held: OwnedFd, announce: A)
        -> Result<Served<Kept<Self>>, Failure>
    where A: FnOnce(&str) -> Result<(), Failure>;

    /// Provided. The client started this process as a coprocess: it holds our
    /// stdin, and reads the address from our stdout. `BC_START` is the other half.
    async fn serve_coprocess(&self, at: &Path) -> Result<Served<Kept<Self>>, Failure>;
}

/// What comes back.
pub type Kept<R> = <<R as Rig>::Reaction as Reacting>::Kept;

pub struct Run<K>    { pub shells: Vec<Attended<K>>, pub subject: ExitStatus, pub failed: Option<Failure> }
pub struct Whole<K>  { pub shells: Vec<Attended<K>>, pub subject: ExitStatus }   // Run::whole(): failed discharged
pub struct Served<K> { pub shells: Vec<Attended<K>>, pub failed: Option<Failure> }

pub struct Attended<K> {
    pub shell: Arc<Shell>,
    pub kept: K,
    /// When nobody could write on its pipe any more; `None` for a shell the
    /// session outlived.
    pub parted: Option<Micros>,
}

pub enum ExitStatus { Code(u8), Signal(u8) }   // shell_code(): 128 + n for a signal

/// The run flat again, in the order it was said — by the senders' own clocks.
pub struct Said<'a> { pub shell: &'a Arc<Shell>, pub message: &'a Message }
pub fn heard<K: AsRef<[Message]>>(shells: &[Attended<K>]) -> Vec<Said<'_>>;

/// What one shell's client said, once.
pub struct Message {
    pub verb: Verb,           // Say | Ask
    pub stamp: Stamp,         // sent_at: the shell's $EPOCHREALTIME; heard_at: the run's clock at the read
    pub words: Vec<String>,   // the client's arglist and nothing of the protocol's
}
impl Message {
    /// The words after `lead`, if the message begins with it — how a decoder
    /// claims one family of messages and declines the rest.
    pub fn behind(&self, lead: &str) -> Option<&[String]>;
}
/// `key value` pairs, a payload convention a client may choose.
pub fn field<'a>(words: &'a [String], key: &str) -> Option<&'a str>;

/// One command a blocked shell runs; its status is `BC_INSTR ask`'s.
pub struct Answer(…);
impl Answer {
    pub fn of(command: impl Into<String>, args: impl IntoIterator<Item = impl Into<String>>) -> Self;
    pub fn status(code: u8) -> Self;   // `return code`
    pub fn unknown() -> Self;          // `return 127` — bash's own "command not found"
    pub fn ok() -> Self;               // `return 0`
}

/// A shell, made once from its account. `bash::shell::Shell`.
pub struct Shell {
    pub nth: usize,          // the order it joined in, from zero
    pub pid: Pid,
    pub shlvl: u32,
    pub subshell: u32,       // $BASH_SUBSHELL
    pub joined: Stamp,       // when it joined, on both clocks
    pub bash: Bash,          // version: Version, binary: PathBuf, zero: String, invocation: Invocation
    pub options: Options,    // flags: Flags ($-), shellopts, bashopts — a snapshot
}
pub struct Invocation { pub command: Option<String>, pub standard_input: bool, pub interactive: bool }
impl Invocation { pub fn from_a_file(&self) -> bool }         // whether `zero` is a path
impl Version    { pub fn at_least(&self, major: u32, minor: u32, patch: u32) -> bool }
impl Flags      { pub fn has(&self, flag: char) -> bool }

/// One error for anything that stops work. `crate::failure`.
pub struct Failure { … }                                     // Failure::new(doing, cause); Display, Error
pub trait Doing<T> { fn doing(self, what: impl FnOnce() -> String) -> Result<T, Failure>; }

/// How a script joins, in bash — every way there is. Both binaries print it.
pub const JOINING: &str;
```

## One complete rig

Keeps what each shell says, tells the subject which target to use, is driven,
and reaches its shells through `BASH_ENV`:

```rust
use std::sync::Arc;
use mb_resolver::bash::rig::*;

struct Deploying;                                        // the description

struct Told { shell: Arc<Shell>, heard: Vec<Message> }   // one shell's reaction

impl Rig for Deploying {
    type Reaction = Told;

    /// A word the subject can say in every shell, and the label it speaks under.
    fn setup(&self) -> Setup {
        Setup {
            label: "DEPLOY".into(),
            bash: "STAGE() { BC_INSTR DEPLOY say STAGE \"$@\"; }\n".into(),
        }
    }

    /// The shell is a member from construction, never a parameter afterwards.
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

    /// `BC_INSTR DEPLOY ask target` in the subject → `declare -g target=staging` runs there.
    async fn answer(&mut self, asked: Message) -> Result<Answer, Failure> {
        Ok(match asked.words.first().map(String::as_str) {
            Some("target") => Answer::of("declare", ["-g", "target=staging"]),
            _ => Answer::unknown(),
        })
    }

    async fn finish(self) -> Result<Self, Failure> { Ok(self) }
}

/// What lets `heard` flatten a run of these.
impl AsRef<[Message]> for Told {
    fn as_ref(&self) -> &[Message] { &self.heard }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Failure> {
    // Driven with the usual reach: BASH_ENV, so every non-interactive bash in
    // the tree joins as it starts. A rig with an environment of its own
    // implements `Driving` instead — tests/proofs/starting.rs shows one.
    let deploying = Reached { rig: Deploying, reaching: Reaching::BashEnv };
    let ran = deploying.run(&["bash", "deploy.bash"]).await?.whole()?;   // one entry per shell
    for at in &ran.shells {
        println!("pid {} ({}) said {} things", at.shell.pid, at.shell.bash.version, at.kept.heard.len());
    }
    for said in heard(&ran.shells) {                                      // the run flat, in the order said
        println!("pid {}: {:?}", said.shell.pid, said.message.behind("STAGE"));
    }
    println!("{}", ran.subject);                                          // exit 0 | killed by signal 15
    Ok(())
}
```

A rig that wants only the messages names `Vec<Message>` as its `Reaction` and
writes no reaction at all. What several shells share — a file, a merged view —
is the rig's, handed to each reaction as an `Rc<RefCell<_>>` share; the borrow
is never held across an `.await`.

The same rig served — a bash script starts it and joins:

```rust
impl Serving for Deploying {}
// in main: Deploying.serve_coprocess(&at).await? — `at` from this server's own CLI
```
```bash
source lib/joining.bash
BC_START ./deploying --at deploy.d # coproc; the dir is this script's choice, the
                                   # line it reads back says the session is laid
STAGE compile                      # a word the rig defined
BC_INSTR DEPLOY ask target && echo "$target"
BC_LEAVE                           # let go, wait, the server's status
```

## What the session does underneath

Names in `src/bash/rig/`: `session.rs`, `attend.rs`, `watch.rs`, `wire/`.

1. **Open.** The workspace — a temporary one under `Driving`, the client's
   prescribed `at` under `Serving` — is made and canonicalised, the label and
   the dir's spelling validated, the control fifo `<dir>/join` made and held
   read-write (so it never reaches end of input), and three files written:
   the generic `prelude.bash`, `rig.bash` (`Setup::bash`), and the generated
   invocation `session.bash` — which is `Layout::address`.
2. **A shell joins.** Sourcing the invocation sources the prelude, then runs
   `BC_JOIN LABEL '<dir>'`, which binds the coordinate and attaches: the
   shell takes its account (one array literal, its `$EPOCHREALTIME` first),
   makes `up.<token>`, writes token and account to `join` in frames of at
   most `PIPE_BUF` bytes — `<token> + <bytes>` / `<token> . <bytes>` — and
   blocks in `exec {fd}>up.<token>` until the run opens the read end; then
   `rig.bash` is sourced. The token is `<label>::<pid>.<µs>.<random>` and
   names two files, nothing else.
3. **The run admits it.** `Control::next` reassembles frames per token into
   `Announced { token, account }`; `Session::announced` makes `rep.<token>`,
   opens the pipe (releasing the shell), builds `Shell` from the account,
   awaits `Rig::joined`, and `spawn_local`s `attend`. Nothing in this loop
   awaits a shell.
4. **The task.** `attend` reads a line, decodes it as `Message` — `SAY` to
   `hear`, `ASK` to `answer` and the `Answer` down `rep.<token>` — until end
   of input (the shell parted: every holder of the write end is gone) or the
   session closes; then `finish`, then `Attended { shell, kept, parted }`.
5. **The end.** `Watch` is an `AsyncFd`: the subject's pidfd under `Driving`,
   the client's handle under `Serving`. When it fires, `Driving` kills the
   group and reaps; the session then closes: shells announced and not opened
   are released, `join` unlinked, every task reads what its pipe still holds
   and finishes.

Guarantees a caller leans on: a message is a bash array literal on one line
and arrives whole at any width (one writer per pipe); a shell that says one
thing and exits within microseconds loses nothing (the blocking open is the
rendezvous); `heard` orders by the senders' clocks; a `Failure` from any
reaction ends the run — the subject is killed under `Driving`, released under
`Serving`; a line the protocol did not write ends the run naming it. What the
proofs establish, one by one, is the table in
[measurements.md](measurements.md#what-the-proofs-establish).

## The two tools

Neither is privileged; each is a rig plus a reading. Both ship the words a
call site says as one file that is injected *and* vendored — the words name a
hook, and only the hook exists twice ([vendoring.md](vendoring.md)).

| | its bash | its rig | its reading |
|---|---|---|---|
| **bashcap** | `BASHCAP`, `WITH_BASHCAP` — the frame walk plus a snapshot: variables, `BASH_REMATCH`, notes | `BashCap::writing(into)`; `Capturing` writes one JSON object per snapshot as it arrives; `Kept = usize` | `bashcap show FILE` |
| **bashprof** | `BASHPROF_TIMETHIS label cmd…` — a call tree that travels on the wire | `BashProf`; `Vec<Message>` | `recorded(&heard(..))` → `Profile::of(..)`: records, tree, timings |

```
bashcap  run   [--reach bash-env|by-hand] --into FILE [--verbose] [--trace-calls] -- cmd…
bashcap  serve --at DIR --into FILE [--verbose] [--trace-calls]
bashcap  show  FILE
bashprof run   [--reach bash-env|by-hand] --into FILE [--output human|tree|tree-with-err|raw] -- cmd…
bashprof serve --at DIR --into FILE [--output …]
```

`run` and `serve` differ only in who started the shells; the exit code of
`run` is the subject's own.

## Where things are

```
src/bash/rig/
  mod.rs          Rig, Reacting, the two shipped reactions, the re-exports, JOINING
  joining.txt     JOINING's text
  attended.rs     Setup, Layout, Attended, Kept, Said, heard
  driving.rs      Driving, Reaching, Reached, Run, Whole, ExitStatus; Subject (the process group)
  serving.rs      Serving, Served
  session.rs      Session: open, serve, announced, close
  attend.rs       one shell's task
  watch.rs        Watch: a pidfd or a held handle
  wire/           prelude.bash, lay() — the three files; Control (frames), Lines (bytes), Pipe (lines), message.rs
src/bash/shell.rs Shell, Bash, Version, Invocation, Options, Flags
src/bash/stack/   the frame walk: with_walk, Stack, Frame
src/bashcap/, src/bashprof/, src/bin/
assets/           joining.bash, bashcap.bash, bashprof.bash — what a client vendors
tests/examples/   worked rigs, public API only — read top to bottom
tests/proofs/     bash-level proofs of the transport
tests/joined/     a program a fixture script starts
tests/cli.rs      the binaries
```

Read next: [rig.md](rig.md) for the session in detail, [wire.md](wire.md) for
the protocol line by line, [shell.md](shell.md) for the account, then the tool
that concerns you.

# The wire — the bash, the pipe, the frame, the message

`src/bash/rig/wire/`, with its bash in `src/bash/rig/wire/prelude.bash`

A named pipe every shell joins by itself, a line-oriented frame carrying
provenance and routing, and a message that is one bash array literal.

## The client surface

```bash
BC_INSTR say a b c      # ship the arglist and return
BC_INSTR ask a b c      # ship it, block, and run the answer
```

```bash
BC_INSTR() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"

    case "${1-}" in
        say) shift; [[ $BASHPID == "$__BC__owner" ]] || __bc_join
             __bc_send SAY "$@" ;;
        ask) shift; __bc_ask "$@" ;;
        *)   __bc_complain "unknown verb ${1-}"; return "$__BC__FAILED" ;;
    esac
}
```

The leading word is consumed in bash, so the arglist an answer sees is what
the subject wrote after `ask`. An unrecognised one ships nothing and is an
instrumentation failure like any other: named on stderr at the subject's call
site, and 125.

## The prelude

```rust
fn prelude(dir: &Path, bash: &str) -> Result<PathBuf, Failure>;
```

**Nothing is templated.** `prelude.bash` is shipped verbatim and finds its own
workspace:

```bash
__BC__DIR="${BASH_SOURCE[0]%/*}"
__BC__UP="$__BC__DIR/up"
__BC__narrow=$(( (__BC__PIPE_BUF - 24) / 4 ))
…
source "$__BC__DIR/rig.bash"
```

so the run lays two files into the workspace — the protocol's bash and the
rig's, always written and possibly empty — and points `BASH_ENV` at the first.
`dir` must be absolute, which is why `run_in` canonicalises it: that path is
what every shell reads its own location from.

Because the file is shipped as it is, it is real bash: `bash -n` and
`shellcheck` run on it directly, and the non-invasiveness invariants are
properties of the file rather than of a generated string.

## Error flow

Commands in `prelude.bash` that can fail are followed by `|| __BC_BAIL` or
`|| __BC_THROW` — the ordinary way bash meant to be embedded anywhere handles
its own errors. It carries weight here because a script may call `BC_INSTR`
inside an or-list, and bash disables `errexit` for everything an or-list
calls: unguarded, a function of ours would carry on past its own first
failure and report the second one instead.

```bash
shopt -s expand_aliases

alias __BC_BAIL='return $?'
alias __BC_THROW='{ __bc_complain "${FUNCNAME[0]} ($?)"; return "$__BC__FAILED"; }'
```

`__BC_BAIL` forwards the status, for a fault an inner function already named.
`__BC_THROW` names one first and returns `__BC__FAILED`.

Aliases rather than functions, because `return` has to act in the frame that
failed. They expand at parse time, so `expand_aliases` is on before anything
using them is read — the one option the protocol turns on, and it stays on, so
a subject's own aliases expand where they otherwise would not.

`$?` is read in the first command of each, which is where it survives; a
command before it would set its own.

Three things are unguarded on purpose:

| | why |
|---|---|
| the array assignment in `__bc_ask` | cannot fail — text bash cannot read as a literal becomes one element |
| running the answer | its status is the result the caller asked for |
| the closing `source` | a `BASH_ENV` file's status is discarded, and errexit does not reach its top level |

Two bash properties the code is shaped around:

- **`(( x += n ))` is a command, and its status is false when the result is
  zero.** Cursors move by assignment — `x=$(( x + n ))` — which has no status
  of its own.
- **A trapped signal does not end a blocked `read`.** Bash runs the handler
  and resumes it, so a shell waiting on an answer waits until one arrives or
  it is killed.

### What a failure looks like

```
BC_INSTR: __bc_join (1) at build.bash:42
```

One line per fault, naming the subject's own call site rather than a line of
ours. `BC_INSTR` returns **125** — what `env` and `timeout` return when the
wrapper rather than the payload failed — so *the instrumentation broke* is
distinguishable from *the answer ran and returned non-zero*, which is the
subject's own business. A verb the protocol does not define is the same kind
of fault and the same 125: it ships nothing and says which word it was.

The answer's own status is the one command deliberately left unguarded: a
subject that asked a question wants what came back.

## The pipes

| | the up pipe | `rep.<pid>` |
|---|---|---|
| named | `up`, one constant | after the shell that asks |
| lives for | the run | one question |
| created by | the run, in `Wire::create` | the asking shell, per ask |
| removed by | the workspace going | the run, with the answer |
| the run holds | `O_RDWR`, `O_NONBLOCK` | `O_WRONLY`, opened and closed per answer |
| the shell holds | `exec {__BC__fd}>"$__BC__UP"` | `exec {__bc_fd}<>"$__bc_reply"`, closed on receipt |

**A reply pipe is made for one question and removed with its answer.** A shell
is blocked from the moment it asks until the moment it reads, so the name is
free again before it can ask anything else, and `mkfifo` is a single attempt
against a name nothing else holds. Neither side accumulates: the run keeps no
descriptor per shell, and the workspace keeps no file per ask.

A shell killed between `mkfifo` and its answer leaves its pipe behind. Nothing
later meets it — a workspace belongs to one run, since creating the up pipe is
what claims the directory — and within the run the shell that left it is gone.

**Both pipes are held open at both ends by their owner.** The run holds the up
pipe `O_RDWR`, so its open never blocks, a shell exiting never looks like
end-of-stream, and the reader waits with `poll` rather than a timer. A shell
holds its reply pipe `O_RDWR` so `read` blocks on data rather than on the
open, and unlinking the name while it is open does not disturb it.

The run opens a reply pipe `O_NONBLOCK` and clears the flag once the open
succeeds. Opening a pipe to write otherwise blocks until someone reads, and a
shell that asked and then died leaves nobody to; `ENXIO` ends the run naming
that pid instead.

```rust
impl Wire {
    pub fn create(dir: &Path) -> Result<Self, Failure>;

    /// Readable exactly when the subject has said something.
    pub fn reader(&self) -> RawFd;

    /// Everything the pipe currently holds.
    pub fn drain(&mut self) -> Result<Vec<Message>, Failure>;

    /// Answer the shell blocked on a question, and remove its pipe.
    pub fn answer(&self, pid: Pid, answer: Answer) -> Result<(), Failure>;

    /// Nothing may be left half-read.
    pub fn finish(self) -> Result<(), Failure>;
}
```

`drain` hands back what it just read and forgets it. The transport keeps no
run and depends on no layer above it.

## Joining, and the fork guard

```bash
__bc_join() {
    local IFS=' '
    __BC__parent=${__BC__owner:-$PPID}

    exec {__BC__fd}>"$__BC__UP"
    __BC__owner=$BASHPID
    __BC__seq=0

    __bc_send JOIN parent … shlvl … subshell … versinfo … bash … zero … \
                   flags … shellopts … bashopts … command …
}
```

**A shell opens with an account of itself**, which carries `seq 0` and is what
says a shell has joined. Everything in it is passed as bash reports it — which
bash, how it was given its code, where it sits, what it had switched on — and
what any of that means is decided on the other side. Adding a fact is a word
here and a field there; nothing in between reads them.

That is once per shell, not once per message. None of it can change while a
shell lives: a subshell has a `$BASHPID` of its own and joins as a shell of its
own, and `set` refuses `-i`, `-c` and `-s`. Putting any of it on every message
would cost ~6 µs each where the narrow lane exists to save 7. What does change
while a shell runs — `$PWD` — is in the walk instead, which is where it is read.

Every later message carries the kind and the clock, and nothing else of the
protocol's:

```
SAY at=1755209023.481907  <the client's arglist>
```

`IFS` is taken for the joining frame alone: the version is joined with `[*]`,
which uses the subject's — see [scoping.md](scoping.md).

Every shell opens the pipe itself, by name, from a path baked into the
prelude. **Nothing is inherited**, so no descriptor has to survive a fork and
a client's own use of a particular fd cannot collide.

`$BASHPID != $__BC__owner` detects a fork — a subshell inherits the variable
but not the pid — so a `( … )` or a `$( … )` rejoins with its own descriptor
and its own sequence counter. `__bc_parent` is the inherited owner when there
is one, so a subshell names its *emitting* parent; `$PPID` there names the
grandparent.

## Frames

One line, one frame:

```
<marker> <pid> <seq> <chunk>\n
```

**The header carries only what reassembly needs**: whether more chunks follow,
and the `(pid, seq)` key they share. Everything else about a message lives
inside it and is not read until the message is whole — assembling and
interpreting are different jobs, and this is the seam between them.

**The delimiter separates frames and is part of none of them.** It is appended
after a frame is built and consumed before one is parsed, on both sides. Both
emitters render a newline inside a value as `$'\n'` — bash through `@Q`, Rust
through `emit_q_words` — so framing needs no length prefix and a value
carrying newlines arrives as one message.

```rust
struct Frame { continues: bool, pid: Pid, seq: u32, chunk: String }
```

Private to `framing.rs`: a frame exists between the read and the message. The
header sits outside the message because a continuation must be routed before
there is a message to parse. `+` means more chunks follow, `.` means this is
the last, and that is the header's entire semantic content.

Whether the sender is waiting is in the *message*, as its leading `SAY`, `ASK`
or `JOIN`, which `Arrived::read` shifts off — so the frame header stays the
smallest thing reassembly can work from. What comes out is one of two things
and they do not mix:

```rust
pub(crate) enum Arrived {
    /// A shell's account of itself, which is what makes a shell.
    Account { pid: Pid, stamp: Stamp, words: Vec<String> },

    /// Anything else it said, which presupposes one.
    Message { pid: Pid, message: Message },
}
```

An account is not a `Message`: it is what makes a shell, and a `Message`
presupposes one. `pid` is routing and stops at the serving — what a reaction
sees of the sending shell is the shell it was built with.

The frame header and the arrival clock reach the reader as one value, so no
message ever holds a placeholder:

```rust
pub(super) struct Envelope { pid: Pid, nth: u64, seq: u32, heard_at: Micros }

impl Arrived { fn read(envelope: Envelope, literal: &str) -> Result<Self, Failure>; }
```

An answer carries **no header**: the shell that asked is its only reader, so
`pipes` writes the message and a delimiter and nothing else.

```bash
__BC__PIPE_BUF=4096
__BC__narrow=$(( (__BC__PIPE_BUF - 24) / 4 ))
```

Every frame is one atomic write, so it fits `PIPE_BUF` whole. The limit lives
in the bash because only the writer splits: reassembly is driven by the `+`/`.`
marker, so Rust never needs the number.

**Two widths, because the boundary is bytes and bash counts characters.**
`__BC__narrow` is what fits however the text is encoded — a character is at
most four bytes, the header and delimiter under 24 — and a message that narrow
is written where it is built. Anything wider goes through `__bc_frame`, which
takes `LC_ALL` for its own frame so `${#…}` and `${…:from:room}` count the
bytes `PIPE_BUF` counts, fills the frame exactly, chunks with `+`, terminates
with `.`, and reuses one header so every chunk shares a `(pid, seq)` — the
reassembly key.

`LC_ALL` is a `local`, restored when the frame returns and gone before the
subject runs anything of its own; the message itself was quoted before it, in
the subject's own locale, so nothing about the wire's content changes. See
[measurements.md](measurements.md#the-pipe_buf-boundary).

A byte-exact cut can fall **inside a character**, which is why reassembly
joins chunks as bytes and decodes the message once, at its last chunk.

```rust
#[derive(Default)]
pub struct Reassembly {
    bytes: Vec<u8>,
    partial: HashMap<(Pid, u32), Vec<u8>>,
    read: u64,
}

impl Reassembly {
    pub fn feed(&mut self, bytes: &[u8], heard_at: Micros) -> Result<Vec<Arrived>, Failure>;
    pub fn finish(self) -> Result<(), Failure>;
}
```

`read` counts the messages the run has completed, and that count is what a
message carries as `Stamp::nth`. The run folds per shell, so each fold keeps its
own order and nothing else; this is what puts them back together.

The buffer is **bytes**, not text: a read boundary falls anywhere, including
inside a multi-byte character, so a frame is decoded only once the delimiter
has said where it ends. `finish` fails if a frame lacks its delimiter or a
message lacks its last chunk.

`feed` finds every frame in the buffer before cutting it once. Taking them off
the front one at a time would rescan and move the remainder behind each frame,
which is quadratic in the frames one read carries — and a 64 KB read off a
busy pipe carries hundreds.

The clock is an argument rather than a call inside the fold. `Wire::drain`
reads it once per `read`, since everything one read returns arrived at one
moment, and bytes to messages stays a pure function. A system clock behind the
epoch fails the run rather than reading as zero.

## Messages

One bash array literal — `declare -a x="$msg"` on the bash side,
`parse_array` on the Rust side, the same shape both ways.

```rust
/// What one shell's client said, once.
pub struct Message {
    pub verb: Verb,        // Say or Ask
    pub stamp: Stamp,
    pub words: Vec<String>,
}

/// When one message was written and when it arrived.
pub struct Stamp {
    pub nth: u64,          // counted over the whole run, in arrival order
    pub seq: u32,          // counted per shell, from its account at 0
    pub sent_at: Micros,   // the sending shell's $EPOCHREALTIME
    pub heard_at: Micros,  // the run's clock when the last frame arrived
}

impl Line {
    /// The words after `lead`, if this message begins with it.
    pub fn behind(&self, lead: &str) -> Option<&[String]>;
}
```

**Nothing about the shell is here.** Its pid, its parent and its `$SHLVL` do
not change while it lives, so they are what it said once on joining and are
reached through the shell a reaction was handed — see [tree.md](tree.md).

```rust
/// Value of the first `key value` pair with this key.
pub fn field<'a>(words: &'a [String], key: &str) -> Option<&'a str>;
```

`field` reads a *payload* convention a client may choose. It has nothing to do
with the `key=value` headers the protocol writes in front of one, which
`Ahead::header` shifts back off before a client ever sees the words.

`words` is what the subject passed, in order, an empty arglist included.
`behind` and `field` are conveniences a decoder opts into. An element may
itself be a literal, decoded with `Schema::n_d(k)`, which is how a payload
carries structure without sentinel words.

Both clocks are kept because they answer different questions: `sent_at` orders
messages across the process tree as the shells saw it, `heard_at` says when
the run learned of one. Neither orders a run: everything one read carried
shares a `heard_at`, so `nth` is what arrival order is spelled as.

### Typed decoding

There is no trait. A decoder is a function of the shape

```rust
fn timing(line: &Line) -> Option<Result<Timing, String>>;
```

`None` means *not this family's message*: some other tool's, and no error.
`Some(Err)` means recognised and malformed. The two levels separate sharing a
wire from failing to decode. `Snapshot::of` is this shape.

## Asking

```bash
__bc_ask() {
    [[ $BASHPID == "$__BC__owner" ]] || __bc_join

    local __bc_reply="$__BC__DIR/rep.$BASHPID"
    local __bc_fd
    mkfifo "$__bc_reply"
    exec {__bc_fd}<>"$__bc_reply"

    __bc_send ASK "$@"

    local __bc_line
    IFS= read -r __bc_line <&"$__bc_fd"
    exec {__bc_fd}>&-

    local -a __bc_answer="$__bc_line"
    "${__bc_answer[@]}"
}
```

`local -a` is bash's own parser unpacking the array literal; the shell then
runs it, and its status becomes `BC_INSTR ask`'s.

```rust
/// One command, as an arglist — the same shape a message has, on the same
/// wire, encoded the same way.
pub struct Answer(Vec<String>);

impl Answer {
    /// A command and the arguments it is given.
    pub fn of(
        command: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self;

    /// Return `code` and nothing else. `u8` is what bash's `return` carries.
    pub fn status(code: u8) -> Self;

    /// A word this rig has no answer for: `return 127`, bash's own "command
    /// not found". What a `Reacting` that only listens writes in `answer`.
    pub fn unknown() -> Self;

    /// Ran, and nothing to say about it: `return 0`.
    pub fn ok() -> Self;
}
```

An answer is a command array, so it reaches anything the shell knows,
including words the prelude defined. It performs no I/O: an answer that wants
to send more bash than one command's worth writes a file **wherever it likes**
and names it — `Answer::of("source", [path])`.

The command word stands apart from its arguments because a command of no words
is not one: an empty array runs nothing and leaves the shell holding whatever
status it had. Splitting the signature is what makes that unrepresentable
rather than checked.

Assignments made by a sourced step are global and reach the client; a `local`
in one would not, and is the single thing a step must avoid.

## See also

- [rig.md](rig.md) — who calls `drain` and `answer`
- [tree.md](tree.md) — the views `parent` and `seq` make possible
- [measurements.md](measurements.md) — the frame limit, and what each proof establishes

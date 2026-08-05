# The wire — pipes, frames, messages

`src/bash/rig/wire/`, with its bash in `bash/rig/wire.bash`

Three things stacked: a named pipe every shell joins by itself, a line-oriented
frame that carries provenance and routing, and a message that is one bash array
literal.

One rule shapes all of it:

> **Every pipe is held open at both ends by its owner.**

The `up` pipe is the operator's, so the operator holds it `O_RDWR` — the open
never blocks, a shell exiting never looks like end-of-stream, and the reader
can wait on it with `poll` rather than a timer. A shell's reply pipe is that
shell's, and it holds it `O_RDWR` for the same reasons in mirror, which is why
the operator's write never blocks and never sees `ENXIO`. Breaking that
symmetry on the reply pipe used to cost 200 µs per ask.

## The client surface

One name, two operations:

```bash
BC_INSTR say a b c      # ship the arglist and return
BC_INSTR ask a b c      # ship it, block, and continue with the answer
```

```bash
BC_INSTR() {
    local __BC__at="${BASH_SOURCE[1]:-}:${BASH_LINENO[0]:-0}:${FUNCNAME[1]:-main}"
    local __bc_op=${1:-}
    shift || return 2
    case "$__bc_op" in
        say) __bc_say "$@" ;;
        ask) __bc_ask "$@" ;;
        *)   return 2 ;;
    esac
}
```

The leading word is consumed here, in bash, to pick the operation; it never
reaches Rust, so the arglist an answer sees is what the subject wrote after
`ask`. That is a different thing from the payload convention described below,
and it is the only place in the system where a position means something.

`__BC__at` is taken **at the root**, because only here is `FUNCNAME[1]` the
client rather than one of our own frames. Bash scopes locals dynamically, so
every subfunction sees it without being handed it. Nothing reports it yet
beyond the debug log; the mechanism is what a later operation needs.

## The pipes

Two kinds, and the difference matters.

| | `up` — one, shared | `rep.<pid>` — one per asking shell |
|---|---|---|
| created by | the operator, in `Wire::create` | the asking shell, on its first ask |
| the operator holds | `O_RDWR`, `O_NONBLOCK`, 1 MiB buffer | `O_WRONLY`, opened once per shell and cached |
| the shell holds | `exec {__BC__up}>"$__BC__UP"` | `exec {__BC__replyfd}<>"$__BC__reply"` |

```rust
impl Wire {
    pub fn create(dir: &Path) -> std::io::Result<Self>;
    pub fn up_path(&self) -> &Path;

    /// The descriptor to wait on. Readable exactly when the subject has said
    /// something.
    pub fn reader(&self) -> RawFd;

    /// Takes everything the pipe currently holds, and hands back whoever is
    /// now blocked waiting for an answer.
    pub fn drain(&mut self) -> std::io::Result<Vec<Ask>>;

    pub fn seen(&self) -> &Capture;
    pub fn answer(&mut self, pid: Pid, reply: Reply) -> std::io::Result<()>;
    pub fn finish(self) -> Capture;
}
```

Each flag earns its place:

- **`O_RDWR` on the reader.** A FIFO opened read-only blocks until a writer
  appears, and returns end-of-file once the last writer closes. Holding a write
  end ourselves means `create` never blocks, the writer count never reaches
  zero, and bash's write-only `exec` never blocks either.
- **`O_NONBLOCK` on the reader.** The caller decides when to wait, with `poll`;
  `drain` reads until the pipe is empty and returns.
- **1 MiB pipe buffer.** The subject keeps writing while the operator is busy
  answering someone else, without stalling on a full pipe.

### Joining, and the fork guard

Nothing is inherited. Every shell opens the pipe itself, from a path baked into
the prelude:

```bash
__bc_join() {
    local __bc_parent=${__BC__owner:-$PPID}
    exec {__BC__up}>"$__BC__UP"
    __BC__owner=$BASHPID
    __BC__seq=0
    __BC__reply="$__BC__DIR/rep.$BASHPID"
    set -- __ORIGIN__ parent "$__bc_parent" shlvl "$SHLVL" source "${BASH_SOURCE[-1]:-}"
    __bc_pack "$@"
    __bc_ship '.'
}
```

Because no descriptor has to survive a fork there is no bash-version surface at
all, and `exec {var}>` allocates a descriptor ≥ 10, so a client using fd 3 or 4
cannot collide with us.

Every send is preceded by

```bash
[[ $BASHPID == "$__BC__owner" ]] || __bc_join
```

which is the fork detector, and it catches both cases that exist:

| scenario | `__BC__owner` at the guard | `parent` recorded |
|---|---|---|
| first call in the top shell | `""`, the prelude just ran | `$PPID` — the rig's own process |
| `( … )` subshell | the parent's pid, inherited | `${__BC__owner}` — the emitting parent, exactly |
| `bash child.sh` | `""`, the prelude re-ran via `BASH_ENV` | `$PPID` — the parent process |

`$PPID` alone would be wrong for the subshell: inside one it names the
*grandparent*. Reading the inherited `__BC__owner` before overwriting it is
what makes the process forest true.

The guard sits at the top of `__bc_say` and `__bc_ask`, which are the only two
places a message is sent from, so there is no way to write through a stale
descriptor by accident.

## Frames

One line, one frame:

```
<at> <pid> <seq> <kind> <chunk>
```

```rust
pub enum Kind { Continues, Post, Ask }   // '+'  '.'  '?'

pub struct Frame { pub stamp: Stamp, pub kind: Kind, pub chunk: String }
impl Frame { pub fn parse(raw: &str) -> Result<Self, WireError>; }
```

The header sits **outside** the message because a continuation has to be routed
before there is a message to parse. `kind` carries reply-expectation at the
transport layer, so "someone is blocked on this" is a property of the parsed
type rather than something inferred from a payload.

```rust
pub const FRAME_LIMIT: usize = 3900;
```

Below `PIPE_BUF` (4096) with room for the header, so every frame is one atomic
write and concurrent shells cannot interleave. A longer message goes through
`__bc_split`, which chunks it with `+` and terminates with the real kind,
reusing one header so every chunk shares a `seq`. That pair is the reassembly
key in `Wire::accept`.

This is not theoretical: unframed, eight concurrent writers emitting 9000-byte
messages produced 16 mangled lines and lost 8 messages outright.

## Messages

A message is **one bash array literal** — `declare -a x="$msg"` on the bash
side, `QuotedNest::parse_literal` on the Rust side, the same shape in both
directions.

```rust
pub struct Record { pub words: Vec<String> }

impl Record {
    pub fn new(words: impl IntoIterator<Item = impl Into<String>>) -> Self;
    pub fn behind(&self, lead: &str) -> Option<&[String]>;
    pub fn parse_message(literal: &str) -> Result<Self, WireError>;
    pub fn to_message(&self) -> String;
}

/// Value of the first `key value` pair with this key, over words a decoder
/// has already claimed.
pub fn field<'a>(words: &'a [String], key: &str) -> Option<&'a str>;
```

`words` is what the subject passed, in order. The rig reads no position of it.
`behind` is how a tool opts into the leading-discriminator convention, and
`field` is a convenience for the commonest payload shape — both entirely
optional.

An element may itself be a literal, decoded with `Schema::n_d(k)`, which is how
structure survives without sentinels. See [values.md](values.md#trees-and-the-two-codecs).

### Provenance

```rust
pub struct Micros(pub u64);   // from $EPOCHREALTIME; both radix characters accepted
pub struct Pid(pub u32);
pub struct Stamp { pub at: Micros, pub pid: Pid, pub seq: u32 }
pub struct Stamped<T> { pub stamp: Stamp, pub value: T }
pub type Line = Stamped<Record>;
```

The stamp is written by the **sender**, which is what makes cross-shell
chronological ordering meaningful. `Micros::parse_epoch` accepts `.` and `,`
because `$EPOCHREALTIME` uses the locale's radix character and the rig refuses
to force a locale on the subject.

### Typed decoding

```rust
pub trait FromRecord: Sized {
    type Err;
    fn from_record(record: &Record) -> Option<Result<Self, Self::Err>>;
}
```

Three outcomes, all real: `None` — not this family's record; `Some(Err)` — ours
and malformed; `Some(Ok)` — ours. `None` is what lets several tools share one
wire without the rig knowing anything about either. The idiomatic shape is
recognise, then decode:

```rust
impl FromRecord for Timing {
    type Err = String;
    fn from_record(record: &Record) -> Option<Result<Self, Self::Err>> {
        Some(Self::decode(record.behind("TIMEIT")?))
    }
}
```

## Control

The ask half, in full:

```bash
__bc_ask() {
    [[ $BASHPID == "$__BC__owner" ]] || __bc_join
    [[ -n $__BC__replyfd ]] || {
        [[ -p $__BC__reply ]] || mkfifo "$__BC__reply"
        exec {__BC__replyfd}<>"$__BC__reply"
    }

    set -- __ASK__ "$@"
    __bc_pack "$@"
    __bc_ship '?'

    local __bc_line
    IFS= read -r __bc_line <&"$__BC__replyfd"
    declare -a __bc_answer="$__bc_line"
    "${__bc_answer[@]}"
}
```

The last line is the whole of continuing: `declare -a` is bash's own parser
unpacking the array literal, and then the shell *runs it*. Its status is the
function's status, and therefore `BC_INSTR ask`'s.

This shell holds both ends of its own reply pipe, so the descriptor is opened
once however many times it asks, and `read` blocks on data rather than on the
open. Assignments in a sourced step are global and therefore reach the client;
a `local` in one would not, and is the single thing a step must avoid.

```rust
pub const ASK_TAG: &str = "__ASK__";

pub struct Ask {
    pub stamp: Stamp,
    pub args: Vec<String>,
}

/// One command, as an arglist — the same shape a message has.
pub struct Reply(Vec<String>);

impl Reply {
    pub fn of(words: impl IntoIterator<Item = impl Into<String>>) -> Self;
    pub fn nothing() -> Self;          // [":"]
    pub fn status(code: i32) -> Self;  // ["return", "<code>"]
    pub fn source(path: &Path) -> Self;
    pub fn eval(code: &str) -> Self;
    pub fn words(&self) -> &[String];
}
```

The ask travels up as an ordinary message, so breakpoints appear in the capture
like anything else; `__ASK__` is stripped before it reaches an answer, because
it is the transport's word and not the subject's.

**A reply has no variants and never will.** A bash command array can reach
anything the shell knows, so the fidelity comes from the vocabulary the prelude
defined rather than from cases here:

```text
[":"]                                    nothing
["return", "1"]                          resume with a status
["exit", "9"]                            end the shell
["source", "/…/step.bash"]               run code
["declare", "-g", "picked=elderberry"]   assign
["eval", "picked=x; note ready"]         interim, for debugging
["WITH_BASHCAP", "-BCS:probe", "deploy"] a call into the tool's own words
```

There is no "unanswered" and no "refused": a rig with nothing useful to say
answers `["return", "127"]`, and a refusal is a command that says what went
wrong and returns non-zero. That is why the rig never writes to the subject's
own streams — anything an answer wants said, the subject says.

[`Turn::source`](run.md#turn--one-question-and-everything-around-it) writes a
body into the run's workspace and hands back the command that sources it, which
is the file-free `eval` route's counterpart for anything worth keeping.

## Damage

```rust
pub struct Damage { pub reason: String, pub text: String }
```

A frame that would not parse, a message that would not decode, and any
message whose continuation never arrived are carried in `Capture::damage`
rather than dropped. `Wire::finish` converts leftover framing state into
damage on the way out.

## See also

- [design.md](design.md) — why a named pipe, why framing, and what it costs
- [capture.md](capture.md) — what the lines become
- [source.md](source.md) — the bash this document quotes, and where it lives

# The wire — pipes, frames, messages

`src/bash/rig/wire/`, with its bash in `bash/rig/wire.bash` and
`bash/rig/control.bash`

Three things stacked: a named pipe every shell joins by itself, a line-oriented
frame that carries provenance and routing, and a message that is one bash array
literal.

## The pipes

Two kinds, and the difference matters.

| | `up` — one, shared | `rep.<pid>` — one per asking shell |
|---|---|---|
| created by | Rust, in `Wire::create` | the asking shell, `mkfifo`, on its first ask |
| Rust holds | `O_RDWR`, `O_NONBLOCK`, 1 MiB buffer | opens `O_WRONLY\|O_NONBLOCK` per answer |
| bash holds | `exec {__BC__up}>"$__BC__UP"` | `read -r … < "$__BC__reply"` |

```rust
impl Wire {
    pub fn create(dir: &Path) -> std::io::Result<Self>;
    pub fn up_path(&self) -> &Path;
    pub fn drain(&mut self) -> std::io::Result<()>;
    pub fn seen(&self) -> &Capture;
    pub fn take_asks(&mut self) -> Vec<Ask>;
    pub fn answer(&mut self, pid: Pid, reply: Reply) -> std::io::Result<()>;
    pub fn flush(&mut self) -> std::io::Result<()>;
    pub fn finish(self) -> Capture;
}
```

Each flag earns its place:

- **`O_RDWR` on the reader.** A FIFO opened read-only blocks until a writer
  appears, and returns end-of-file once the last writer closes. Holding a write
  end ourselves means `create` never blocks, the writer count never reaches
  zero — so a shell exiting never looks like end-of-stream — and bash's
  write-only `exec` never blocks either, because a reader already exists.
- **`O_NONBLOCK` on the reader.** `drain` runs inside a poll loop; it must
  return `WouldBlock` rather than park the loop.
- **1 MiB pipe buffer.** The subject keeps writing between service passes
  (200 µs apart) without stalling on a full pipe.

### Joining, and the fork guard

Nothing is inherited. Every shell opens the pipe itself, from a path baked into
the prelude:

```bash
__BC__join() {
    local __bc_parent=${__BC__owner:-$PPID}
    exec {__BC__up}>"$__BC__UP"
    __BC__owner=$BASHPID
    __BC__seq=0
    __BC__reply="$__BC__DIR/rep.$BASHPID"
    declare -a __bc_origin=(__ORIGIN__ parent "$__bc_parent" shlvl "$SHLVL" source "${BASH_SOURCE[-1]:-}")
    @@POST@@
}
```

Because no descriptor has to survive a fork there is no bash-version surface at
all, and `exec {var}>` allocates a descriptor ≥ 10, so a client using fd 3 or 4
cannot collide with us.

Every send is preceded by

```bash
[[ $BASHPID == "$__BC__owner" ]] || __BC__join
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

The guard is never written by hand — see [codegen.md](codegen.md#the-guard).

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
`__BC__split`, which chunks it with `+` and terminates with the real kind,
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

The gateway, in full:

```bash
BC_INSTR() {
    [[ $BASHPID == "$__BC__owner" ]] || __BC__join
    [[ -p $__BC__reply ]] || mkfifo "$__BC__reply"

    declare -a __bc_ask=(__ASK__ "$@")
    @@ASK@@

    local __bc_line
    IFS= read -r __bc_line < "$__BC__reply"
    declare -a __bc_answer="$__bc_line"

    case "${__bc_answer[0]}" in
        source) source "${__bc_answer[1]}" ;;
        *)      return "${__bc_answer[1]}" ;;
    esac
}
```

`read < fifo` blocks in the *open*, until Rust opens the write end — that is
the rendezvous. `declare -a __bc_answer="$line"` is bash's own parser unpacking
the array literal, and `source` rather than `eval` means a continuation lands
in the caller's own scope.

```rust
pub const ASK_TAG: &str = "__ASK__";

pub struct Ask { pub stamp: Stamp, pub args: Vec<String> }

pub enum Reply {
    Continue { status: i32 },
    Source { body: BashSrc },
}
```

The ask travels up as an ordinary message, so breakpoints appear in the capture
like anything else; `__ASK__` is stripped before it reaches an answer, because
it is the transport's word and not the subject's.

`Reply` has two variants because a blocked shell can be let go two ways: with a
status, or with code. There is no third for refusal — refusal is code that says
what went wrong and returns non-zero, written by the client, which is why the
rig never writes to the subject's own streams.

`Wire::answer` writes a `Source` body to `step.<pid>.<n>.bash` so the shell only
ever has to source a path. `Wire::flush` opens the reply pipe
`O_WRONLY|O_NONBLOCK`; a shell that has not reached its `read` yet yields
`ENXIO`, and that answer stays queued for the next pass rather than stalling
the loop. The shell `mkfifo`s **before** sending, so the pipe always exists by
the time Rust sees the question.

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
- [codegen.md](codegen.md) — how a send is constructed

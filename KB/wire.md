# The wire — the bash, the fifos, the lines, the message

`src/bash/rig/wire/`, with its bash in `src/bash/rig/wire/prelude.bash`.

A control fifo every shell announces itself on — with its account, in frames
— a pipe of its own for every shell, and a message that is one bash array
literal on one line.

```
wire/  mod.rs        the paths (join, up, rep), lay(), mkfifo
       control.rs    `Control` — the join fifo: frames in, `Announced { token, account }` out; close
       lines.rs      `Lines` — a fifo read end, cut at newlines; `Raw` bytes out
       pipe.rs       `Pipe` — one shell's up + rep: next, drain, answer, close
       message.rs    `Message`, `Verb`, `Stamp`, `Micros`, `Pid`, `Answer`, `Account`, `Line`
       prelude.bash
```

## The client surface

```bash
BC_JOIN LABEL DIR word…    # once, from the rig's own bash, at source
BC_INSTR LABEL say a b c   # ship the arglist and return
BC_INSTR LABEL ask a b c   # ship it, block, and run the answer
```

The label is the early positional argument. It is a lookup key in bash —
`__BC__DIR`, `__BC__FD`, `__BC__REP`, `__BC__OWNER` are associative arrays over
it — so one process can hold several sessions, and Rust is never told it. A
label nobody joined is an error by absence: named on stderr at the call site,
status 125.

`DIR` is the session's workspace — the one coordinate, bound to the label by
the join and read from `__BC__DIR` by everything after it; `rig.bash` gets it
as `$1`, so joining a second label is a second `BC_JOIN OTHER "$1"`. A subject
script joining by hand spells any dir it can name — `${BC_SESSION%/*}`, the
address's own dirname. `BC_JOIN` refuses a relative dir, a label that will
not name a file, and a label already joined. The words after `DIR` are the
caller's, kept per label (`__BC__META`, `@Q`-quoted) and announced with every
attach — a fork's reattach carries its label's words — landing verbatim on
`Shell::brought`. The protocol itself never self-locates and reserves no word
in them.

```bash
BC_INSTR() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"

    [[ -n ${__BC__DIR[${1-}]-} ]] \
        || { __bc_complain "label ${1-} is not joined"; return "$__BC__FAILED"; }
    [[ $BASHPID == "${__BC__OWNER[$1]}" ]] || __bc_reattach "$1" || __BC_BAIL

    case "${2-}" in
        say) __bc_send "$1" SAY "${@:3}" ;;
        ask) __bc_ask "$1" "${@:3}" ;;
        *)   __bc_complain "unknown verb ${2-}"; return "$__BC__FAILED" ;;
    esac
}
```

## Three files

```rust
fn lay(dir: &Path, bash: &str) -> Result<String, Failure>;   // returns the address
```

```
<dir>/prelude.bash   generic, shipped verbatim: BC_JOIN, BC_INSTR, the internals
<dir>/rig.bash       Rig::bash — the rig's words, effects and joins
<dir>/session.bash   generated: the invocation, and the address
```

The invocation is the one generated file, and the one place the coordinate is
spelled — label and dir quoted through `emit_scalar`, so a hostile path still
joins:

```bash
source '<dir>/prelude.bash'
source '<dir>/rig.bash' '<dir>'
```

It is self-contained — correct with an empty environment — and it passes the
coordinate as the rig's bash's `$1`: `source file args…` binds positionals
for the sourced file's duration. `lay` validates the dir as one line of UTF-8
text, since the address goes into `BC_SESSION`, onto the announce line, and
into this file; the label is bash's to check, at the join. `dir` must be
absolute, which is why the session canonicalises it. Re-sourcing the
invocation in a child process re-runs the joins, which is how `BASH_ENV`
reaches a tree; re-sourcing it in a shell already joined is refused by
`BC_JOIN` (`already joined`, 125).

## Three fifos

```
<dir>/join           the control fifo — many writers, one announcement per shell, in frames
<dir>/up.<token>     one shell's pipe — one writer, one line per message
<dir>/rep.<token>    one shell's answers — one line each
```

| | made by | writers | the run holds | the shell holds |
|---|---|---|---|---|
| `join` | the run, at open | every shell, once | `O_RDWR`: never end of input | opened, written, closed per attach |
| `up.<token>` | the shell, before it announces | exactly one process | `O_RDONLY\|O_NONBLOCK`, `tokio::net::unix::pipe::Receiver` | `exec {fd}>` for its life |
| `rep.<token>` | the run, on the announcement | the run | `open_sender` per answer | `exec {fd}<>` for its life |

**The pipe has no frame; the control fifo has one.** A shell's pipe has one
writer, so nothing can interleave and no write need be atomic: a message wider
than `PIPE_BUF` is one `printf` the kernel hands over in pieces, in order, and
the reader cuts at newlines and does nothing else. The control fifo has many
writers and a write is atomic only up to `PIPE_BUF` (4096 on Linux), so what
goes there is cut into frames that each fit whole. `mkfifo` is not a builtin,
so an attach costs one fork — see [measurements.md](measurements.md).

### The control fifo

```
<token> + <bytes>\n      a frame with more to come
<token> . <bytes>\n      the last frame
```

```bash
__bc_announce() {
    local LC_ALL=C
    local __bc_room=$(( 4096 - ${#1} - 4 )) __bc_from=0
    while (( ${#2} - __bc_from > __bc_room )); do
        printf '%s + %s\n' "$1" "${2:__bc_from:__bc_room}" || __BC_THROW
        __bc_from=$(( __bc_from + __bc_room ))
    done
    printf '%s . %s\n' "$1" "${2:__bc_from}" || __BC_THROW
}
```

`local LC_ALL=C` makes `${#2}` and `${2:a:b}` count bytes, so a frame is at
most `4096` bytes whatever the account holds, and the subject's locale is back
on return ([scoping.md](scoping.md)). A frame may therefore end inside a
character. Frames of different shells interleave; `Control` keeps the bytes of
each unfinished announcement by token, appends each frame, and on `.` decodes
the whole as UTF-8 and reads it as the `Account`:

```rust
pub(crate) struct Control { lines: Lines, dir: PathBuf, partial: HashMap<String, Vec<u8>> }
pub(crate) struct Announced { pub token: String, pub account: Account }

impl Control {
    pub(crate) async fn next(&mut self) -> Result<Announced, Failure>;   // cancellation-safe
    pub(crate) fn close(self) -> Result<(), Failure>;
}
```

A line that is not a frame — no token that could name a file, no ` + ` or
` . ` after it — ends the run naming the line. `close` releases every shell
announced whole and not yet opened, drops an announcement left in the middle,
and unlinks `join`.

## Attaching: the blocking open is the rendezvous

```bash
__bc_attach() {
    local __bc_dir=${__BC__DIR[$1]}
    local __bc_tok="$1::$BASHPID.${EPOCHREALTIME#*[.,]}.${SRANDOM:-$RANDOM$RANDOM}"
    local __bc_fd __bc_rep __bc_acct

    [[ -p "$__bc_dir/join" ]] || { __bc_complain "no session at $__bc_dir"; return "$__BC__FAILED"; }
    __bc_account __bc_acct
    mkfifo "$__bc_dir/up.$__bc_tok"                                 || __BC_THROW
    __bc_announce "$__bc_tok" "$__bc_acct" >"$__bc_dir/join"        || __BC_BAIL
    exec {__bc_fd}>"$__bc_dir/up.$__bc_tok"                         || __BC_THROW
    exec {__bc_rep}<>"$__bc_dir/rep.$__bc_tok"                      || __BC_THROW
    …
}
```

Take the account, make the pipe, announce token and account together, block in
opening the pipe's write end until the run opens the read end, open the reply
pipe the run made meanwhile. The order is what makes it safe: the run cannot
open before the fifo exists, the shell cannot write before the run has opened,
and the run knows everything about the shell before it releases it. The
`[[ -p ]]` check is there because `>` would create a regular file where no
fifo is; a session that closed unlinked `join`.

**When a process attaches:** at source, from `BC_JOIN` in the rig's bash. A fork —
which sourced nothing — attaches on its first `BC_INSTR`: `$BASHPID` is not
`__BC__OWNER[label]`, so `__bc_reattach` closes the descriptors it inherited
and runs `__bc_attach`. A silent fork holds its parent's pipe open for as long
as it lives, which is right, because it could still write on it.

**The token** — `<label>::<pid>.<µs>.<random>` — names two files and appears
in nothing else. Uniqueness is structural (a pid and a microsecond); the
random tail is defence. A collision fails at `mkfifo` in the shell that chose
it, and Rust keys nothing on it.

## Lines

Every line is a bash array literal, the protocol's words in front:

```
('at=1786786563.138850' 'pid' '4711' 'shlvl' '2' … 'command' '')      the account, on the control fifo, once
('SAY'  'at=1786786563.138912' 'REC' 'compiled' 'x.rs')                every line on the shell's pipe
('ASK'  'at=…' 'which' 'target')
```

The **account** has no verb — the clock comes first — and is unbounded
(`command` is `$BASH_EXECUTION_STRING`), which is what the frames are for.
Everything in it is passed as bash reports it — see [shell.md](shell.md). The
pipe carries `SAY` and `ASK` and nothing else; any other first word ends the
run.

```rust
pub(crate) struct Raw  { pub bytes: Vec<u8>, pub heard_at: Micros }    // as read off any fifo
pub(crate) struct Line { pub text: String, pub heard_at: Micros }      // a pipe's, decoded
pub(crate) struct Account { pub stamp: Stamp, pub words: Vec<String> }  // Account::read(text, heard_at)

pub struct Message { pub verb: Verb, pub stamp: Stamp, pub words: Vec<String> }   // Message::read(line)
pub struct Stamp { pub sent_at: Micros, pub heard_at: Micros }
```

`Stamp` is two clocks: the sending shell's `$EPOCHREALTIME`, and the run's at
the read that completed the line. Nothing about the shell is here — its pid,
`$SHLVL`, `$BASH_SUBSHELL` cannot change while it lives, so they are in the
account and reached through the shell a reaction was handed.

`Message::behind(lead)` is how a decoder claims a family of messages; `field`
reads a `key value` payload convention a client may choose, unrelated to the
`key=value` headers the protocol writes in front.

`Lines` yields bytes and reads straight into its buffer at each read, so a
`next` dropped mid-await loses nothing; the only await is on readiness.
`drain` reads everything already there without waiting; `finish` reports a
line left half-written. What a line means is the reader's: `Pipe` decodes each
as UTF-8, `Control` reassembles frames first.

## Sending

```bash
__bc_send() {
    local IFS=' ' __bc_fd=${__BC__FD[$1]}
    set -- "$2" "at=$EPOCHREALTIME" "${@:3}"
    printf '(%s)\n' "${*@Q}" >&"$__bc_fd" || __BC_THROW
}
```

One line, one `printf`. `${*@Q}` joins with `IFS[0]`, hence the `local IFS`
— see [scoping.md](scoping.md).

## Asking

```bash
__bc_ask() {
    __bc_send "$1" ASK "${@:2}" || __BC_BAIL

    local __bc_line
    IFS= read -r __bc_line <&"${__BC__REP[$1]}" || __BC_THROW

    local -a __bc_answer="$__bc_line"
    "${__bc_answer[@]}"
}
```

The reply pipe was opened at attach, read-write, so the read waits for an
answer rather than seeing end of input. `local -a` is bash's own parser
unpacking the array literal; the shell then runs it, and its status becomes
`BC_INSTR ask`'s.

```rust
pub struct Answer(Vec<String>);

impl Answer {
    pub fn of(command: impl Into<String>, args: impl IntoIterator<Item = impl Into<String>>) -> Self;
    pub fn status(code: u8) -> Self;   // `return code`
    pub fn unknown() -> Self;          // `return 127`, bash's own "command not found"
    pub fn ok() -> Self;               // `return 0`
}
```

On the Rust side `Pipe::answer` opens `rep.<token>` with `open_sender`, fresh
per answer: the open is the liveness rendezvous, the answer-side mirror of the
join's blocking open — never blocks, `ENXIO` if the asker died — and awaits
`write_all`, so an answer
past the pipe's buffer holds up nothing but that shell. An answer that wants
to send more bash than one command's worth writes a file wherever it likes and
names it: `Answer::of("source", [path])`. Assignments made by a sourced step
are global and reach the client.

## Error flow

Commands in `prelude.bash` that can fail are followed by `|| __BC_BAIL` or
`|| __BC_THROW`. A script may call `BC_INSTR` inside an or-list, and bash
disables `errexit` for everything an or-list calls: unguarded, a function of
ours would carry on past its own first failure.

```bash
shopt -s expand_aliases

alias __BC_BAIL='return $?'
alias __BC_THROW='{ __bc_complain "${FUNCNAME[0]} ($?)"; return "$__BC__FAILED"; }'
```

Aliases rather than functions, because `return` has to act in the frame that
failed; `expand_aliases` is on before anything using them is parsed — the one
option the protocol turns on, and it stays on. `$?` is read in the first
command of each.

```
BC_INSTR: label NOPE is not joined at build.bash:42
BC_INSTR: __bc_attach (1) at build.bash:7
```

One line per fault, naming the subject's own call site. `BC_INSTR` returns
**125** — what `env` and `timeout` return when the wrapper rather than the
payload failed — so *the instrumentation broke* is distinguishable from *the
answer ran and returned non-zero*.

Unguarded on purpose: the array assignment in `__bc_ask` (cannot fail), running
the answer (its status is the result), and the closing `source` (a `BASH_ENV`
file's status is discarded).

## Lifecycle

**End of input on `up.<token>` is the goodbye.** The run holds the read end
alone, so when the last holder of the write end lets go — the shell exited, or
closed its fd — the task sees end of input: that is `Attended::parted`. There
is no `PART` verb and nothing to send.

At close, the run releases every announced pipe not yet opened (its shell goes
on and takes `SIGPIPE` at its next write), unlinks `join`, and each task reads
what its pipe already holds. A shell's two fifos are unlinked when its task
ends, so a kept workspace holds names only for shells still alive.

## See also

- [rig.md](rig.md) — the session and the task
- [shell.md](shell.md) — what the account carries
- [measurements.md](measurements.md) — what the kernel does, and what each proof establishes

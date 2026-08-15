# The wire — the bash, the fifos, the lines, the message

`src/bash/rig/wire/`, with its bash in `src/bash/rig/wire/prelude.bash`.

A control fifo every shell announces itself on, a pipe of its own for every
shell, and a message that is one bash array literal on one line.

```
wire/  mod.rs        the paths (join, up, rep), prelude(), mkfifo
       control.rs    `Control` — the join fifo: tokens, close
       lines.rs      `Lines` — a fifo read end, cut at newlines
       pipe.rs       `Pipe` — one shell's up + rep: next, drain, answer, close
       message.rs    `Message`, `Verb`, `Stamp`, `Micros`, `Pid`, `Answer`, `Account`, `Line`
       prelude.bash
```

## The client surface

```bash
BC_JOIN LABEL              # once, from the rig's own bash, at source
BC_INSTR LABEL say a b c   # ship the arglist and return
BC_INSTR LABEL ask a b c   # ship it, block, and run the answer
```

The label is the early positional argument. It is a lookup key in bash —
`__BC__DIR`, `__BC__FD`, `__BC__REP`, `__BC__OWNER` are associative arrays over
it — so one process can hold several sessions, and Rust is never told it. A
label nobody joined is an error by absence: named on stderr at the call site,
status 125.

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

## The prelude

```rust
fn prelude(dir: &Path, bash: &str) -> Result<PathBuf, Failure>;
```

**Nothing is templated.** `prelude.bash` is shipped verbatim and finds its own
workspace from `${BASH_SOURCE[0]%/*}` — inside a function, the file the
function was defined in — and ends by sourcing `rig.bash` beside it. The run
lays both into the workspace and points `BASH_ENV` at the first. `dir` must be
absolute, which is why the session canonicalises it.

## Three fifos

```
<dir>/join           the control fifo — many writers, one token per line
<dir>/up.<token>     one shell's pipe — one writer, one line per message
<dir>/rep.<token>    one shell's answers — one line each
```

| | made by | writers | the run holds | the shell holds |
|---|---|---|---|---|
| `join` | the run, at open | every shell, once | `O_RDWR`: never end of input | opened, written, closed per attach |
| `up.<token>` | the shell, before it announces | exactly one process | `O_RDONLY\|O_NONBLOCK`, `tokio::net::unix::pipe::Receiver` | `exec {fd}>` for its life |
| `rep.<token>` | the run, on hearing the token | the run | `open_sender` per answer | `exec {fd}<>` for its life |

**There is no frame.** A token is under 64 bytes and one `write`, so the
control fifo needs no key. A shell's pipe has one writer, so nothing can
interleave and no write need be atomic: a message wider than `PIPE_BUF` is one
`printf` the kernel hands over in pieces, in order. The reader on both cuts at
newlines and does nothing else. `mkfifo` is not a builtin, so an attach costs
one fork — see [measurements.md](measurements.md).

## Attaching: the blocking open is the rendezvous

```bash
__bc_attach() {
    local __bc_dir=${__BC__DIR[$1]}
    local __bc_tok="$1::$BASHPID.${EPOCHREALTIME#*[.,]}.${SRANDOM:-$RANDOM$RANDOM}"
    local __bc_fd __bc_rep

    [[ -p "$__bc_dir/join" ]] || { __bc_complain "no session at $__bc_dir"; return "$__BC__FAILED"; }
    mkfifo "$__bc_dir/up.$__bc_tok"                 || __BC_THROW
    printf '%s\n' "$__bc_tok" >"$__bc_dir/join"     || __BC_THROW
    exec {__bc_fd}>"$__bc_dir/up.$__bc_tok"         || __BC_THROW
    exec {__bc_rep}<>"$__bc_dir/rep.$__bc_tok"      || __BC_THROW
    …
    __bc_account "$1" || __BC_BAIL
}
```

Make the pipe, announce it, block in opening its write end until the run
opens the read end, open the reply pipe the run made meanwhile, say who you
are. The order is what makes it safe: the run cannot open before the fifo
exists, and the shell cannot write before the run has opened. The `[[ -p ]]`
check is there because `>` would create a regular file where no fifo is; a
session that closed unlinked `join`.

**When a process attaches:** at source, from `BC_JOIN` in `rig.bash`. A fork —
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
('JOIN' 'at=1786786563.138850' 'pid' '4711' 'shlvl' '2' … 'command' '')     line one: the account
('SAY'  'at=1786786563.138912' 'REC' 'compiled' 'x.rs')                       every line after
('ASK'  'at=…' 'which' 'target')
```

The **account** is the first line on a pipe and only ever the first. Its size
is unbounded (`command` is `$BASH_EXECUTION_STRING`), which is why it travels
the single-writer pipe and not the control fifo. Everything in it is passed as
bash reports it — see [shell.md](shell.md). A first line that is not `JOIN`,
or a `JOIN` after it, is malformed and ends the run.

```rust
pub(crate) struct Line { pub text: String, pub heard_at: Micros }      // as read off a pipe
pub(crate) struct Account { pub stamp: Stamp, pub words: Vec<String> }  // Account::read(line)

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

`Lines` reads straight into its buffer at each read, so a `next` dropped
mid-await loses nothing; the only await is on readiness. `drain` reads
everything already there without waiting; `finish` reports a line left
half-written.

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

On the Rust side `Pipe::answer` opens `rep.<token>` with `open_sender` — never
blocks; `ENXIO` if the asker is gone — and awaits `write_all`, so an answer
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

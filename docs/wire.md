# The wire — the bash, the fifos, the lines, the message

This is the bottom of the stack: the protocol everything above stands on.
Read it when you want to know *exactly* what crosses between a shell and
the session — to debug a run, to extend a tool, or to convince yourself
the guarantees in [overview.md](overview.md) are real. Nothing here is
API; the chapter quotes the shipped bash itself, in hand copies of
`src/rig/wire/prelude.bash` and its neighbours.

Where things live:

```
src/rig/wire/
       mod.rs        lay(), mkfifo
       control.rs    `Control` — the join fifo: frames in, `Announced { token, account }` out
       lines.rs      `Lines` — a fifo read end, cut at newlines; `Raw` bytes out
       pipe.rs       `Pipe` — one shell's up + rep: next, drain, answer, close
       message.rs    `Message`, `Verb`, `Stamp`, `Micros`, `Pid`, `Answer`, `Account`, `Line`
       prelude.bash  the client half, shipped verbatim into every workspace
```

## The client surface

A script that takes part says three words and nothing else of the
protocol's:

```bash
BC_JOIN LABEL DIR word…    # once: bind the label, announce, attach
BC_INSTR LABEL say a b c   # ship the arglist and return
BC_INSTR LABEL ask a b c   # ship it, block, and run the answer
```

The **label** is a lookup key in bash — `__BC__DIR`, `__BC__FD`,
`__BC__REP`, `__BC__OWNER` are associative arrays over it — which is what
lets one process hold several sessions at once. Rust is never told the
label; it only sees pipes.

`BC_JOIN` binds the label to a workspace and refuses the malformed cases
loudly: a relative dir, a label that could not name a file, a label
already joined in this shell. The words *after* the dir are the caller's
own free payload — kept per label, `@Q`-quoted, announced with every
attach, and landing verbatim on `Shell::brought`. The protocol reserves no
word in them and never self-locates. Here is the word as shipped:

```bash
BC_JOIN() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"
    __BC__word=${FUNCNAME[0]}

    [[ -n ${1-} && $1 != */* && $1 != *[[:space:]]* ]] \
        || { __bc_complain "label ${1-} will not name a file"; return "$__BC__FAILED"; }
    [[ ${2-} == /* ]] \
        || { __bc_complain "workspace ${2-} is not an absolute path"; return "$__BC__FAILED"; }
    [[ -z ${__BC__DIR[$1]-} ]] \
        || { __bc_complain "label $1 is already joined from ${__BC__DIR[$1]}"; return "$__BC__FAILED"; }

    __BC__DIR[$1]=$2
    local __bc_label=$1 IFS=' '
    shift 2
    __BC__META[$__bc_label]="${*@Q}"
    __bc_attach "$__bc_label"
}
```

`BC_INSTR` is the speaking word. Two things to notice before the quote:
the first line records *the subject's own call site* (that is what error
messages name), and the `__BC__OWNER` check is how a **fork** — which
inherited the arrays but not a pipe of its own — attaches itself on its
first word:

```bash
BC_INSTR() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"
    __BC__word=${FUNCNAME[0]}

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

A silent fork never attaches, and holds its parent's pipe open for as long
as it lives — correctly, because it could still write on it.

## The files

The session lays two definition files and takes a lock; one more file
exists only when a run provisions it:

```
<dir>/prelude.bash    generic, shipped verbatim: the words above, the internals below
<dir>/rig.bash        Rig::bash — the rig's words; definitions only, inert to source
<dir>/lock            flock()ed for the session's life
<dir>/bash_env.bash   only when provisioned: the two sources, then the stated joining, or not
```

Neither laid file initiates — that story, and the ownership story behind
the lock (refusal of occupied workspaces, the sweep of a killed
predecessor's fifos), is told once in [rigs.md](rigs.md) and holds here
unchanged. Two wire-level facts belong to this chapter: `Layout::new`
validates the directory as one line of UTF-8 text, because it crosses into
bash and onto the announce line; and re-sourcing a *joining*
`bash_env.bash` in a child re-runs the join — that is precisely how
`BASH_ENV` reaches a whole tree — while re-running it in a shell already
joined is refused by `BC_JOIN` (`already joined`, 125).

## The fifos

```
<dir>/join           the control fifo — many writers, one announcement per shell
<dir>/up.<token>     one shell's pipe — one writer, one line per message
<dir>/rep.<token>    one shell's answers — one line each
```

| | made by | writers | the run holds | the shell holds |
|---|---|---|---|---|
| `join` | the run, at open | every shell, once | `O_RDWR`: never end of input | opened, written, closed per attach |
| `up.<token>` | the shell, before it announces | exactly one process | `O_RDONLY\|O_NONBLOCK`, async receiver | `exec {fd}>` for its life |
| `rep.<token>` | the run, on the announcement | the run | `open_sender` per answer | `exec {fd}<>` for its life |

Why three kinds, and why only one of them has a framing scheme: a fifo
write is atomic only up to `PIPE_BUF` (4096 bytes on Linux). On a shell's
*own* pipe that never matters — one writer means nothing can interleave,
so a message wider than `PIPE_BUF` is still one `printf` whose pieces
arrive in order, and the reader just cuts at newlines. The **control
fifo** is different: every shell writes its announcement there, the
announcement carries the whole account (unbounded — it includes
`$BASH_EXECUTION_STRING`), and two shells' bytes may interleave at any
`PIPE_BUF` boundary. So announcements, and only announcements, travel in
frames.

### Frames on the control fifo

Each frame fits in one atomic write, and says whether more follow:

```
<token> + <bytes>\n      a frame with more to come
<token> . <bytes>\n      the last frame
```

The sender is ten lines of bash. The one subtlety is `local LC_ALL=C`:
it makes `${#2}` and `${2:a:b}` count **bytes**, so a frame is ≤ 4096
bytes whatever the text holds — and the subject's locale is back on
return. A frame may therefore end *inside* a multibyte character, which
is fine, because reassembly happens in bytes:

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

On the Rust side, `Control` keeps the unfinished announcements' bytes per
token, appends each frame, and on the `.` frame decodes the whole as
UTF-8 and reads it as the `Account`. Its surface (abridged):

```rust
pub(crate) struct Announced { pub token: String, pub account: Account }

impl Control {
    pub(crate) async fn next(&mut self) -> Result<Announced, Failure>;   // cancellation-safe
    pub(crate) fn close(self) -> Result<(), Failure>;
}
```

A line that is not a frame — no token that could name a file, no ` + ` or
` . ` after it — ends the run naming the line: this protocol does not
guess. `close` releases every shell announced whole and not yet opened,
drops an announcement left in the middle, and unlinks `join` last.

## Attaching: the blocking open is the rendezvous

The attach is the one choreographed moment, so read it as choreography.
The shell's side:

```bash
__bc_attach() {
    local __bc_dir=${__BC__DIR[$1]}
    local __bc_tok="$1::$BASHPID.${EPOCHREALTIME#*[.,]}.${SRANDOM:-$RANDOM$RANDOM}"
    local __bc_fd __bc_rep __bc_acct

    [[ -p "$__bc_dir/join" ]] || { __bc_complain "no session at $__bc_dir"; return "$__BC__FAILED"; }
    __bc_account __bc_acct "$1"
    mkfifo "$__bc_dir/up.$__bc_tok"                                 || __BC_THROW
    __bc_announce "$__bc_tok" "$__bc_acct" >"$__bc_dir/join"        || __BC_BAIL
    exec {__bc_fd}>"$__bc_dir/up.$__bc_tok"                         || __BC_THROW
    exec {__bc_rep}<>"$__bc_dir/rep.$__bc_tok"                      || __BC_THROW

    __BC__FD[$1]=$__bc_fd
    __BC__REP[$1]=$__bc_rep
    __BC__OWNER[$1]=$BASHPID
}
```

Step by step: take the account; make *your own* pipe; announce token and
account together on the control fifo; then **block** opening your pipe's
write end. That open completes only when the run opens the read end — and
the run does that only after it has read your whole announcement and made
your reply fifo. So the ordering is airtight in both directions: the run
cannot open a fifo that does not exist yet, the shell cannot write a
message before the run is listening, and by the time the shell is
released, the run already knows everything about it. This is also why a
shell that says one thing and exits within microseconds loses nothing —
it cannot get ahead of its own admission.

(The `[[ -p ]]` check before writing is not decoration: `>` on a missing
path would create a regular *file* where a fifo should be. A session that
closed unlinked `join`, so the check is also how a late shell learns
there is nothing to join.)

**The token** — `<label>::<pid>.<µs>.<random>` — names the two fifos and
appears in nothing else. Uniqueness is structural (a pid at a
microsecond); the random tail is defence in depth. A collision fails at
`mkfifo`, in the shell that chose the token, and Rust keys nothing on it.

## What a line is

Every line on every fifo is a **bash array literal**, with the protocol's
words in front — but the shapes never share a channel:

```
('at=1786786563.138850' 'pid' '4711' … 'command' '')      the account: no verb, clock first —
                                                          once per shell, framed on the control
                                                          fifo, at the join
('SAY'  'at=1786786563.138912' 'REC' 'compiled' 'x.rs')   a message — the shell's own pipe
('ASK'  'at=…' 'which' 'target')                          the other verb; there is no third
```

Session setup and conversation cannot mix, and each reader enforces its
side: `Account::read` refuses a line with a verb where the clock goes, and
a pipe line whose first word is not `SAY` or `ASK` is refused as
"`…` is not a verb". Once a shell is admitted, nothing about it travels
again — its pipe speaks only the two verbs.

Bash's own quoted forms are the codec — `${*@Q}` on the way out,
`local -a x="$line"` or `bash-strings`' `parse_array` on the way in — so
word boundaries, newlines, tabs, and bytes bash cannot display survive
with no escape scheme of ours. The Rust value types mirror the wire
directly (abridged):

```rust
pub struct Message { pub verb: Verb, pub stamp: Stamp, pub words: Vec<String> }
pub struct Stamp   { pub sent_at: Micros, pub heard_at: Micros }
```

`Stamp` is the two clocks: the sending shell's `$EPOCHREALTIME`, and the
run's clock at the read that completed the line — which is why nothing is
ever timed in bash, and why a whole profiling tool can be "the interval
between two stamps". Note what is *not* here: the shell's pid, `$SHLVL`,
`$BASH_SUBSHELL`. Those cannot change while a shell lives, so they
travelled once, in the account, and are reached through the `Shell` your
reaction was handed.

Two reading conventions, deliberately distinct: `Message::behind(lead)`
claims a *family* of messages by first word (a decoder gets `None` for
another tool's), and `field(words, key)` reads an optional `key value`
payload convention — unrelated to the `key=value` headers the protocol
itself writes up front.

## Sending, and asking

A `say` is one write:

```bash
__bc_send() {
    local IFS=' ' __bc_fd=${__BC__FD[$1]}
    set -- "$2" "at=$EPOCHREALTIME" "${@:3}"
    printf '(%s)\n' "${*@Q}" >&"$__bc_fd" || __BC_THROW
}
```

(`${*@Q}` joins on the first character of `IFS`, hence the `local IFS` —
the full scoping story is [scoping.md](scoping.md).)

An `ask` is a write, a blocking read, and then something unusual — the
reply is *executed*:

```bash
__bc_ask() {
    __bc_send "$1" ASK "${@:2}" || __BC_BAIL

    local __bc_line
    IFS= read -r __bc_line <&"${__BC__REP[$1]}" || __BC_THROW

    local -a __bc_answer="$__bc_line"
    "${__bc_answer[@]}"
}
```

The reply pipe was opened `<>` (read-write) at attach, so the read waits
for an answer instead of hitting end of input. `local -a` is bash's own
parser unpacking the answer's array literal; the shell runs it, and its
status becomes `BC_INSTR ask`'s — which is how "the answer said no" is an
ordinary testable status in the subject.

On the Rust side, the answer is a value with four constructors:

```rust
pub struct Answer(Vec<String>);

impl Answer {
    pub fn of(command, args) -> Self;  // any command, any argv
    pub fn status(code: u8) -> Self;   // `return code`
    pub fn unknown() -> Self;          // `return 127`, bash's own "command not found"
    pub fn ok() -> Self;               // `return 0`
}
```

`Pipe::answer` opens `rep.<token>` fresh for each answer with
`open_sender`: that open is the liveness mirror of the join's blocking
open — it never blocks, and `ENXIO` means the asker died. The write is
awaited, so an answer past the pipe's buffer holds up nothing but its own
shell. An answer that wants to deliver more bash than one command's worth
writes a file wherever it likes and answers `Answer::of("source",
[path])` — assignments made by a sourced step are global and reach the
client.

## When the protocol itself fails

The prelude may not use `set -e` (the subject's options are the
subject's), so every command in it that can fail is guarded:

```bash
shopt -s expand_aliases

alias __BC_BAIL='return $?'
alias __BC_THROW='{ __bc_complain "${FUNCNAME[0]} ($?)"; return "$__BC__FAILED"; }'
```

Aliases rather than functions because `return` must act in the frame that
failed — this is the one shell option the protocol turns on
(`expand_aliases`), and it stays on. What a subject sees when the
instrumentation breaks:

```
BC_INSTR: label NOPE is not joined at build.bash:42
BC_INSTR: __bc_attach (1) at build.bash:7
```

One line per fault, naming the *subject's own call site*, and status
**125** — the code `env` and `timeout` use when the wrapper rather than
the payload failed. So three outcomes stay distinguishable at every call
site: the instrumentation broke (125), the answer ran and said no (its
own status), the command was fine (0).

Three spots are deliberately unguarded: the array assignment in
`__bc_ask` (cannot fail), running the answer (its status *is* the
result), and a `BASH_ENV` file's own `source` (bash discards its status).

## Lifecycle, in one paragraph

End of input on `up.<token>` is the goodbye: the run alone holds the read
end, so when the last write-end holder is gone — the shell exited, or
closed its fd — the task sees end of input, and that moment is
`Attended::parted`. There is no `PART` verb and nothing to send. At close,
the run releases every announced-but-unopened pipe (its shell takes
`SIGPIPE` at its next write), each task reads what its pipe already holds,
a shell's two fifos are unlinked when its task ends, and `join` is
unlinked last — so a kept workspace holds fifo names only for shells
still alive.

## See also

- [rigs.md](rigs.md) — the session loop these fifos feed
- [shell.md](shell.md) — every word the account carries
- [measurements.md](measurements.md) — the kernel facts (PIPE_BUF, fifo
  semantics) and what each proof establishes

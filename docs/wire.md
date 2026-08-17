# The wire

The protocol everything above stands on: what crosses between a shell and the
session, byte for byte. Nothing here is API. The chapter quotes the shipped
bash itself, in hand copies of `src/rig/wire/prelude.bash` and its neighbours.

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

A script that takes part has three words and nothing else of the protocol's:

```bash
BC_JOIN LABEL DIR word…       # once: bind the label, announce, attach

declare -- BC_SAY__ARG_LABEL=LABEL
BC_SAY a b c                  # ship the arglist and return

declare -- BC_ASK__ARG_LABEL=LABEL
declare -a BC_ASK__ARGS=(a b c)
BC_ASK                        # ship it, block, and run the answer here
```

`BC_SAY` and `BC_ASK` are aliases. That is what puts the answer in the frame
that asked, and it is why the two are parametrised by variables rather than by
arguments: an alias's trailing words attach to the last command of its
expansion, and for `BC_ASK` that command is the answer itself. `BC_SAY` has no
such tail, so its words ride on the right where a caller expects them.

The label is a lookup key in bash, with `__BC__DIR`, `__BC__FD`, `__BC__REP`
and `__BC__OWNER` as associative arrays over it, which lets one process hold
several sessions at once. Rust is never told the label and sees only pipes.

`BC_JOIN` binds the label to a workspace and refuses the malformed cases: a
relative dir, a label that could not name a file, a label already joined in
this shell. The words after the dir belong to the caller, and are kept per
label, `@Q`-quoted, announced with every attach, and landed verbatim on
`Shell::brought`. The protocol reserves no word in them and never
self-locates.

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
    declare __bc_label=$1 IFS=' '
    shift 2
    __BC__META[$__bc_label]="${*@Q}"
    __bc_attach "$__bc_label"
}
```

Two aliases carry what the speaking words share. `__BC_REACH` checks that the
label in `__bc_l` names a session this process holds open, and is where a fork
— which inherited the arrays but not a pipe of its own — takes its own.
`__BC_WRITE` is the one shape a message has on the wire. Both are aliases so
they run in the frame that already holds the words, which costs no call and
leaves one source for each.

```bash
alias __BC_REACH='
    [[ -n ${__BC__DIR[$__bc_l]-} ]] \
        || { __bc_complain "label $__bc_l is not joined"; return "$__BC__FAILED"; }
    [[ $BASHPID == "${__BC__OWNER[$__bc_l]}" ]] || __bc_reattach "$__bc_l" || __BC_BAIL'

alias __BC_WRITE='printf "(%s)\n" "${*@Q}" >&"${__BC__FD[$__bc_l]}" || __BC_THROW'
```

`__bc_say` is what `BC_SAY` expands to. Its first line records the subject's
own call site, which is what error messages name.

```bash
__bc_say() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"
    __BC__word=BC_SAY

    declare __bc_l=${BC_SAY__ARG_LABEL:?BC_SAY__ARG_LABEL}
    __BC_REACH

    declare IFS=' '
    set -- SAY "at=$EPOCHREALTIME" "$@"
    __BC_WRITE
}
alias BC_SAY='__bc_say'
```

A silent fork never attaches and holds its parent's pipe open for as long as
it lives, which is correct, because it could still write on it.

## The files

The session lays two definition files and takes a lock. One more file exists
only when a run provisions it.

```
<dir>/prelude.bash    generic, shipped verbatim: the words above, the internals below
<dir>/rig.bash        Rig::bash — the rig's words; definitions only, inert to source
<dir>/lock            flock()ed for the session's life
<dir>/bash_env.bash   only when provisioned: the two sources, then the stated joining, or not
```

Neither laid file initiates, and the ownership story behind the lock — refusal
of occupied workspaces, the sweep of a killed predecessor's fifos — is told
once in [rigs.md](rigs.md) and holds here unchanged.

Two wire-level facts belong to this chapter. `Layout::new` validates the
directory as one line of UTF-8 text, because it crosses into bash and onto the
announce line. And re-sourcing a joining `bash_env.bash` in a child re-runs
the join, which is how `BASH_ENV` reaches a whole tree, while re-running it in
a shell already joined is refused by `BC_JOIN` with `already joined` and
status 125.

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

Only one of the three has a framing scheme, because a fifo write is atomic
only up to `PIPE_BUF`, 4096 bytes on Linux. On a shell's own pipe that never
matters: one writer means nothing can interleave, so a message wider than
`PIPE_BUF` is still one `printf` whose pieces arrive in order, and the reader
cuts at newlines.

The control fifo is different. Every shell writes its announcement there, the
announcement carries the whole account, which is unbounded because it includes
`$BASH_EXECUTION_STRING`, and two shells' bytes may interleave at any
`PIPE_BUF` boundary. Announcements therefore travel in frames.

### Frames on the control fifo

Each frame fits in one atomic write and says whether more follow:

```
<token> + <bytes>\n      a frame with more to come
<token> . <bytes>\n      the last frame
```

The sender is ten lines of bash. `declare LC_ALL=C` makes `${#2}` and
`${2:a:b}` count bytes, so a frame is at most 4096 bytes whatever the text
holds, and the subject's locale is back on return. A frame may therefore end
inside a multibyte character, which reassembly in bytes handles.

```bash
__bc_announce() {
    declare LC_ALL=C
    declare __bc_room=$(( 4096 - ${#1} - 4 )) __bc_from=0
    while (( ${#2} - __bc_from > __bc_room )); do
        printf '%s + %s\n' "$1" "${2:__bc_from:__bc_room}" || __BC_THROW
        __bc_from=$(( __bc_from + __bc_room ))
    done
    printf '%s . %s\n' "$1" "${2:__bc_from}" || __BC_THROW
}
```

On the Rust side `Control` keeps the unfinished announcements' bytes per
token, appends each frame, and on the `.` frame decodes the whole as UTF-8 and
reads it as the `Account`. Its surface, abridged:

```rust
pub(crate) struct Announced { pub token: String, pub account: Account }

impl Control {
    pub(crate) async fn next(&mut self) -> Result<Announced, Failure>;   // cancellation-safe
    pub(crate) fn close(self) -> Result<(), Failure>;
}
```

A line that is not a frame — no token that could name a file, no ` + ` or ` . `
after it — ends the run naming the line. `close` releases every shell
announced whole and not yet opened, drops an announcement left in the middle,
and unlinks `join` last.

## Attaching

The blocking open is the rendezvous. The shell's side:

```bash
__bc_attach() {
    declare __bc_dir=${__BC__DIR[$1]}
    declare __bc_tok="$1::$BASHPID.${EPOCHREALTIME#*[.,]}.${SRANDOM:-$RANDOM$RANDOM}"
    declare __bc_fd __bc_rep __bc_acct

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

The shell takes its account, makes its own pipe, announces token and account
together on the control fifo, and then blocks opening its pipe's write end.
That open completes when the run opens the read end, and the run does that
only after reading the whole announcement and making the reply fifo. The
ordering holds in both directions: the run cannot open a fifo that does not
exist yet, the shell cannot write a message before the run is listening, and
by the time the shell is released the run knows everything about it. A shell
that says one thing and exits within microseconds cannot get ahead of its own
admission.

The `[[ -p ]]` check before writing matters because `>` on a missing path
would create a regular file where a fifo should be. A session that closed
unlinked `join`, so the check is also how a late shell learns there is nothing
to join.

The token, `<label>::<pid>.<µs>.<random>`, names the two fifos and appears in
nothing else. A pid at a microsecond is already unique and the random tail is
defence in depth. A collision fails at `mkfifo`, in the shell that chose the
token, and Rust keys nothing on it.

## What a line is

Every line on every fifo is a bash array literal with the protocol's words in
front, and the shapes never share a channel:

```
('at=1786786563.138850' 'pid' '4711' … 'command' '')      the account: no verb, clock first —
                                                          once per shell, framed on the control
                                                          fifo, at the join
('SAY'  'at=1786786563.138912' 'REC' 'compiled' 'x.rs')   a message — the shell's own pipe
('ASK'  'at=…' 'which' 'target')                          the other verb; there is no third
```

Session setup and conversation cannot mix, and each reader enforces its side.
`Account::read` refuses a line with a verb where the clock goes, and a pipe
line whose first word is not `SAY` or `ASK` is refused as not a verb. Once a
shell is admitted its pipe speaks only the two verbs, and each has a word
of its own.

Bash's own quoted forms are the codec: `${*@Q}` on the way out, `declare -a
x="$line"` or `bash-strings`' `parse_array` on the way in. Word boundaries,
newlines, tabs and bytes bash cannot display survive with no escape scheme of
ours. The Rust value types mirror the wire, abridged:

```rust
pub struct Message { pub verb: Verb, pub stamp: Stamp, pub words: Vec<String> }
pub struct Stamp   { pub sent_at: Micros, pub heard_at: Micros }
```

`Stamp` holds the two clocks, the sending shell's `$EPOCHREALTIME` and the
run's clock at the read that completed the line. That is why nothing is timed
in bash, and why a whole profiling tool is the interval between two stamps.

The shell's pid, `$SHLVL` and `$BASH_SUBSHELL` are absent from a message.
They cannot change while a shell lives, so they travelled once in the account
and are reached through the `Shell` your reaction was handed.

Two reading conventions are distinct. `Message::behind(lead)` claims a family
of messages by first word, giving a decoder `None` for another tool's.
`field(words, key)` reads an optional `key value` payload convention,
unrelated to the `key=value` headers the protocol writes up front.

## Asking, and running the answer

An ask is a write, a blocking read, and then the reply is run — but not here.
`__bc_ask` only leaves it in `__BC__ANSWER`; the alias runs it one frame out,
where the call was written.

```bash
__bc_ask() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"
    __BC__word=BC_ASK
    __BC__ANSWER=(__bc_no_answer)

    declare __bc_l=${BC_ASK__ARG_LABEL:?BC_ASK__ARG_LABEL}
    __BC_REACH

    declare IFS=' '
    set -- ASK "at=$EPOCHREALTIME" "${BC_ASK__ARGS[@]}"
    __BC_WRITE

    declare __bc_line
    IFS= read -r __bc_line <&"${__BC__REP[$__bc_l]}" || __BC_THROW

    declare -ga __BC__ANSWER="$__bc_line"
}
alias BC_ASK='__bc_ask; "${__BC__ANSWER[@]}"'
```

The reply pipe was opened `<>` at attach, so the read waits for an answer
instead of hitting end of input. `declare -ga …="$line"` is bash parsing the
reply as an array literal, using the syntax it prints itself. `${*@Q}` joins on
the first character of `IFS`, hence the scoped `IFS`; the full scoping story is
[scoping.md](scoping.md).

The two statements are sequenced with `;` rather than joined with `&&`. Under
`errexit` a failing operand of `&&` that is not the last is exempt, so a wire
fault there would be stepped over silently. Sequenced, a fault stops the shell;
and where `errexit` is off, `__BC__ANSWER` was reset to `__bc_no_answer` before
anything could fail, so the ask reports 125 rather than running an answer meant
for an earlier question.

`BC_ASK` exits with whatever the answer returned, which is how a reply that
says no reaches the subject as an ordinary, testable status.

On the Rust side the answer is a value with five constructors:

```rust
pub struct Answer(Vec<String>);

impl Answer {
    pub fn of(command, args) -> Self;   // any command, any argv
    pub fn status(code: u8) -> Self;    // `__bc_status code`
    pub fn unknown() -> Self;           // 127, bash's own "command not found"
    pub fn ok() -> Self;                // 0
    pub fn returning(code: u8) -> Self; // `return code`, in the frame that asked
}
```

`status` and `returning` differ in how far they reach. `__bc_status` is a
prelude function, so `return` inside it ends that function and leaves the ask
with a status the script can test. `returning` sends bash's own `return`, which
runs in the asking frame and ends the function holding the call site — a
capability the alias buys, and one to reach for deliberately.

A word the rig answers with has to be a function. The answer runs as
`"${__BC__ANSWER[@]}"`, and that expansion names commands, not aliases, so a
saying word meant to be called from a reply is defined as a function even where
the same rig gives scripts an alias.

`Pipe::answer` opens `rep.<token>` fresh for each answer with `open_sender`.
That open is the liveness mirror of the join's blocking open: it never blocks,
and `ENXIO` means the asker died. The write is awaited, so an answer past the
pipe's buffer holds up its own shell alone. An answer carrying more bash than
one command's worth writes a file and answers `Answer::of("source", [path])`,
and assignments a sourced step makes are global and reach the client.

## When the protocol itself fails

The prelude may not use `set -e`, since the subject's options are the
subject's, so every command in it that can fail is guarded:

```bash
shopt -s expand_aliases

alias __BC_BAIL='return $?'
alias __BC_THROW='{ __bc_complain "${FUNCNAME[0]} ($?)"; return "$__BC__FAILED"; }'
```

These are aliases because `return` must act in the frame that failed. That is
the one shell option the protocol turns on, `expand_aliases`, and it stays on.

What a subject sees when the instrumentation breaks:

```
BC_SAY: label NOPE is not joined at build.bash:42
BC_SAY: __bc_attach (1) at build.bash:7
```

One line per fault, naming the subject's own call site, with status 125 — the
code `env` and `timeout` use when the wrapper rather than the payload failed.
Three outcomes stay distinguishable at every call site: the instrumentation
broke at 125, the answer ran and said no with its own status, and the command
was fine at 0.

Three spots are unguarded. The array assignment in `__bc_ask` cannot fail,
running the answer produces the result, and a `BASH_ENV` file's own `source`
has its status discarded by bash.

## Lifecycle

End of input on `up.<token>` is the goodbye. The run alone holds the read end,
so when the last write-end holder is gone, whether the shell exited or closed
its fd, the task sees end of input, and that moment is `Attended::parted`.
There is no `PART` verb and nothing to send.

At close the run releases every announced-but-unopened pipe, whose shell takes
`SIGPIPE` at its next write; each task reads what its pipe already holds; a
shell's two fifos are unlinked when its task ends; and `join` is unlinked
last. A kept workspace therefore holds fifo names only for shells still alive.

## See also

- [rigs.md](rigs.md) — the session loop these fifos feed
- [shell.md](shell.md) — every word the account carries
- [measurements.md](measurements.md) — the kernel facts (PIPE_BUF, fifo
  semantics) and what each proof establishes

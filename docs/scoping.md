# Scoping

Every bash file this crate ships — the prelude, the walk — and every tool's
instrument built on it runs inside the subject's frames rather than beside
them. Where a name binds therefore decides what a helper writes and what its
continuation reads. This chapter is the closed set of scoping facts the
shipped bash stands on, each measured against bash 5.3.9.

## One stack, resolved by name at run time

Variables live in a stack of scopes: the global scope at the bottom, one frame
per live function call above it. A name is resolved by walking from the
innermost frame outward to the first frame holding a binding for it, so what a
function sees depends on who called it.

`local` and `declare` inside a function are the same builtin behaviour. They
create a binding in the current frame, shadowing any outer one, and it is
released when the function returns. `declare -g` writes the global scope
instead. The shipped bash says `declare` throughout, because it also works at
a script's top level, where `local` is an error — and the words are written to
be called from either.

A bare assignment writes the innermost existing binding, and creates a global
when there is none:

| | where `X=(…)` inside a callee lands |
|---|---|
| a caller declared `X` | that caller's frame |
| no frame declared `X` | the global scope, outliving every frame |

## When the walk finds nothing: `set -u`

Under `set -u` an expansion whose name binds nowhere is an expansion error
rather than a command failure. It happens while the command is being built, so
there is no command to fail and no status to test:

```bash
echo "$nope" || echo caught        # 'caught' is never printed
```

The shell exits, whatever frame it was in and whether it was running a script
or sourcing one. The `|| __BC_BAIL` and `|| __BC_THROW` discipline sits one
layer above this and cannot see it ([wire.md](wire.md)). The expansion itself
is the only place it is answerable.

A name the instrument did not set carries its default at every expansion of
it; a name it set one line earlier carries none. Where the tool set the name,
an unbound one is a defect and killing the shell is the right outcome. The
first list is short and closed:

| | unbound means |
|---|---|
| `${1-}`, `${__BC__at:-?}`, `${BASH_SOURCE[1]:-?}` | a client called a word wrong |

A tool's effect keeps its own short list under the same rule; bashprof's
`${__BP_inside-}` reads as the outermost call, stated in its own book.

`${x-}` rather than `${x:-}` wherever empty and unset are different facts.

Which forms are safe is not guessable, and these were measured on 5.3.9:

| | |
|---|---|
| `"$@"`, `"$*"` with no positional parameters | fine — exempt since 4.4 |
| `"${arr[@]}"`, `"${arr[*]@Q}"` on an unset array | fine |
| `"${arr[0]}"`, `"${#arr[@]}"` on an unset array | fatal |
| `"$1"` with no argument | fatal |
| `${!PREFIX@}` with no matches, `"${BASH_REMATCH[@]}"` unset | fine |

`declare x` leaves `x` unset; `declare x=` sets it empty. `BASHPROF_TIMETHIS`
depends on the second, since after an empty hook has run `$__BP_id` has to be
empty rather than unbound.

### The two ways in differ in what `set -u` sees

Under a provisioned run, bash reads `BASH_ENV` while the shell is still
starting, before the subject's own `set -u` line, so only function bodies
later run under it. A client that joins by its own lines has `set -u` on
first, so the top level of everything it sources — the prelude, the rig's
definitions — and its own join line run under it too. Every `__BC__*` name is
assigned before anything reads it, which is what makes the second case hold.

## `IFS` is the subject's, and `[*]` reads it

`"${arr[*]}"` joins with the first character of `$IFS`, and `$IFS` belongs to
the subject. A shipped file that joins an array takes one of its own for that
frame:

```bash
__bc_account() { declare IFS=' '; … }   # prelude: the version is "(${BASH_VERSINFO[*]@Q})"
__bc_say()     { declare IFS=' '; … }   # prelude: the line is "(${*@Q})"
__bc_capture() { declare IFS=' '; … }   # bashcap's effect does the same
```

`declare IFS=' '` is released on return, including where the subject had `IFS`
unset: the binding is dropped rather than restored to a value, so the
subject's own state comes back whichever it was. A subject running under
`IFS=,` is what finds this, since the array arrives comma-joined and reads
back as one element.

`[@]` does not join and needs nothing. Neither does `printf -v x '%s '
"${@@Q}"`, which writes its own separator.

Proved by
`tests/proofs/transparency.rs::a_clients_own_trap_and_ifs_are_untouched`,
which sets `IFS=,` and then reads the version back off the shell.

## `LC_ALL` is the subject's, and `${#s}` reads it

`${#s}` and `${s:a:b}` count in the shell's locale. The one place a shipped
file has to count bytes, cutting the account into frames of at most
`PIPE_BUF`, takes `LC_ALL=C` the same way, for that frame:

```bash
__bc_announce() {
    declare LC_ALL=C
    declare __bc_room=$(( 4096 - ${#1} - 4 )) __bc_from=0
    …
}
```

Two `declare`s rather than one, because the words of a `declare` are expanded
before it runs, so `${#1}` in the same statement would still count characters.
The same ordering is why a word's parameters cannot be set as a command prefix
on the call that reads them — see below.
An assignment to `LC_ALL` takes effect at once, and the return puts the
subject's back, unset included
([measurements.md](measurements.md#bash-constraints-that-bound-the-design)).

## The slot pattern

A helper that computes a value and then calls a continuation cannot hold that
value in its own frame. The continuation runs while the helper is still on the
stack, but the value belongs to the span rather than to the helper. The frame
that owns the lifetime declares the slot, and the helper writes through to it.

```bash
span() {
    declare -a CAPTURED             # the slot; this frame owns its lifetime
    with_capture continuation "$@"
}

with_capture() {
    CAPTURED=(…)                    # resolves to span's binding
    "$@"                            # the continuation reads it from there
}
```

Three properties follow from the `declare`, and none hold without it. The
write lands in `span`'s frame, so the binding is released when `span` returns.
A nested `span` declares its own, so an inner one leaves the outer's intact.
And nothing reaches the global scope.

The initialiser is not part of the mechanism: `declare -a CAPTURED` and
`declare -a CAPTURED=(…)` behave identically here, since nothing reads the
slot between the declaration and the helper's write.

Nesting three spans deep, reading the slot after the sub-call returns:

```
with the declaration                  without it
BEGIN A   X='depth-A'                 BEGIN A   X='depth-A'
BEGIN B   X='depth-B'                 BEGIN B   X='depth-B'
BEGIN C   X='depth-C'                 BEGIN C   X='depth-C'
END   C   X='depth-C'                 END   C   X='depth-C'
END   B   X='depth-B'                 END   B   X='depth-C'
END   A   X='depth-A'                 END   A   X='depth-C'
```

### The two ways it inverts

When the helper declares the slot, the binding is in the helper's frame. The
continuation still reads the value, because it runs below the helper, and the
span reads whatever it declared, because the helper's binding was released
before control returned:

```bash
span() { declare -a X=(marker); helper cont; }   # span sees 'marker'
helper() { declare -a X=(computed); "$@"; }      # cont sees 'computed'
```

When nobody declares the slot it lands in the global scope. It survives the
span, and a nested span overwrites the enclosing one's, as the right-hand
column above shows.

## Namerefs

`declare -n out="$1"` binds `out` to whatever name `$1` holds, resolved by the
same outward walk at each use. It carries the target's name explicitly rather
than relying on both sides agreeing on one, and it nests: each caller passes
its own slot's name.

A nameref whose own name equals its target warns and discards the write:

```
bash: local: warning: X: circular name reference
```

Execution continues and the assignment is lost, so a nameref parameter needs a
name no caller would choose. The `__bc_` prefix keeps this unreachable.

`unset -n` releases the binding without touching the target.

## Aliases, and what they can carry

`BC_SAY` and `BC_ASK` are aliases, because an alias expands textually at the
call site: what it expands to runs in the caller's frame, which is what lets an
answer `declare` there. Everything below follows from that, and each was
measured on 5.3.9.

An alias is expanded when the command using it is **parsed**, not when it runs.
The prelude is sourced as its own command, so anything parsed afterwards sees
the words — including a function defined later in the same file. What does not
work is using one in the same parse unit that defines it:

```bash
{ source "$dir/prelude.bash"; BC_SAY hello; }   # the whole group is parsed first
```

An alias's trailing words attach to the **last command of its expansion**.
`BC_SAY` expands to one command, so words written after it are the message.
`BC_ASK` expands to two — the ask, then the answer — so words after it would
become the answer's arguments; its payload goes in `BC_ASK__ARGS` instead.

A word built over the core is one command for the same reason. Written as a
prefix assignment plus a call it composes wherever a command does:

```bash
alias STAGE='BC_SAY__ARG_LABEL=DEPLOY BC_SAY STAGE'
```

Written as several statements it does not. Only the first would be guarded by
a `||`, and the rest would run unconditionally:

```bash
alias STAGE='declare -- BC_SAY__ARG_LABEL=DEPLOY; BC_SAY STAGE'   # not this
false || STAGE compile        # the declare is skipped; the say happens anyway
```

A command prefix reaches the callee at run time and is released after it, which
is what makes the one-command form work. It cannot be used for a value the same
command *expands*, because a simple command expands its words before performing
its assignments.

`$?` does not survive a word. The commands an alias runs before the payload set
it, so a status is captured on its own line first:

```bash
some_command
declare -i rc=$?
STAGE "finished $rc"
```

An answer cannot name an alias. It runs as `"${__BC__ANSWER[@]}"`, and that
expansion names commands, so a rig gives scripts an alias and gives its own
answers a function — `eval` is the exception, since it re-parses and expands
aliases again.

The parameters are ordinary variables, so a frame that sets one is visible to
anything it calls. Declaring them keeps that to the frame that meant it, and
the `<WORD>__ARG_` prefix keeps them out of a subject's way; a callee that
reads one without setting it will see an enclosing frame's.

## Subshells

A subshell receives a copy of the whole scope stack. Writes inside it resolve
against that copy and do not return:

```bash
span() { declare -a X=(before); ( X=(inside) ); }   # X is still (before)
```

The slot pattern works unchanged inside a subshell and carries nothing out of
one. This is the same boundary that makes a buffered record flushed from a
subshell's `EXIT` unrecoverable
([measurements.md](measurements.md#bash-constraints-that-bound-the-design)).

## `$?` and the frame

`$?` survives only as the first command of a block, and a right-hand side is
expanded before any name in the same statement is assigned. A status is
therefore captured on its own line, before any other command in the frame
runs:

```bash
"$@"
declare __rc=$?
```

## See also

- [measurements.md](measurements.md#bash-constraints-that-bound-the-design) —
  traps, `extdebug`, `mkfifo`, and the rest of what bounds the bash design
- [wire.md](wire.md) — the guards, which are aliases so that `return` acts in
  the frame that failed

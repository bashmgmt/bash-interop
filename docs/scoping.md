# Scoping

Every bash file this crate ships — the prelude, the walk — and every
tool's instrument built on it runs *inside the subject's frames* rather
than beside them. Where a name binds therefore decides what a helper
writes and what its continuation reads, and this chapter is the closed set
of scoping facts the shipped bash stands on. Each was measured (bash
5.3.9), not assumed.

## One stack, resolved by name at run time

Variables live in a stack of scopes: the global scope at the bottom, one frame
per live function call above it. A name is resolved by walking from the
innermost frame outward to the first frame holding a binding for it. What a
function sees therefore depends on who called it.

`local` and `declare` inside a function are the same builtin behaviour: they
create a binding **in the current frame**, shadowing any outer one, and it is
released when the function returns. `declare -g` writes the global scope
instead.

**A bare assignment writes the innermost existing binding, and creates a
global when there is none.** This is the whole mechanism:

| | where `X=(…)` inside a callee lands |
|---|---|
| a caller declared `X` | that caller's frame |
| no frame declared `X` | the global scope, outliving every frame |

## When the walk finds nothing: `set -u`

Under `set -u` an expansion whose name binds nowhere is an **expansion error**,
not a command failure. It happens while the command is being built, so there is
no command to fail and no status to test:

```bash
echo "$nope" || echo caught        # 'caught' is never printed
```

The shell exits, whatever frame it was in and whether it was running a script
or sourcing one. The `|| __BC_BAIL` / `|| __BC_THROW` discipline is one layer
above this and structurally cannot see it — see [wire.md](wire.md). The only
place it is answerable is the expansion itself.

**A name the instrument did not set carries its default at every expansion of
it; a name it set one line earlier carries none.** Where the tool set the name,
an unbound one is a defect and killing the shell is the right outcome. The
first list is short and closed:

| | unbound means |
|---|---|
| `${1-}`, `${__BC__at:-?}`, `${BASH_SOURCE[1]:-?}` | a client called a word wrong |

A tool's effect keeps its own short list under the same rule — bashprof's
`${__BP_inside-}` reads "the outermost call" — stated in its own book.

`${x-}` rather than `${x:-}` wherever empty and unset are different facts.

Which forms are safe is not guessable, and these were measured on 5.3.9:

| | |
|---|---|
| `"$@"`, `"$*"` with no positional parameters | fine — exempt since 4.4 |
| `"${arr[@]}"`, `"${arr[*]@Q}"` on an unset array | fine |
| `"${arr[0]}"`, `"${#arr[@]}"` on an unset array | **fatal** |
| `"$1"` with no argument | **fatal** |
| `${!PREFIX@}` with no matches, `"${BASH_REMATCH[@]}"` unset | fine |

`local x` leaves `x` unset; `local x=` sets it empty. `BASHPROF_TIMETHIS`
depends on the second: after an empty hook has run, `$__BP_id` has to be empty
rather than unbound.

### The two ways in differ in what `set -u` sees

Under a provisioned run, bash reads `BASH_ENV` while the shell is still
starting, **before** the subject's own `set -u` line — so only function
bodies later run under it. A client that joins by its own lines has
`set -u` on *first*, so the top level of everything it sources — the
prelude, the rig's definitions — and its own join line run under it too.
Every `__BC__*` name is assigned before anything reads it, which is what
makes the second case hold.

## `IFS` is the subject's, and `[*]` reads it

`"${arr[*]}"` joins with the first character of `$IFS`, and `$IFS` is the
subject's. A shipped file that joins an array has to take one of its own for
exactly that frame:

```bash
__bc_account() { local IFS=' '; … }     # prelude: the version is "(${BASH_VERSINFO[*]@Q})"
__bc_send()    { local IFS=' '; … }     # prelude: the line is "(${*@Q})"
__bc_capture() { local IFS=' '; … }     # bashcap's effect does the same
```

`local IFS=' '` is released on return, **including where the subject had `IFS`
unset** — the binding is dropped, not restored to a value, so the subject's own
state comes back whichever it was. A subject running under `IFS=,` is what
finds this: the array arrives comma-joined and reads back as one element.

`[@]` does not join and needs nothing. Neither does `printf -v x '%s ' "${@@Q}"`,
which writes its own separator.

Proved by `tests/proofs/transparency.rs::a_clients_own_trap_and_ifs_are_untouched`,
which sets `IFS=,` and then reads the version back off the shell.

## `LC_ALL` is the subject's, and `${#s}` reads it

`${#s}` and `${s:a:b}` count in the shell's locale. The one place a shipped
file has to count bytes — cutting the account into frames of at most
`PIPE_BUF` — takes `LC_ALL=C` the same way, for exactly that frame:

```bash
__bc_announce() {
    local LC_ALL=C
    local __bc_room=$(( 4096 - ${#1} - 4 )) __bc_from=0
    …
}
```

Two `local`s, not one: the words of a `local` are expanded before it runs, so
`${#1}` in the same statement would still count characters. An assignment to
`LC_ALL` takes effect at once, and the return puts the subject's back — unset
included ([measurements.md](measurements.md#bash-constraints-that-bound-the-design)).

## The slot pattern

A helper that computes a value and then calls a continuation cannot hold that
value in its own frame: the continuation runs while the helper is still on the
stack, but the value has to belong to the span, not to the helper. The frame
that owns the lifetime declares the slot; the helper writes through to it.

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

Three properties follow from the `declare`, and none of them hold without it:

- the write lands in `span`'s frame, so the binding is released when `span`
  returns
- a nested `span` declares its own, so an inner one leaves the outer's intact
- nothing reaches the global scope

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

## The two ways it inverts

**The helper declaring the slot** puts the binding in the helper's frame. The
continuation still reads the value, because it runs below the helper — and the
span reads whatever it declared, because the helper's binding was released
before control returned:

```bash
span() { declare -a X=(marker); helper cont; }   # span sees 'marker'
helper() { declare -a X=(computed); "$@"; }      # cont sees 'computed'
```

**Nobody declaring the slot** puts it in the global scope. It survives the
span, and a nested span overwrites the enclosing one's — the right-hand column
above.

## Namerefs

`local -n out="$1"` binds `out` to whatever name `$1` holds, resolved by the
same outward walk at each use. It carries the target's name explicitly instead
of relying on both sides agreeing on one, and it nests: each caller passes its
own slot's name.

**A nameref whose own name equals its target warns and discards the write.**

```
bash: local: warning: X: circular name reference
```

Execution continues and the assignment is lost, so a nameref parameter needs a
name no caller would choose — the `__bc_` prefix is what keeps this
unreachable.

`unset -n` releases the binding without touching the target.

## Subshells

A subshell receives a copy of the whole scope stack. Writes inside it resolve
against that copy and do not return:

```bash
span() { declare -a X=(before); ( X=(inside) ); }   # X is still (before)
```

The slot pattern therefore works unchanged inside a subshell and carries
nothing out of one. This is the same boundary that makes a buffered record
flushed from a subshell's `EXIT` unrecoverable — see
[measurements.md](measurements.md#bash-constraints-that-bound-the-design).

## `$?` and the frame

`$?` survives only as the first command of a block, and a right-hand side is
expanded before any name in the same statement is assigned. A status is
therefore captured on its own line, before any other command in the frame runs:

```bash
"$@"
local __rc=$?
```

## See also

- [measurements.md](measurements.md#bash-constraints-that-bound-the-design) —
  traps, `extdebug`, `mkfifo`, and the rest of what bounds the bash design
- [wire.md](wire.md) — the guards, which are aliases so that `return` acts in
  the frame that failed

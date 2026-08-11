# Scoping

Every bash file this crate ships — `rig/wire/prelude.bash`, `bashcap/*.bash`,
and a tool's own instrument — runs inside the subject's frames rather than
beside them. Where a name binds decides what a helper writes and what its
continuation reads.

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
  traps, `PIPE_BUF`, `extdebug`, and the rest of what bounds the bash design
- [wire.md](wire.md) — the guards, which are aliases so that `return` acts in
  the frame that failed

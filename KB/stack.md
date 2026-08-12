# The call stack

`src/bash/stack.bash` writes it, `src/bash/stack.rs` reads it. One instrument
and one reader, shared by every tool that reports where a shell is.

## What bash keeps

Five parallel arrays, maintained by the shell itself:

```
FUNCNAME     ('__bc_stack' 'BASHCAP' 'f__C' 'f__B' 'main')
BASH_SOURCE  (…)                                            aligned 1:1
BASH_LINENO  ('4' '8' '9' '10' '0')                         shifted by one
BASH_ARGC    ('2' '0' '0' '0' '1')                          aligned 1:1
BASH_ARGV    ('2' 'walk' 'x')            one flat stack, groups reversed
```

`BASH_ARGC` and `BASH_ARGV` exist only under `extdebug` — see
[bashcap.md](bashcap.md) for how that is turned on. Expanding an unset array is
not an error, including under `set -u`, so an instrument writes all five
unconditionally.

## The instrument

```bash
__bc_stack() {
    local -n __bc_stack_out="$1"

    __bc_stack_out+=(
        skip    "$2"
        funcs   "(${FUNCNAME[*]@Q})"
        sources "(${BASH_SOURCE[*]@Q})"
        lines   "(${BASH_LINENO[*]@Q})"
        argc    "(${BASH_ARGC[*]@Q})"
        argv    "(${BASH_ARGV[*]@Q})"
    )
}
```

Six expansions. Nothing sliced, summed, reversed or looped over.

`$1` names the caller's own array, so nesting works and no global is involved —
see [scoping.md](scoping.md). The nameref is `__bc_stack_out`, a name no caller
would choose, because a nameref pointing at itself **warns and discards the
write** rather than failing.

`$2` is how many leading frames belong to the instrument, counting
`__bc_stack`'s own. Both current callers pass 2: their own function and this
one.

Each section is a bash array literal, read back with `parse_array` — see
[values.md](values.md).

## The reader

```rust
pub struct Frame {
    pub funcname: String,
    pub source: String,
    pub lineno: u32,
    pub args: Option<Vec<String>>,
}

/// A walk, innermost first. Never empty, and one array in JSON.
pub struct Stack { /* private */ }

impl Stack {
    pub fn of(frames: Vec<Frame>) -> Option<Self>;   // None for no frames
    pub fn at(&self) -> &Frame;                      // where the walk was taken
    pub fn outer(&self) -> &[Frame];                 // the frames above it
    pub fn frames(&self) -> impl Iterator<Item = &Frame>;
}

pub struct Args<'a>    { pub argc: &'a str, pub argv: &'a str }
pub struct Columns<'a> { pub skip: usize, pub funcs: &'a str, pub sources: &'a str,
                         pub lines: &'a str, pub args: Option<Args<'a>> }

impl<'a> Columns<'a> {
    pub fn of(words: &'a [String]) -> Result<Self, Failure>;
    pub fn frames(&self) -> Result<Stack, Failure>;
}
```

A walk is **one value, not a head and a tail.** Which frame is the call site is
`at()`, and a `Stack` cannot be empty: `Stack::of` is the one place that can
say so, and `Columns::frames` turns that into a `Failure` where the message is
read. Nothing downstream carries the question.

Three indices are undone, and all three are arithmetic:

**`skip`** drops the instrument's own frames. It is at least 1 and never the
whole walk — what is left is where the subject is.

**The line shift.** `BASH_LINENO[i]` is where frame `i` *was called from*, so
where frame `i` is *executing* is `BASH_LINENO[i - 1]`. Because `skip >= 1`,
that index is in range for every reported frame — the off-by-one is
unrepresentable rather than guarded.

**The argument stack.** `BASH_ARGC[i]` is the width of group `i`; its offset is
the sum of the widths before it, and its contents are reversed within the
group. Summing forward and reading each group backward gives the arguments in
the order the call was written.

### When arguments are absent

`BASH_ARGC` aligns 1:1 with `FUNCNAME` only where the shell was recording. Turn
`extdebug` on part-way and it is **short**, and short means every width belongs
to a different frame.

**Alignment is the test, not `shopt -q`,** and an unaligned record is carried as
absent. That is why `Frame::args` is an `Option`: `None` is "not recorded" and
`Some([])` is "called with none". A tool that never wants arguments omits the
two sections entirely and gets the same `None`.

A record that *does* line up but claims more arguments than were sent is a
corrupt one, and fails the run.

## Why the columns rather than rows

An instrument could assemble whole frames in bash and ship them as an array of
arrays. That is one more level of `@Q`, which re-escapes every quote, and a
walk over `BASH_ARGV` written in bash. Measured at depth 8 with three arguments
per frame, 4000 iterations, against an empty-loop floor of 2.7 µs:

| | µs/op | payload bytes |
|---|---:|---:|
| assembling rows, with the argument walk | 201 | 522 |
| six raw `${arr[*]@Q}` expansions | 21 | 314 |

The columns are also closer to what bash keeps: `BASH_ARGC` plus `BASH_ARGV` is
a width-prefixed flat word stream, which is `LinkedArr`'s own shape. Shipping
them as they are means the index arithmetic lands where the compiler can see
it, and where it is checked without running bash.

## Who uses it

| | skip | arguments |
|---|---|---|
| `BASHCAP` (`src/bashcap/`) | 2 | under `--trace-calls` |
| `BASHPROF_TIME_CPS` (`tests/examples/bashprof.rs`) | 2 | whatever the shell has |

Both reach it through `bash::STACK`, prepended to their own bash in
`Startup::bash`.

## See also

- [values.md](values.md) — `parse_array`, the shape each section is
- [bashcap.md](bashcap.md) — `extdebug`, and what else a snapshot carries
- [scoping.md](scoping.md) — why the nameref rather than a global
- [measurements.md](measurements.md) — what a snapshot costs

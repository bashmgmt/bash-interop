# Shipping a script whose call sites outlive the tool

`assets/bashcap.bash`, `assets/bashprof.bash`

An instrumented call site is source, not a debugging insertion:
`BASHPROF_TIME_CPS build make all` names a phase and is committed. Without the
tool that word does not exist and the script exits 127, so a script that ships
carries the word with it.

## One file, two deliveries

Each tool's words live in one file, and that file is both what the tool injects
and what a client vendors. What exists twice is only the **effect**: the words
call a hook, and a hook is either the tool's or an empty definition the client
installs.

```
assets/bashcap.bash       BASHCAP, WITH_BASHCAP, __bc_take_flags  → __bc_capture
src/bashcap/effect.bash   __bc_capture

assets/bashprof.bash      BASHPROF_TIME_CPS                       → __bp_begin, __bp_end
src/bashprof/effect.bash  __bp_begin, __bp_end
```

`assets/joining.bash` is the one file with no second delivery: `BC_JOIN` and
`BC_LEAVE` run before there is a session to inject anything into, so a client
vendors them and nothing injects them. It has no guard either — a script that
calls `BC_JOIN` wants a session, and the tool being absent is a missing command
rather than a call site to neutralise. See
[rig.md](rig.md#the-coprocess-convention).

The client sources the words unconditionally and guards the hook:

```bash
source lib/bashprof.bash
declare -F __bp_begin >/dev/null || { __bp_begin() { :; }; __bp_end() { :; }; }
```

```bash
source lib/bashcap.bash
declare -F __bc_capture >/dev/null || __bc_capture() { :; }
```

`>/dev/null` is not optional: `declare -F NAME` prints the name and `declare -f
NAME` prints the entire body.

## Why the guard names the hook

The tool defines everything through `BASH_ENV`, which bash sources **before the
script's first line**, in every shell. The client's `source` therefore always
comes second — and it redefines the words with the same bytes, which changes
nothing. The guard sits on the half that differs, so a client cannot displace
the real effect whichever way round the two arrive.

A bash function definition is global wherever it executes, so the guard may sit
inside a function of the client's own.

## The rule that makes a file shippable both ways

**A words file names nothing that only exists once the protocol has been
sourced** — no `BC_INSTR`, no `__BC_BAIL`, no `__BC_THROW`, no `__bc_stack`.
The list is `bash::INJECTED_NAMES`, and each tool's vendoring test asserts its
words file against it.

Two consequences follow from putting the work in a hook rather than inline:

- **The walk is one frame deeper.** `__bc_stack`'s `$2` counts the leading
  frames that belong to the instrument, and a hook is one of them. `BASHCAP`
  passes 3, `WITH_BASHCAP` passes 2 — its own frame is the call site.
- **`__BC_BAIL` returns from the hook**, since it is `return $?` and acts in
  the frame it expands in. The word writes `|| return $?` at the call, which
  reads the same with an empty hook.

What a hook takes for itself, bash gives back when it returns — `local IFS=' '`
for the `[*]@Q` joins restores the subject's own, including one that was unset.
A hook that returns before the measured call also stands in nobody else's walk;
both are in [measurements.md](measurements.md#what-a-callees-frame-gives-back).

## What a word owes its call site

The same reading with the tool and without it, which is more than a
pass-through:

| | |
|---|---|
| `WITH_BASHCAP` | consumes the same leading `-BCV:`/`-BCS:` flags before the continuation, or they are run as a command |
| `BASHPROF_TIME_CPS` | returns 125 when called without a label — without it a call with no arguments shifts nothing, runs nothing and reports success |

One file means one definition of each, so there is nothing to keep in step.
`__bc_take_flags` is the single parser both bashcap words use, and the shift
width is derived from it: every consumed word lands in one of two arrays, so
their combined length is how far to shift.

Both directions are covered where they can fail. `src/bashcap/tests/vendoring.rs`
and `src/bashprof/tests/vendoring.rs` run a vendored script with no tool at
all, and the same script under the tool where the guard has to leave the real
hooks standing. Each test writes the file the tool injects, byte for byte, and
each writes `set -euo pipefail` at the top of it — the shape a shipped script
has, and the option that reaches furthest into a tool. What that costs the
instrument is in [scoping.md](scoping.md#when-the-walk-finds-nothing-set--u).

## Which bash a client needs

The two halves have different floors, and the vendored one is the only floor a
client can be held to. From the bash changelog rather than measured here — this
tree has only 5.3.9 on it.

| | needs | for |
|---|---|---|
| injected (`prelude.bash`, `stack.bash`, the effects) | **5.0** | `$EPOCHREALTIME`, which every message is stamped with |
| vendored (`bashcap.bash`, `bashprof.bash`) | **4.4** | `"$@"` with no positional parameters under `set -u`, and `${x@Q}` where an effect is present |
| vendored (`joining.bash`) | 4.1 | `coproc`, `exec {fd}>&-` |

So a client on bash 4.4 runs its own scripts with the words in place and the
empty hooks behind them; putting a tool behind those hooks needs bash 5. Nothing
checks either at run time — a shell that is too old says so at the first line
that needs the feature, which is what a version guard would have said anyway.

## Where the files are

Under `assets/`, because they are shipped to a client. Every one is
`include_str!`d into the instrument as well, which is what makes a client's
copy and the injected one the same bytes.
`__fixtures/bashcap_demo/bashcap.bash` is a vendored copy that `make
bashcap-demo` diffs against the asset.

## See also

- [bashcap.md](bashcap.md#the-clients-side) — the words a client calls
- [bashprof.md](bashprof.md#the-tool) — the same, for measurements
- [scoping.md](scoping.md) — what `BASH_ENV` reaches, and when

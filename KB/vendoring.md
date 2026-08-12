# Shipping a script whose call sites outlive the tool

`assets/bashcap_polyfill.bash`, `assets/bashprof_polyfill.bash`

An instrumented call site is source, not a debugging insertion: `BASHPROF_TIME_CPS build make all` names a phase and is committed. Without the tool that word does not exist and the script exits 127, so a script that ships needs a pass-through definition of its own.

Installing that definition is **the client's decision**, and the client makes it:

```bash
source lib/bashprof_polyfill.bash
declare -F BASHPROF_TIME_CPS >/dev/null || __define_bashprof_polyfill
```

```bash
source lib/bashcap_polyfill.bash
declare -F BASHCAP >/dev/null || __define_bashcap_polyfill
```

## Why the assets define a definer

Sourcing either file installs nothing — it defines one function whose body holds the stubs. Sourcing is therefore free of effect on the shell's semantics and can sit with the client's other unconditional `source` lines, while the decision is taken separately, where the client wants it.

A bash function definition is global wherever it executes, so the guard may sit inside a function of the client's own.

## Why the guard exists

The tool defines the real word through `BASH_ENV`, which bash sources **before the script's first line**, in every shell. A stub installed unconditionally therefore overwrites the real definition, and the run reports nothing measured with no error anywhere. `declare -F` is the whole guard, and `>/dev/null` is not optional: `declare -F NAME` prints the name and `declare -f NAME` prints the entire body.

## What a stub owes the call site

The same reading with the tool and without it, which is more than a pass-through:

| | |
|---|---|
| `WITH_BASHCAP` | consumes the same leading `-BCV:`/`-BCS:` flags before the continuation, or they are run as a command |
| `BASHPROF_TIME_CPS` | returns 125 when called without a label — the status the real word returns, and without it a call with no arguments shifts nothing, runs nothing and reports success |

Both are covered where they can fail: `src/bashcap/tests.rs` runs a vendored fixture with no tool at all, and `tests/cli.rs` runs one script both ways — unprofiled, and under bashprof where the guard has to leave the real word standing.

## Where the files are

Under `assets/`, not `src/`, because nothing in the crate injects or exports them; every `.bash` under `src/` is `include_str!`d into the instrument. The tests reach the assets by `include_str!` so they cannot rot, and `__fixtures/bashcap_demo/polyfill.bash` is a vendored copy that `make bashcap-demo` diffs against the asset.

## See also

- [bashcap.md](bashcap.md#the-clients-side) — the words a client calls
- [bashprof.md](bashprof.md#the-tool) — the same, for measurements
- [scoping.md](scoping.md) — what `BASH_ENV` reaches, and when

# bash-interop

Run a bash program, hear every shell in its process tree, and answer the
questions those shells ask — while the program behaves exactly as it does when
nothing is listening.

The usual ways of watching a shell script are indirect. `set -x` produces a
trace that no two people parse the same way, exported variables only travel
downward, and wrapping the interpreter tells you nothing about what happened
inside a function. This crate takes the other route. The script states what it
wants observed, and a Rust program on the other end of a pipe hears it and can
reply.

## What a script does

Once a script has joined a session, it can do two things.

`say` ships a list of words and returns immediately, because nothing is waiting
on it:

```bash
BC_INSTR BUILD say STARTED phase compile
```

`ask` ships a list of words and blocks until your Rust code replies:

```bash
BC_INSTR BUILD ask cache-lookup "$sha"
```

The words are yours in both cases. There is no schema and no reserved
vocabulary — the wire moves an argument list of any width, and the protocol
reads none of its positions.

## What an answer is

Understand this part first. Everything your Rust code can do to a running
shell follows from it.

When your reaction answers an `ask`, it returns a list of words. Those words
travel back to the shell that asked, which parses them with bash's own array
syntax and then runs them as a command, in the frame that asked:

```bash
local -a __bc_answer="$__bc_line"    # the reply, read as a bash array literal
"${__bc_answer[@]}"                  # and invoked, right here
```

There is no `eval` anywhere in that. Bash parses an array literal — the same
notation `declare -p` prints — and calls the result.

Handing back a command rather than a value is what makes the channel general.
One command, run in the frame that asked, already covers the things you would
otherwise design a protocol around:

| your reaction returns | the asking shell does |
|---|---|
| `Answer::of("echo", [path])` | prints it, so `x=$(BC_INSTR … ask …)` captures a value |
| `Answer::of("declare", ["-g", "target=staging"])` | sets a variable in its own process |
| `Answer::status(3)` | `return 3`, so `if BC_INSTR … ask …` branches on the reply |
| `Answer::of("source", [path])` | runs a file of any length that you just wrote |
| `Answer::of("exit", ["9"])` | ends the subject |

The `ask` exits with the status of whatever ran, so a reply that says no is an
ordinary shell failure the script can test.

The vocabulary is not limited to builtins either. A rig injects bash of its own
into every shell that joins, so you can define a helper there and answer by
calling it:

```rust
// the rig's bash, sourced by every shell in the tree
fn bash(&self, _at: &Layout) -> String {
    "use_toolchain() { export CC=$1 CXX=$2; hash -r; }\n".to_string()
}

// ... and later, deciding what an ask gets back
Ok(Answer::of("use_toolchain", ["clang", "clang++"]))
```

Your Rust program decides; a function you wrote carries the decision out inside
the shell that asked.

## The Rust side

You implement two traits. `Rig` describes the arrangement — what bash to inject
and how to build a counterpart for each shell that turns up. `Reacting` is that
counterpart:

```rust
impl Reacting for Watching {
    type Kept = Vec<Message>;

    async fn hear(&mut self, said: Message) -> Result<(), Failure>;
    async fn answer(&mut self, asked: Message) -> Result<Answer, Failure>;
    async fn finish(self) -> Result<Self::Kept, Failure>;
}
```

Every bash process that joins gets its own pipe, its own task and its own
`Reacting` value, so a subshell and its parent never get confused for each
other, and a slow reply holds up only the shell waiting on it. The runtime is
single-threaded, which is why nothing here asks you for a `Send` bound.

Two things matter before you put a call site into code you ship.

A call site is a real dependency, and a loud one on purpose. Run the script
with no session anywhere and `BC_INSTR` is simply a command that does not
exist: status 127, and nothing else happens. Load the prelude but join no
session and it reports `label … is not joined` at your call site and returns
125. Neither case fails quietly, because a script that asked to be observed
and silently was not is the worse outcome. If a script has to run both ways,
say so in one line at the top:

```bash
declare -F BC_INSTR >/dev/null || BC_INSTR() { :; }
```

Second, nothing is timed, counted or inferred on the bash side. A message
carries the sending shell's own clock and its own view of the call stack, so
what your program reports is what a shell said about itself at the moment it
spoke, rather than something reconstructed afterwards from ordering.

## Reading on

[The overview](https://bashmgmt.github.io/bash-interop/overview.html) walks the
whole model once. The module documentation for `rig` carries a worked example
that compiles, and [the book](https://bashmgmt.github.io/bash-interop/) is the
reference: the wire and its message forms, what a shell reports about itself,
the frame walk, and the measurements the transport rests on.

Built on [bash-strings](https://github.com/bashmgmt/bash-strings).
[bashcap](https://github.com/bashmgmt/bashcap) and
[bashprof](https://github.com/bashmgmt/bashprof) are small rigs over this
crate, and read as examples of one.

Licensed under the MIT licence.

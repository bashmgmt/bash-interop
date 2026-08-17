# bash-interop

Run a bash program, hear every shell in its process tree, and answer the
questions those shells ask.

A script joins a session and speaks:

```bash
BC_JOIN MYTOOL "$session"
BC_INSTR MYTOOL say STARTED phase build
answer=$(BC_INSTR MYTOOL ask WHERE-IS libc)
```

On the Rust side you write a rig: the bash to inject, and what to do with
each shell that appears.

```rust
impl Rig for MyTool {
    type Reaction = MyReaction;
    fn bash(&self, at: &Layout) -> String { ... }
    async fn joined(&self, at: &Layout, shell: Arc<Shell>)
        -> Result<Self::Reaction, Failure> { ... }
}
```

Every shell in the tree gets its own pipe, its own task and its own reaction,
so a subshell and its parent stay distinct. `say` is one-way. `ask` blocks the
shell until the rig answers, which is how a script queries something the Rust
side knows. The runtime is single-threaded.

Two properties are worth knowing before you read further.

The words are shell functions that do nothing when no session is present. A
script carrying them behaves the same run without a tool as it does under one,
so they can be committed in place.

Nothing is timed, counted or inferred in bash. A message carries the sending
shell's clock and its frame walk, so what the Rust side reports is what a
shell said about itself at the moment it spoke.

## Reading order

[`docs/overview.md`](docs/overview.md) covers the shape of a session. The
module documentation for `rig` carries a worked example that compiles.
[`docs/`](docs/README.md) is the reference: the wire and its message forms,
what a shell reports about itself, the stack walk, and the measurements the
transport rests on.

Built on [bash-strings](https://github.com/bashmgmt/bash-strings).
[bashcap](https://github.com/bashmgmt/bashcap) and
[bashprof](https://github.com/bashmgmt/bashprof) are small rigs over this
crate, and read as examples of one.

Licensed under the MIT licence.

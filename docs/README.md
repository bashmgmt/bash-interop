# bash-interop — the book

Run bash under instrumentation and hear what it says: a session per run, a
pipe and a task per shell, words a script speaks and answers it runs. This
book is the reference for the current design — what the pieces are, how the
two session setups work, and where each responsibility lies.

| chapter | what it covers |
|---|---|
| [overview.md](overview.md) | the model in one pass, and the vocabulary |
| [design.md](design.md) | the design decisions the shape follows from |
| [rigs.md](rigs.md) | `Rig`, `Reacting`, `Layout`, `Provision` — the API |
| [driving.md](driving.md) | Rust orchestrates: `run`, `run_at`, the environment closure |
| [serving.md](serving.md) | bash orchestrates: `serve`, the coprocess convention |
| [joining.md](joining.md) | every way a shell joins, and who initiates |
| [wire.md](wire.md) | the protocol: files, fifos, frames, messages |
| [shell.md](shell.md) | a shell's account of itself |
| [stack.md](stack.md) | the frame walk, both halves |
| [scoping.md](scoping.md) | where names bind in the shipped bash |
| [vendoring.md](vendoring.md) | what a client vendors, and why the copies cannot drift |
| [measurements.md](measurements.md) | the kernel and bash facts the transport stands on |

Code quoted from the tree is anchored: a fenced block preceded by an HTML
comment declaring `quote: <file> anchor=<name>` is the region between
`ANCHOR: <name>` and `ANCHOR_END: <name>` in that file, kept identical by
[`sync-quotes.bash`](sync-quotes.bash) — run it after changing an anchored
region; CI runs it with `--check`.

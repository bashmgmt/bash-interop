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
| [measurements.md](measurements.md) | the kernel and bash facts the transport stands on |

Code blocks are hand copies of the tree, named by the file they copy —
when touching either side, check the other. The complete client scripts
also live as fixtures in `bashprof/__fixtures/book/`, where that crate's
cli suite runs them as printed.

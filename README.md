# bash-interop

Run a bash program, hear every shell in its process tree, and answer the
questions those shells ask — without changing how the program behaves when
nothing is listening. A script says `BC_INSTR LABEL say|ask …`; a Rust rig
hears it, one reaction per shell, one pipe and one task each, on a
single-threaded runtime.

Start with the book — [`docs/overview.md`](docs/overview.md) — then the
crate doc (`cargo doc --open`): `rig`'s module doc carries a complete worked
example. [`docs/`](docs/README.md) holds the full reference: the wire, the
shell's account, the stack walk, the measured facts the transport stands on.

Built on [`bash-strings`](../bash-strings) (bash's own quoted forms). The
shipped tools `bashcap` and `bashprof` are thin users of this crate.

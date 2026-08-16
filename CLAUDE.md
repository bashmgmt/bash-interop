# bash-interop — working in this crate

The instrumentation core: `failure` (one error), `shell` (a shell's account of
itself), `stack` (the frame walk, both halves), `rig` (session, wire,
prelude.bash, the Driving/Serving roles, `JOINING`), `scratch` (public test
material).

**docs/ is the book** — `docs/README.md` is the contents page,
`overview.md` the way in, `measurements.md` holds the kernel and bash facts
the transport stands on: check there before proposing a transport change;
`design.md` is the design above the code. Do not restate any of it in code
comments. Code blocks in the book are hand copies of the tree — when
touching either side, check the other.

```bash
cargo test --lib -- --test-threads=1
cargo test --test proofs -- --test-threads=1
cargo clippy --all-targets -- -D warnings   # silent, and stays silent
cargo doc --no-deps --document-private-items
```

A corner case belongs beside the thing it covers, in `src/<module>/tests/`;
`tests/proofs/` are bash-level proofs over the public surface only. Style
follows the parent workspace's CLAUDE.md: comments carry technical fact, never
narrative; non-defensive; one way to do a thing; prefer deleting.

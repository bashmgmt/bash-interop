# bash-interop — working in this crate

The instrumentation core: `failure` (one error), `shell` (a shell's account of
itself), `stack` (the frame walk, both halves), `rig` (session, wire,
prelude.bash, the Driving/Serving roles, `JOINING`), `scratch` (public test
material). `assets/joining.bash` is the vendorable client half, exported as
`rig::JOINING_BASH`.

**KB/ is the knowledge base** — `onboarding.md` to start, one document per
module, `measurements.md` holds the kernel and bash facts the transport stands
on: check there before proposing a transport change; `architecture.md` is the
design above the code. Do not restate either in code comments.

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

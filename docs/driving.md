# Driving — Rust orchestrates

```rust
pub trait Driving: Rig {
    /// A workspace of the run's own, gone when the run ends.
    async fn run<A, E>(&self, argv: &[A], environment: E) -> Result<Run<Kept<Self>>, Failure>
    where A: AsRef<OsStr>, E: FnOnce(&Layout) -> Result<Vec<(OsString, OsString)>, Failure>;

    /// The caller's directory instead — it exists, and is the caller's to
    /// have made — left behind: a reading taken later may follow source
    /// paths into it.
    async fn run_at<A, E>(&self, at: &Path, argv: &[A], environment: E) -> Result<Run<Kept<Self>>, Failure>
    where A: AsRef<OsStr>, E: FnOnce(&Layout) -> Result<Vec<(OsString, OsString)>, Failure>;
}
// the role opt-in is an empty impl block; the closure's return is the
// subject's WHOLE environment delta — the core exports nothing

pub struct Run<K> { pub shells: Vec<Attended<K>>, pub subject: ExitStatus, pub failed: Option<Failure> }
pub struct Whole<K> { pub shells: Vec<Attended<K>>, pub subject: ExitStatus }   // Run::whole()

```

**The command line is run as it is given, and carries its own program.**
`rig.run(&["bash", "x.bash"], |at| …)`, and a caller wanting a launcher
writes one into the argv: `&["env", "TARGET=staging", "bash", "x.bash"]`. The
subject's environment is exactly the closure's return, built from the
settled `Layout` — fallible, because provisioning writes a file:
`Ok(vec![at.bash_env(Provision::Joining(&rig.joining(at)))?])` is the usual
sentence, the join of every non-interactive bash in the tree;
`Provision::Definitions` beside a `("BC_SESSION", at.text())` pair of the
caller's own spelling gives every shell the words and leaves initiation to
the scripts (the tools' by-hand convention); `Ok(vec![])` and any variable
of the caller's own are equally legitimate.
`tests/proofs/starting.rs` proves the strong side: a variable the closure did
not return is absent.

**Reaching a `Run` means bash got to its own end**, so `subject` is always the
subject's own status. `failed` is what went wrong closing up. A `Failure` in
place of a `Run` means the run never got that far.

The run spawns the subject with `process_group(0)`, watches its pidfd, and when
it fires **kills the group and then reaps** — in that order, because while the
subject is unreaped its group cannot have been recycled. Only then does the
session close, so every task reads what its shell wrote up to the kill and
sees end of input. `Drop` does the same if `run` left by any other path.


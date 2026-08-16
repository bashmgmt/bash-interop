# Driving — Rust orchestrates

The driven role is for when your Rust program is in charge: it has a rig, it
has a command line to run — a build, a test suite, one script — and it wants
the whole thing carried out under instrumentation and the results back as
values. The subject is started *by the run*, owned by it, and seen out by
it.

Using it is one call:

```rust
let ran = Deploying
    .run(&["bash", "deploy.bash"], |at| {
        Ok(vec![at.bash_env(Provision::Joining(&Deploying.joining(at)))?])
    })
    .await?;
```

The two entry points (abridged — rustdoc is authoritative):

```rust
pub trait Driving: Rig {
    /// A workspace of the run's own, gone when the run ends.
    async fn run(&self, argv, environment) -> Result<Run<Kept<Self>>, Failure>;

    /// The caller's directory instead — it exists, and is the caller's to
    /// have made — left behind: a reading taken later may follow source
    /// paths into it.
    async fn run_at(&self, at: &Path, argv, environment) -> Result<Run<Kept<Self>>, Failure>;
}
```

They differ only in where the workspace lives. `run` opens a temporary
directory that vanishes with the run; `run_at` uses a directory you made —
and *must* have made, missing is a refusal — and leaves it behind, which
matters when a reading taken later wants to follow source paths into it.
Opting a rig in is an empty impl block: `impl Driving for Deploying {}`.
The orchestration is entirely provided.

## The command line is yours, verbatim

`argv` is run exactly as given and carries its own program:
`&["bash", "deploy.bash"]`, `&["make", "test"]`. There is no hidden shell
and no argument rewriting, so anything you would type gets written as it is
— a launcher included: `&["env", "TARGET=staging", "bash", "deploy.bash"]`.

## The environment closure — the run states the subject's world

The second argument answers a question most tools answer for you, silently:
*what does the subject's environment get?* Here the core adds **nothing** —
no exported variable, no blessed pathway — and your closure's return is the
subject's whole environment delta. It receives the settled `Layout`
(workspace made, files laid) and is fallible, because provisioning writes a
file.

The three usual sentences, each a complete answer:

```rust
// Blanket: provision a joining startup file. Every non-interactive bash
// in the subject's tree joins as it starts — the right default for
// subjects that know nothing of the session.
|at| Ok(vec![at.bash_env(Provision::Joining(&rig.joining(at)))?])
```

```rust
// Chosen: provision definitions only, and hand the coordinate to the
// scripts under a name of YOUR convention — they initiate where they say.
// (bashprof spells this BASHPROF_SESSION, bashcap BASHCAP_SESSION.)
|at| Ok(vec![
    at.bash_env(Provision::Definitions)?,
    ("DEPLOY_SESSION".into(), at.text().into()),
])
```

```rust
// Nothing: the subject runs with no additions at all. Shells can still
// join by hand if some script knows the workspace by other means.
|at| Ok(vec![])
```

Any further variables of your own ride along in the same vector. The strong
side of "the core adds nothing" is proven, not promised:
`tests/proofs/starting.rs` starts a subject and shows a variable the
closure did not return is absent from it.

## What comes back

```rust
pub struct Run<K>   { pub shells: Vec<Attended<K>>, pub subject: ExitStatus, pub failed: Option<Failure> }
pub struct Whole<K> { pub shells: Vec<Attended<K>>, pub subject: ExitStatus }   // via Run::whole()
```

Reaching a `Run` at all means bash was started and seen out — `subject` is
always the subject's own exit status, untouched. `failed` is anything that
went wrong *closing up* (a half-written line found at the end, a reaction
that would not finish). The two are deliberately separate facts: a subject
may exit 0 while the reading is damaged, or exit 9 while the reading is
fine. `Run::whole()` is the discharge point — it returns `Whole`, with
`failed` converted into an `Err` — so code that only wants a clean run
writes `.run(…).await?.whole()?` and holds a type that *cannot* carry an
undischarged problem. A `Failure` in place of a `Run` means the run never
got that far (the workspace was occupied, the spawn failed).

## The subject's life, exactly

The run spawns the subject with `process_group(0)` — a group of its own —
and watches its **pidfd**, never installing a signal handler. When the
subject exits, the watch fires and the run **kills the group, then reaps**,
in that order: while the subject is unreaped, its group id cannot have been
recycled, so the kill can never hit a stranger. The group kill is what ends
stragglers — a background process the subject left behind would otherwise
hold pipes open forever. Only after that does the session close, so every
task reads what its shell wrote up to the kill and sees a clean end of
input. If `run` leaves by any other path — a reaction's `Failure`, a panic
unwinding — `Drop` on the subject does the same kill-and-reap; there is no
exit that leaks the group.

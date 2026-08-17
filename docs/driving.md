# Driving

Under the driven role your Rust program is in charge. It has a rig and a
command line to run — a build, a test suite, one script — and wants the whole
thing carried out under instrumentation with the results back as values. The
run starts the subject, owns it, and sees it out.

Using it is one call:

```rust
let ran = Deploying
    .run(&["bash", "deploy.bash"], |at| {
        Ok(vec![at.bash_env(Provision::Joining(&deploy_join(at)))?])
    })
    .await?;
```

The two entry points, abridged; rustdoc is authoritative:

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

They differ in where the workspace lives. `run` opens a temporary directory
that vanishes with the run. `run_at` uses a directory you made, refusing when
it is missing, and leaves it behind, which matters when a reading taken later
follows source paths into it. Opting a rig in is an empty impl block, `impl
Driving for Deploying {}`, since the orchestration is provided.

## The command line

`argv` runs exactly as given and carries its own program: `&["bash",
"deploy.bash"]`, `&["make", "test"]`. There is no hidden shell and no argument
rewriting, so a launcher goes in the same way anything else does — `&["env",
"TARGET=staging", "bash", "deploy.bash"]`.

## The environment closure

The second argument settles what the subject's environment gets. The core adds
nothing on its own, and the closure's return is the subject's whole
environment delta. It receives the settled `Layout`, with the workspace made
and the files laid, and it is fallible, because provisioning writes a file.

Three closures cover the usual cases.

<!-- quote: tests/book.rs anchor=env-joining -->
```rust
// Blanket: provision a joining startup file. Every non-interactive
// bash in the subject's tree joins as it starts — the right default
// for subjects that know nothing of the session. The line is the
// wrapper's own statement (rigs.md: the sketch's deploy_join).
|at| {
    Ok(vec![at.bash_env(
        Provision::Joining(&deploy_join(at)),
    )?])
},
```

<!-- quote: tests/book.rs anchor=env-definitions -->
```rust
// Chosen: provision definitions only, and hand the coordinate to
// the scripts under a name of YOUR convention — they initiate where
// they say. (bashprof spells this BASHPROF_SESSION, bashcap
// BASHCAP_SESSION.)
|at| {
    Ok(vec![
        at.bash_env(Provision::Definitions)?,
        (
            "DEPLOY_SESSION".into(),
            at.text().into(),
        ),
    ])
},
```

<!-- quote: tests/book.rs anchor=env-nothing -->
```rust
// Nothing: the subject runs with no additions at all. Shells can
// still join by hand if some script knows the workspace by other
// means.
|_at| Ok(vec![]),
```

Further variables of your own ride along in the same vector.
`tests/proofs/starting.rs` starts a subject and shows that a variable the
closure did not return is absent from it.

## What comes back

```rust
pub struct Run<K>   { pub shells: Vec<Attended<K>>, pub subject: ExitStatus, pub failed: Option<Failure> }
pub struct Whole<K> { pub shells: Vec<Attended<K>>, pub subject: ExitStatus }   // via Run::whole()
```

Reaching a `Run` means bash was started and seen out, and `subject` is the
subject's own exit status, untouched. `failed` holds anything that went wrong
while closing up, such as a half-written line found at the end or a reaction
that would not finish. They are separate facts: a subject may exit 0 with a
damaged reading, or exit 9 with a sound one.

`Run::whole()` is the discharge point. It returns `Whole`, with `failed`
converted into an `Err`, so code that wants only a clean run writes
`.run(…).await?.whole()?` and holds a type that cannot carry an undischarged
problem. A `Failure` in place of a `Run` means the run never got that far, as
when the workspace was occupied or the spawn failed.

## The subject's life

The run spawns the subject with `process_group(0)`, giving it a group of its
own, and watches its pidfd without installing a signal handler.

When the subject exits, the watch fires and the run kills the group and then
reaps, in that order. While the subject is unreaped its group id cannot have
been recycled, so the kill cannot reach a stranger. The group kill is what
ends stragglers, since a background process the subject left behind would
otherwise hold pipes open indefinitely.

Only after that does the session close, so every task reads what its shell
wrote up to the kill and sees a clean end of input. If `run` leaves by another
path, whether a reaction's `Failure` or a panic unwinding, `Drop` on the
subject does the same kill and reap.

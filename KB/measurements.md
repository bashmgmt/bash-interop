# Measurements and limits

Numbers measured on this machine, the bash constraints that bound the design,
and what each proof establishes.

## The `PIPE_BUF` boundary

Eight shells writing to one FIFO, one `printf` per line, 160 lines expected:

| bytes written per line | intact | corrupt |
|---:|---:|---:|
| 3901 | 160 | 0 |
| 4096 | 160 | 0 |
| 4097 | 114 | **46** |
| 9001 | 123 | **37** |

`PIPE_BUF` is 4096, and the boundary is sharp: at or below it a write lands
whole; one byte past it, concurrent writers interleave. `__BC__limit` is 3900,
leaving room for the ~37-byte header and the delimiter.

## Cost in bash

Per message, inside bash, minimum of seven runs of 4000:

| | µs |
|---|---|
| build the message, no I/O | 13.8 |
| write it to the pipe | ~15.5 |
| sending, inlined at every call site | 21 |
| sending through one bash function | 28 |

Message assembly dominates either way, and a tool reading real state costs
far more: a full `bashcap` snapshot is ~480 µs, so the 7 µs difference between
an inlined send and a function call is under two percent of what an
instrumented call site actually costs.

Joining — the first `say` in a new shell, which opens the pipe — costs 10 µs
and sends nothing of its own. Provenance rides on the messages the shell goes
on to write, as two more `printf` arguments.

### The one-frame lane

`say` is two bash function calls and returns after one write. Over 4000 of
them, against a floor of 1.8 µs for the bare loop, that measures **32.9
µs/op**; inlining `__bc_send` into the `say` arm as well measures 31.5, and
the remaining 1.4 µs costs a second copy of packing and shipping, which `ask`
and `join` still need.

## The frame walk

Assembling whole frames in bash, against shipping bash's five stack arrays as
they are. Depth 8, three arguments per frame, 4000 iterations, empty-loop floor
2.7 µs:

| | µs/op | payload bytes |
|---|---:|---:|
| rows, with the argument walk in bash | 201 | 522 |
| six raw `${arr[*]@Q}` expansions | 21 | 314 |

The bytes are the second `@Q`: nesting re-escapes every quote, and the payload
counts against `__BC__limit`. See [stack.md](stack.md).

## Cost of a snapshot

`bashcap run` over 2000 `BASHCAP` calls at a six-deep stack, wall clock per
snapshot — the whole path, bash through the wire to the decoded JSON:

| | untraced | `--trace-calls` |
|---|---:|---:|
| the walk assembled in bash | 572 µs | 737 µs |
| the walk shipped as columns | 482 µs | 527 µs |

Arguments used to cost 165 µs a snapshot and now cost 45: what is left is the
two extra expansions and the wider payload, rather than a loop.

## Cost end to end

300 operations each:

| | polling at 200 µs | on `poll` |
|---|---|---|
| `say` | 85 µs | 56 µs |
| `ask` | 333 µs | 98 µs |

Against ~28 µs of bash work, an ask under the polling loop spent the
remainder waiting on the operator's own timer. The loop polled because it had
to notice the child exiting as well as read the pipe; a `pidfd` makes the exit
a readable descriptor, so one `poll` covers both and the interval disappears.

## Memory

`BashCap` decodes and writes in `hear`, so a snapshot reaches the file as it
arrives. Resident memory does not track the run:

| snapshots | peak RSS | output |
|---:|---:|---:|
| 200 | 7.7 MB | 0.19 MB |
| 2 000 | 7.8 MB | 1.9 MB |
| 20 000 | 7.5 MB | 18.9 MB |

## What the proofs establish

`tests/proofs/`, over the public API only. Each spawns real bash to cover one
mechanism that cannot be checked by reading the source — everything that can
be is left to the compiler. One file per subject.

| `transport.rs` | establishes |
|---|---|
| `every_descendant_shell_reaches_the_wire` | subshells, command substitutions and child processes all reach the pipe; five shells, one root, three deep, and `SHLVL` never drops toward a descendant |
| `concurrent_writers_never_interleave` | 8 writers × 80 messages, half of them 9000 bytes, arrive whole |
| `nothing_is_lost_at_the_end` | 200 messages written immediately before exit are readable after the subject is gone |
| `a_newline_inside_a_value_is_escaped_not_framed` | a value containing `\n` arrives as one message, not two frames |

| `transparency.rs` | establishes |
|---|---|
| `a_signalled_subject_is_reported_and_loses_nothing` | `Signal(15)`, `.shell_code() == 143`, and what was said before the signal survives |
| `a_clients_own_trap_and_ifs_are_untouched` | a client's own `EXIT` trap and `IFS` survive a message going out |

| `answering.rs` | establishes |
|---|---|
| `a_session_survives_every_way_of_answering` | 57 asks across two shells, every answer form, one deliberately slow, mixed with a message too wide for one frame |

| `starting.rs` | establishes |
|---|---|
| `a_rig_may_add_to_the_environment_and_reach_every_shell` | `Startup::env` reaches the subject and a child it starts, and `Startup::bash` puts the rig's word in both |
| `the_command_line_is_run_as_asked` | the run starts the program the argv names, with nothing appended |

| `owning.rs` | establishes |
|---|---|
| `a_named_workspace_is_left_behind` | `run_in` leaves its prelude and its pipe where it was told to |
| `every_reply_pipe_goes_with_its_answer` | 43 asks from two shells leave no reply pipe behind |
| `a_workspace_belongs_to_one_run` | a second run in the same directory stops at the up pipe |
| `a_shell_left_asking_does_not_outlive_the_run` | the run does not wait for a straggler, and the straggler does not survive it |
| `a_panicking_answer_kills_the_subject` | the same guarantee reached by unwinding, naming the blocked pid |

| `malformed.rs` | establishes |
|---|---|
| `an_unfinished_message_is_reported_beside_the_subjects_status` | a message whose last chunk never comes is reported as `Run::failed`, the subject's own status survives it, and the messages around it arrive |
| `a_message_that_will_not_read_ends_the_run` | a chunk claiming another message's key corrupts it, and the reader refuses it rather than handing on nonsense |

| `failing.rs` | establishes |
|---|---|
| `a_rig_that_cannot_answer_ends_the_run_and_kills_the_subject` | `run` yields the rig's reason, and the shell blocked on the ask does not outlive it |
| `a_reply_pipe_name_already_taken_is_the_subjects_to_handle` | `mkfifo` is one attempt: a name something else left is refused at the subject's call site rather than adopted |
| `a_failure_while_hearing_ends_the_run_and_kills_the_subject` | the same for a message nobody was waiting on, without waiting out a subject that would have slept 30 seconds |
| `an_unknown_verb_is_reported_rather_than_ignored` | a verb the protocol does not define is named on stderr and returns 125 |

Bash-level invariants that hold without running anything are asserted against
the shipped text instead, and live beside it: the protocol's in
`src/bash/rig/wire/mod.rs`, each tool's in its own tests.

Every assertion reads messages as **words**, not as a joined string: word
boundaries are what the wire preserves, and comparing joined text would give
that away.

## Bash constraints that bound the design

**Traps do not compose.** Bash allows one handler per signal. Contributing an
`EXIT`/`ERR`/`DEBUG` fragment therefore means adopting whatever handler the
client already installed, which means capturing its text and replaying it —
`eval` — and a client that installs one *after* the prelude runs silently
replaces the result unless the `trap` builtin is shadowed too. This is why
provenance and exit are carried by messages and `wait()` rather than by a
handler.

**A subshell resets caught traps.** Anything buffered in a `( … )` and flushed
from `EXIT` is lost. Combined with `kill -9` and with a single unparsable line
poisoning a batch, this is why a message is written where it is produced
rather than accumulated: each of those costs one message instead of a run.

**`$?` must be read as a frame's first statement.** `local a=$1 b=${x[$a]}`
does not work: bash expands every right-hand side before assigning any of
them.

**A bash arithmetic *command* is false when its result is 0.** `(( at += n ))`
returns 1 whenever the running total is still 0, which under the subject's own
`set -e` ends the script part-way through a snapshot. `x=$(( x + n ))` has no
such status. This is why no instrument in the crate counts in bash.

**Under `extdebug`, a `DEBUG` handler returning non-zero skips the command it
fired for.** An instrument propagating `$?` faithfully would skip everything
after a failure; the handler must return 0, and bash restores `$?` for the
next command itself.

**A pipe write is atomic only up to `PIPE_BUF`** — see above. This is the one
constraint that shows through into the wire format, as the `+`/`.` marker.

# Measurements and limits

Numbers measured on this machine (Linux 6.x, bash 5.3.9), the bash and kernel
constraints that bound the design, and what each proof establishes.

## The kernel, on fifos

| | |
|---|---|
| a reader opens `O_RDONLY\|O_NONBLOCK`, **no writer has ever attached** | quiet — *not* `POLLHUP`, not for 300 ms |
| a writer attaches, no data | still quiet |
| a writer writes | `POLLIN` |
| a writer writes and exits | `POLLIN\|POLLHUP`, data intact |
| all writers gone, having attached | `POLLHUP` |
| a **non-blocking** reader open | unblocks a **blocking** writer open |
| parent exits, a subshell still holds the inherited fd | `POLLIN` only — `POLLHUP` waits for the subshell |
| a reader opens then closes | the blocked writer unblocks, and its next write takes `SIGPIPE` |

> **`POLLHUP` means "a writer attached and all writers are now gone".**

There is no ambiguity between *not yet* and *no longer*, and no state to keep
beside the pipe. This is why end of input on a shell's pipe is its goodbye, and
why the blocking `exec {fd}>up.$tok` is the rendezvous.

## tokio, on the same

Verified with a scratch crate on tokio 1.53, current-thread runtime:

| | |
|---|---|
| `pipe::OpenOptions::new().read_write(true).open_receiver(join)` | quiet with no writer; a writer that wrote and left leaves it **open** — no end of input |
| `pipe::OpenOptions::new().open_receiver(up)` — `O_RDONLY\|O_NONBLOCK` | quiet with no writer ever; a bash that attached, wrote three lines and exited yields the three lines then end of input |
| a bash blocked in `exec 9>up` | released by `open_receiver`, exactly when it was opened |
| `pipe::OpenOptions::new().open_sender(rep)` with no reader | `ENXIO` immediately |
| `Sender::write_all` of 100 KB to a bash `read` | completes; bash reads 100 000 bytes |
| `AsyncFd<pidfd>::readable()` | wakes when the process exits |
| `AsyncFd<read end>::readable()` when the writer closes | wakes, `is_read_closed` |

The whole descriptor layer is stock tokio and nothing is hand-rolled.

## What things cost from bash

| | µs |
|---|---|
| `( : )` — a subshell | 341 |
| `bash -c ':'` | 1471 |
| `bash -c ':'` with a 200-line `BASH_ENV` | 1884 |
| `exec {fd}>fifo` + close, a reader present | 8 |
| `printf` one message to a fifo | 12 |
| **`mkfifo` — this box's, which is uutils in Rust** | **2088** |
| `mkfifo` — GNU coreutils' (`/bin/true` measured 680) or busybox's | ~600 |
| a static 800 KB `mkfifo` — the floor: fork plus a bare exec | 514 |

**Bash has no builtin that makes a fifo.** `mkfifo`, `mknod`, `mkdir` and `ln`
are all external commands; the loadable `mkfifo` builtin is not shipped by
default anywhere; and every fork-free way to *wait* for a fifo the run would
make instead fails to one wall — a fifo gives one process a non-consuming wait
only through `open`, and a shared `open` cannot say which shell it releases.
So a shell that attaches forks once, and that is the one cost of a pipe per
shell: paid at source by every bash process under `BASH_ENV`, and by every
fork that speaks. Asks fork for nothing.

## The token

| | unique |
|---|---|
| `$BASHPID.${EPOCHREALTIME#*[.,]}` | 2000 / 2000 |
| the same plus `${SRANDOM:-$RANDOM$RANDOM}` | 2000 / 2000 |

over 2000 tokens from nested subshells, background forks and child processes.
One process's clock advances between two reads (measured 4 µs apart). `SRANDOM`
is 5.1+ and fresh per subshell; `RANDOM` is reseeded per subshell in 5.x and
inherited before 5.0. A duplicate token fails at `mkfifo` in the shell that
chose it, and Rust keys nothing on it.

## Loopback TCP, measured and rejected

`/dev/tcp/127.0.0.1/<port>` would remove every fifo, the fork and the
rendezvous: `printf` to it costs 13 µs against a fifo's 12, and a connect 46 µs.
But bash cannot set `TCP_NODELAY`, and a shell that writes twice and then asks
hits Nagle against the receiver's delayed ACK:

| write, write, ask, read | µs per round |
|---|---|
| over loopback TCP | **41 015** |
| over loopback TCP with the receiver re-arming `TCP_QUICKACK` on every read | 63 |
| over two fifos | 33 |

## Cost in bash, per message

Minimum of seven runs of 4000:

| | µs |
|---|---|
| build the message, no I/O | 13.8 |
| write it to the pipe | ~15.5 |
| sending, inlined at every call site | 21 |
| sending through one bash function | 28 |

Message assembly dominates either way, and a tool reading real state costs
far more: a full `bashcap` snapshot is ~480 µs. Nothing about the shell rides
on a message — its pid, `$SHLVL`, `$BASH_SUBSHELL` and version are in the
account, said once — and what is left in front of a client's arglist is the
verb and one `at=` clock.

## The frame walk

Assembling whole frames in bash, against shipping bash's five stack arrays as
they are. Depth 8, three arguments per frame, 4000 iterations, empty-loop floor
2.7 µs:

| | µs/op | payload bytes |
|---|---:|---:|
| rows, with the argument walk in bash | 201 | 522 |
| six raw `${arr[*]@Q}` expansions | 21 | 314 |

See [stack.md](stack.md).

## What a function layer costs an instrument

An instrument that separates its layers as **functions** puts every layer's
frame on the stack of everything measured below it, and every walk carries
them. `BASHPROF_TIMETHIS` as one function against the same word as a CPS spine
of three, BEGIN payload in bytes by how many measured calls enclose it:

| enclosing measurements | one function | spine of three |
|---:|---:|---:|
| 0 | 349 | 537 |
| 1 | 471 | 1112 |
| 2 | 584 | 1678 |
| 3 | 697 | 2244 |
| **per level** | **~113** | **~566** |

**What costs this is a layer that is still on the stack while the measured call
runs.** `__bp_begin` sends the BEGIN and returns before `"$@"`, so it stands in
its own walk and in nobody else's — ~77 bytes and one frame per level. Two
extra calls per measurement cost ~1.0 µs each.

## What a callee's frame gives back

**`local` restores what was there, including *unset*.** A callee taking
`local IFS=' '` leaves an unset `IFS` unset and an empty one empty; the
distinction a manual restore has to make by hand, bash makes itself.

**A command-prefix assignment scopes to the call**, restores the previous
state — unset included — and reaches expansions inside it, including through a
`local -n` nameref.

## Cost of a snapshot

`bashcap run` over 2000 `BASHCAP` calls at a six-deep stack, wall clock per
snapshot — the whole path, bash through the wire to the decoded JSON:

| | untraced | `--trace-calls` |
|---|---:|---:|
| the walk assembled in bash | 572 µs | 737 µs |
| the walk shipped as columns | 482 µs | 527 µs |

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
mechanism that cannot be checked by reading the source. One file per subject.

| `attaching.rs` | establishes |
|---|---|
| `a_shell_that_speaks_once_and_leaves_loses_nothing` | a `bash -c` that joins, says one thing and exits within microseconds loses nothing: the blocking open is the rendezvous |
| `a_fork_that_speaks_is_a_shell_of_its_own_and_parts_on_its_own` | a fork takes a pipe of its own, its `parted` precedes the parent's, and the parent's words stay the parent's |
| `two_labels_in_one_process_are_two_shells` | `BC_JOIN` twice is two pipes and two shells with one pid |
| `a_label_nobody_joined_is_an_error_by_absence` | `BC_INSTR NOPE …` names the label and the call site, returns 125, and the run knows nothing |

| `transport.rs` | establishes |
|---|---|
| `every_descendant_shell_reaches_the_run` | subshells, command substitutions and child processes are all shells; five of them |
| `many_shells_at_once_arrive_whole_and_apart` | 8 shells × 80 messages, half 9000 bytes, each pipe carries one shell's words |
| `a_message_of_wide_characters_arrives_whole` | 6000 `€` per message, longer than a pipe's atomic write, character for character |
| `nothing_is_lost_at_the_end` | 200 messages written immediately before exit are read after the subject is gone |
| `a_newline_inside_a_value_is_escaped_not_a_line` | a value containing `\n` arrives as one word |

| `transparency.rs` | establishes |
|---|---|
| `a_signalled_subject_is_reported_and_loses_nothing` | `Signal(15)`, `.shell_code() == 143`, and what was said before the signal survives |
| `a_clients_own_trap_and_ifs_are_untouched` | a client's own `EXIT` trap and `IFS` survive a message going out; the version read back under `IFS=,` |
| `a_clients_own_locale_is_untouched_by_a_wide_message` | `LC_ALL` before and after a 9000-byte message |

| `answering.rs` | establishes |
|---|---|
| `a_session_survives_every_way_of_answering` | 57 asks across ten shells, every answer form, one deliberately slow, one 100 KB, mixed with a message too wide for one write |
| `an_answer_may_wait_on_another_shells_word` | an answer awaiting a `Notify` that another shell's `hear` triggers completes — serving is concurrent |

| `starting.rs` | establishes |
|---|---|
| `the_rigs_word_reaches_every_shell_and_so_does_the_callers_environment` | `Setup::bash` puts the rig's word in the subject and a child it starts; a variable set with `env` is inherited the same way |
| `the_command_line_is_run_as_asked` | the run starts the program the argv names, with nothing appended |
| `a_subject_may_join_by_hand_where_it_chooses` | `env -u BASH_ENV`, then `source "$BC_SESSION/prelude.bash"`; children that sourced nothing are not shells |

| `serving.rs` | establishes |
|---|---|
| `a_shell_that_joined_is_heard_until_it_lets_go` | a client's words and its subshell's arrive; the session ends with the handle; the client's status is its own |
| `a_shell_the_session_outlived_is_left_to_its_own_devices` | a client that released the handle while running has `parted: None`, and its next word takes `SIGPIPE` |
| `a_joined_shell_may_publish_the_address_to_its_children` | exporting `BASH_ENV` to the prelude reaches a child process |
| `a_shell_says_what_it_is_rather_than_being_guessed_at` | an interactive shell joins by sourcing, and says `-i`, `-s`, no command line |

| `owning.rs` | establishes |
|---|---|
| `a_named_workspace_is_left_behind_without_its_fifos` | a kept workspace ends with `prelude.bash` and `rig.bash` and nothing that was a pipe |
| `a_shell_left_asking_does_not_outlive_the_run` | the run does not wait for a straggler, and the straggler does not survive it |
| `a_shell_outside_the_group_is_heard_and_never_signalled` | a `setsid` shell is heard, has `parted: None`, and is alive after the run |
| `a_panicking_answer_kills_the_subject` | the panic propagates out of `run`, and the blocked subject is gone |

| `malformed.rs` | establishes |
|---|---|
| `a_line_cut_short_by_a_shell_that_left_ends_the_run` | a fork that exits mid-line ends the run naming the line |
| `a_line_cut_short_at_the_end_is_reported_beside_the_subjects_status` | the same left by a shell the session outlived is `Run::failed`, beside the subject's status |
| `a_line_that_will_not_read_ends_the_run` | `(junk` ends the run quoting it |
| `an_account_out_of_place_ends_the_run` | a second `JOIN` line is not a message |

| `failing.rs` | establishes |
|---|---|
| `a_rig_that_cannot_answer_ends_the_run_and_kills_the_subject` | `run` yields the rig's reason, and the shell blocked on the ask does not outlive it |
| `a_failure_while_hearing_ends_the_run_and_kills_the_subject` | the same for a message nobody was waiting on, promptly, while another shell asks in a loop |
| `an_unknown_verb_is_reported_rather_than_ignored` | a verb the protocol does not define is named on stderr and returns 125 |

Bash-level invariants that hold without running anything are asserted against
the shipped text instead, and live beside it: the protocol's in
`src/bash/rig/wire/mod.rs`, each tool's in its own tests.

## Bash constraints that bound the design

**Traps do not compose.** Bash allows one handler per signal, so contributing
an `EXIT`/`ERR`/`DEBUG` fragment means adopting whatever handler the client
installed. This is why provenance and exit are carried by lines and by the
kernel rather than by a handler.

**A subshell resets caught traps.** Anything buffered in a `( … )` and flushed
from `EXIT` is lost. This is why a message is written where it is produced
rather than accumulated.

**`$?` must be read as a frame's first statement.**

**A bash arithmetic *command* is false when its result is 0.** `x=$(( x + n ))`
has no such status. This is why no instrument in the crate counts in bash.

**Under `extdebug`, a `DEBUG` handler returning non-zero skips the command it
fired for.** The handler must return 0.

**Enabling `extdebug` while `BASH_ENV` is being read starts the debugger.**
`bashcap`'s trace arms itself from a `DEBUG` trap on the next command, which
must be the subject's — so its join comes before the trap.

**`mkfifo` is not a builtin.** See above.

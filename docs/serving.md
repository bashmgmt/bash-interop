# Serving — bash orchestrates

```rust
pub trait Serving: Rig {
    async fn serve(&self, at: &Path, held: OwnedFd) -> Result<Served<Kept<Self>>, Failure>;

    async fn serve_coprocess(&self, at: &Path) -> Result<Served<Kept<Self>>, Failure>;
}

pub struct Served<K> { pub shells: Vec<Attended<K>>, pub failed: Option<Failure> }
```

A script that is already running names **and makes** the workspace, starts
the server, and lets go when it is done. Nothing on this side starts a
process or ends one. `at` is required with no fallback and must exist: the
client prescribed the directory, so it holds the address — the directory
itself — before the server has done anything. It is left behind; a reading
taken later may follow source paths into it, and removing it is the
client's, like everything else it initiated.

Nothing is written back: a serving application is a complete standalone
program, and there is no channel through which the core could hand a client
a value — `serve(at, held)` has no hook. Whether the session is up is the
workspace's to show: the join fifo is present exactly while one serves,
which the lock and the sweep keep truthful, so the client gates on the same
directory it named:

```bash
until BC_UP prof.d; do sleep 0.01; done
BC_LOAD prof.d
BASHPROF_INIT prof.d
```

The one boundary is a server killed outright — `SIGKILL` removes nothing —
whose stale fifo stands until its directory is next opened (swept) or
removed; the host that killed its own server owns that directory anyway.

A `Failure` while serving still sees the session out — every shell released
or finished, the fifos gone — before it is returned.

`held` is a descriptor the initiator keeps open for as long as it wants the
session. Serving ends when the last holder has let go, deliberately or by
dying. A descendant that inherited the handle keeps the session open, since it
can still speak. A shell that keeps talking after the session closed writes
into a fifo whose reader is gone and takes `SIGPIPE`.

### The coprocess convention

**The client holds the server's standard input, and reads nothing back.**
The vendored words, whole:

<!-- quote: assets/joining.bash anchor=words -->
```bash
# $@ is the server's command line, program included, and the workspace is in
# there as the server's own argument — this word does not know it. NAME is a
# literal in `coproc`'s grammar, so there is one server per shell.
BC_START() {
    coproc BC_SERVER { "$@"; }
}

# Is a session serving at $1? The join fifo is present exactly while one is:
# the server locks the workspace, removes its fifos on every failure it can
# observe, and sweeps a killed predecessor's leavings when it opens.
BC_UP() {
    [[ -p "${1:?the session workspace}/join" ]]
}

# Bring the session's definitions into this shell: the protocol's words, then
# the rig's. Nothing joins — that is the caller's next line.
BC_LOAD() {
    local __bc_dir="${1:?the session workspace}"

    source "$__bc_dir/prelude.bash"
    source "$__bc_dir/rig.bash"
}

# Let go, and wait for what the client started. Whoever initiates cleans up;
# nothing on the Rust side kills anything. When this returns the server has
# seen the session out and written whatever it writes, and its status is this
# word's.
BC_LEAVE() {
    local __bc_handle="${BC_SERVER[1]:?no server was started}"
    exec {__bc_handle}>&-

    wait "$BC_SERVER_PID"
}
```

```bash
source lib/joining.bash

mkdir -p prof.d
BC_START bashprof serve --at prof.d --into build.times   # start; nothing awaited
until BC_UP prof.d; do sleep 0.01; done                  # the workspace shows it
BC_LOAD prof.d                                           # definitions, by the same dir
BASHPROF_INIT prof.d                                     # the client's own initiation
BC_INSTR BASHPROF say STEP compile
BC_LEAVE                                     # release, wait, return its status
```

`assets/joining.bash` is the bash half; `Serving::serve_coprocess` is the Rust
half. `coproc` takes a literal NAME, so the server's fds live in `BC_SERVER`
and there is one server per shell; the client feeds the same directory to
every step, and `BC_UP` is one file test. `BC_LEAVE` returns the
server's status, so a client under `set -e` stops on a server that failed,
and by the time it returns whatever the server writes is written.

`JOINING` (`rig/joining.txt`) is the whole list — driven and already joined,
by hand, started as a coprocess, only if there is a session, the vendored words
and their polyfill — and both binaries print it under `run --help` and
`serve --help`.

`__fixtures/joined/build.bash` starts the shipped `bashprof serve` and is
driven from `tests/cli.rs`; `merging.bash` starts `tests/joined/merging.rs`, a
program rather than a harness because a script that starts its own server has
to have something to start.


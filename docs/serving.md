# Serving

Under the served role the program is already running, and it starts the
session, usually by starting a tool's `serve` verb as a coprocess. This is the
mode for a script that instruments itself: it decides when the session begins,
does its work, and collects the reading when it is done. The Rust side starts
nothing, kills nothing, and writes nothing back to the client.

The surface, abridged; rustdoc is authoritative:

```rust
pub trait Serving: Rig {
    async fn serve(&self, at: &Path, held: OwnedFd) -> Result<Served<Kept<Self>>, Failure>;

    /// serve, with the handle being this process's own standard input —
    /// the coprocess convention's server half.
    async fn serve_coprocess(&self, at: &Path) -> Result<Served<Kept<Self>>, Failure>;
}

pub struct Served<K> { pub shells: Vec<Attended<K>>, pub failed: Option<Failure> }
```

Two parameters carry the contract.

`at` is the workspace, which the client names and makes. It is required, has
no fallback, and must already exist, so the client holds the session's address
before the server has done anything and nothing needs to be communicated back.
The directory is left behind when the session ends, since readings taken later
may follow source paths into it, and removing it is the client's job.

`held` is a descriptor whose release ends the session. The session watches
this fd and serves as long as somebody could still hold it open. That somebody
is plural: file descriptors are inherited, so a subshell or child of the client
keeps the session alive for as long as it lives, and the session cannot end
while a process that might still speak exists. When the last holder closes it,
deliberately or by dying, the watch fires and the session closes. A shell that
talks after that writes into a fifo whose reader is gone and takes `SIGPIPE`.

Whether the session is up is the workspace's to show. The `join` fifo exists
exactly while a session serves, kept truthful by the lock and the sweep
([rigs.md](rigs.md)), so the client gates on the same directory it named. The
boundary case is a server killed with `SIGKILL`, which removes nothing; its
stale fifo stands until the directory is next opened and swept, or removed.

A `Failure` while serving still sees the session out — every shell released or
finished, the fifos gone — before it is returned in `Served::failed`.

## The coprocess convention

Bash's `coproc` starts a process and hands the script both ends, the process's
stdin as a write end and its stdout as a read end. The convention here is that
the client keeps the server's stdin as the handle and reads nothing, and
`serve_coprocess` is the server half, taking its own stdin as `held`.

The client's half is four moves of plain bash: start, probe, load and
initiate, let go. The whole script, which also lives in
`bashprof/__fixtures/book/` where the tool's cli suite runs it as printed:

```bash
#!/usr/bin/env bash
# Owns the session: names the workspace, starts the server, probes, loads,
# initiates — and leaves by closing the handle coproc left it.
set -euo pipefail

declare -- workspace="$PWD/prof.d"   # an address is absolute — initiation refuses else
mkdir -p "$workspace"

coproc SERVER { bashprof serve --at "$workspace" --into build.times; }
until [[ -p "$workspace/join" ]]; do sleep 0.01; done   # up exactly while serving

source "$workspace/prelude.bash"    # the protocol's words
source "$workspace/rig.bash"        # the rig's
BASHPROF_INIT "$workspace"

build() { sleep 0.1; }
BASHPROF_TIMETHIS build build

declare -- handle="${SERVER[1]}"
exec {handle}>&-    # let go: what was held is the server's standard input
wait "$SERVER_PID"  # it sees the session out; its status is this script's
```

Three details in that script are worth spelling out.

The `until` gate is there because `coproc` returns before the server has
parsed its arguments, and sourcing the laid files before they exist would
fail. The gate polls the one truthful signal. A client joining much later,
with the session serving all along, needs no gate.

The server's descriptors are `SERVER[0]` and `SERVER[1]`, because `coproc
NAME { … }` takes a literal name; that also means one server per shell under
this convention. The copy into `handle` before the close is because `exec
{name}>&-` closes the descriptor a variable names.

The `wait` returns the server's own exit status, so a client under `set -e`
stops when its server failed. By the time it returns, whatever the server
writes is on disk, because the server writes after seeing the session out.

Every other way a shell can join a served session, by hand from the pieces or
published to children, is a whole script in [joining.md](joining.md).
bashprof's `__fixtures/joined/build.bash` is a working client of this shape,
exercised by its cli suite.

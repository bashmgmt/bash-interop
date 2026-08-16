# Serving — bash orchestrates

The served role inverts the driven one: here the program under study is
already running, and *it* starts the session — usually by starting a tool's
`serve` verb as a coprocess. This is the mode for a script that instruments
itself: it decides when the session begins, does its work, and collects the
reading when it decides to be done. The Rust side starts nothing, kills
nothing, and owes nobody a byte of output.

The surface (abridged — rustdoc is authoritative):

```rust
pub trait Serving: Rig {
    async fn serve(&self, at: &Path, held: OwnedFd) -> Result<Served<Kept<Self>>, Failure>;

    /// serve, with the handle being this process's own standard input —
    /// the coprocess convention's server half.
    async fn serve_coprocess(&self, at: &Path) -> Result<Served<Kept<Self>>, Failure>;
}

pub struct Served<K> { pub shells: Vec<Attended<K>>, pub failed: Option<Failure> }
```

Two parameters carry the whole contract, so take them one at a time.

**`at` — the client names and makes the workspace.** Required, no fallback,
and it must already exist: the client chose the directory, so it holds the
session's address before the server has done anything, and nothing needs to
be communicated back. The directory is left behind when the session ends —
readings taken later may follow source paths into it — and removing it is
the client's job, like everything else it initiated.

**`held` — a descriptor whose release ends the session.** The session
watches this fd and serves exactly as long as *somebody* could still hold
it open. That "somebody" is plural on purpose: file descriptors are
inherited, so a subshell or child of the client keeps the session alive for
as long as it lives — the session cannot end while a process that might
still speak exists. When the last holder closes it (deliberately, or by
dying), the watch fires and the session closes. A shell that talks *after*
that writes into a fifo whose reader is gone and takes `SIGPIPE` — the
ordinary Unix outcome, not a special one.

Nothing is ever written back to the client: `serve` has no announce hook,
no ready line, no channel it could leak a value through. Whether the
session is up is the **workspace's** to show — the `join` fifo exists
exactly while a session serves (the lock and the sweep keep that truthful —
[rigs.md](rigs.md)), so the client gates on the same directory it named.
The one boundary: a server killed outright (`SIGKILL`) removes nothing, and
its stale fifo stands until the directory is next opened (and swept) or
removed — the host that kill-nines its own server owns that directory
anyway.

A `Failure` while serving still sees the session out — every shell released
or finished, the fifos gone — before it is returned in `Served::failed`.

## The coprocess convention

How does a bash script actually hold such a descriptor? Through bash's own
`coproc`: it starts a process and hands the script both ends — the
process's stdin (write end) and stdout (read end). The convention: **the
client keeps the server's stdin as the handle, and reads nothing** —
`serve_coprocess` is the server half, taking its own stdin as `held`.

The client's half is four moves of plain bash — start, probe, load and
initiate, let go — reading top to bottom:

```bash
declare -- workspace="$PWD/prof.d"                      # the address, absolute
mkdir -p "$workspace"                                   # …and the client makes it
coproc SERVER { bashprof serve --at "$workspace" --into build.times; }
until [[ -p "$workspace/join" ]]; do sleep 0.01; done   # the workspace shows readiness

source "$workspace/prelude.bash"                        # definitions, by the same dir
source "$workspace/rig.bash"
BASHPROF_INIT "$workspace"                              # the client's own initiation

BC_INSTR BASHPROF say STEP compile                      # …the instrumented work…

declare -- handle="${SERVER[1]}"
exec {handle}>&-                                        # let go: close the held write end
wait "$SERVER_PID"                                      # …and take the server's status
```

Details a careful reader asks about here:

- **Why the `until` gate?** `coproc` returns before the server has even
  parsed its arguments; sourcing the laid files before they exist would
  fail. The gate polls the one truthful signal. A client that joins much
  later (the session serving all along) needs no gate at all.
- **Where do the server's fds live?** `coproc NAME { … }` takes a literal
  name, so they are `SERVER[0]`/`SERVER[1]` — which also means one server
  per shell under this convention. The copy into `handle` before the close
  is because `exec {name}>&-` closes the descriptor a *variable* names.
- **What does the `wait` return?** The server's own exit status, so a
  client under `set -e` stops when its server failed — and by the time it
  returns, whatever the server writes (the profile, the capture) is on
  disk, because the server writes after seeing the session out.

Every other way a shell can join a served session — by hand from the
pieces, published to children — is a whole script in
[joining.md](joining.md). In this repository, `__fixtures/joined/` holds a
working client of exactly this shape, exercised by the test suites.

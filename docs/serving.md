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

The client's side is four vendored words. What each does, in one line:

| word | does |
|---|---|
| `BC_START server…` | starts the server as the coprocess; nothing is awaited |
| `BC_UP dir` | one file test: is a session serving at `dir`? |
| `BC_LOAD dir` | sources the two laid files — definitions in, nothing joined |
| `BC_LEAVE` | closes the handle, waits for the server, returns its status |

And the words themselves, whole, as a client vendors them
(`assets/joining.bash` — the same bytes the byte-assert in every consumer
enforces):

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

A complete client, reading top to bottom:

```bash
source lib/joining.bash

declare -- workspace="$PWD/prof.d"                            # the address, absolute
mkdir -p "$workspace"                                         # …and the client makes it
BC_START bashprof serve --at "$workspace" --into build.times  # start; nothing is read back
until BC_UP "$workspace"; do sleep 0.01; done                 # the workspace shows readiness
BC_LOAD "$workspace"                                          # definitions, by the same dir
BASHPROF_INIT "$workspace"                                    # the client's own initiation

BC_INSTR BASHPROF say STEP compile                            # …the instrumented work…

BC_LEAVE                                                      # let go, wait, take its status
```

Details a careful reader asks about here:

- **Why the `until BC_UP` gate?** `coproc` returns before the server has
  even parsed its arguments; sourcing the laid files before they exist
  would fail. The gate polls the one truthful signal. A client that joins
  much later (the session serving all along) needs no gate at all.
- **Where do the server's fds live?** `coproc NAME { … }` takes a literal
  name, so they are `BC_SERVER[0]`/`BC_SERVER[1]` — which also means one
  server per shell under this convention.
- **What does `BC_LEAVE` return?** The server's own exit status, so a
  client under `set -e` stops when its server failed — and by the time it
  returns, whatever the server writes (the profile, the capture) is on
  disk, because the server writes after seeing the session out.

Every other way a shell can join a served session — by hand from the
pieces, published to children — is a whole script in
[joining.md](joining.md). In this repository, `__fixtures/joined/` holds a
working client of exactly this shape, exercised by the test suites.

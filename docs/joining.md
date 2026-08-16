# Joining — every way in, and who initiates

Joining has two halves, and the design keeps them apart. **Loading** brings
the definitions into a shell: `source <dir>/prelude.bash` (the protocol's
words), then `source <dir>/rig.bash` (the rig's — inert, like the first).
**Initiation** opens the channel: one line of client code,
`BC_JOIN <label> <dir> [word…]` or the rig's init function wrapping it. A
shell that loaded and never initiated has the words and is not a shell of
the run; a shell that initiates without loading fails loudly at an unknown
command.

The one exception is stated, never implied: a run may **provision**
`<dir>/bash_env.bash`, and whoever provisions it states first whether it
initiates (`Provision::Joining` — the file ends with the rig's joining
line) or only defines (`Provision::Definitions`). That file is what
`BASH_ENV` names, so it reaches every non-interactive bash in the subject's
tree as it starts — the only way to join subjects that have never heard of
the session, and the fringe where auto-initiation lives.

Every way a script joins — `JOINING`, printed by both tools under `--help`:

```bash
# A shell under a driven run with --reach bash-env needs nothing: the
# provisioned file defined and joined it as it started. Under --reach
# by-hand the file only defined, the workspace is in BC_SESSION, and the
# script initiates where it says so:
BASHPROF_INIT "$BC_SESSION"

# A script that starts the tool itself, as a coprocess:
source lib/joining.bash
mkdir -p prof.d
BC_START bashprof serve --at prof.d --into build.times
until BC_UP prof.d; do sleep 0.01; done
BC_LOAD prof.d
BASHPROF_INIT prof.d
BC_LEAVE

# From the pieces, told the workspace as an argument — no vendored words,
# no environment:
source "$1/prelude.bash"
source "$1/rig.bash"
BC_JOIN TELL "$1"

# Publishing to child processes is the client's own: it writes its own
# startup file and exports BASH_ENV to it —
printf 'source %q\nsource %q\nBC_JOIN TELL %q\n' \
    "$workspace/prelude.bash" "$workspace/rig.bash" "$workspace" > own.bash
export BASH_ENV="$PWD/own.bash"
```

Which shells the session reaches is therefore always a decision with an
author: the run's, in its environment closure; the provisioning caller's, in
its stated `Provision`; the script's, at its own init line. The core runs no
initiation and prefers no way in.

The proofs behind each way: `tests/proofs/starting.rs` (provisioned, both
ways, and by-hand), `tests/proofs/serving.rs` (coprocess, from the pieces,
the client-authored startup file, an interactive shell typing the same).

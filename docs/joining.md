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

Every way in reaches the same end state — the words defined, the channel
open — from a different starting situation. Each is one whole script below;
the body of work is the same on purpose, so the prologues carry the whole
difference. (`JOINING`, printed by both tools under `--help`, is the
condensed card of the same list.)

## Driven, `--reach bash-env` — the subject knows nothing

```bash
#!/usr/bin/env bash
# Started as:  bashprof run --into build.times -- bash build.bash
#
# The provisioned bash_env.bash defined the words and said the join in
# every shell of this tree as it started. Nothing of the protocol appears
# here — this is the way in for programs that never heard of the session.
set -euo pipefail

build() { sleep 0.1; }
BASHPROF_TIMETHIS build build
```

## Driven, `--reach by-hand` — defined everywhere, joined where it says

```bash
#!/usr/bin/env bash
# Started as:  bashprof run --reach by-hand --into build.times -- bash build.bash
#
# The provisioned file only defined; BC_SESSION carries the workspace.
# Nothing is joined until this script's own line — shells it starts before
# that are not shells of the run.
set -euo pipefail
declare -- workspace="${BC_SESSION:?the workspace, from the tool}"

bash prepare.bash          # before the join: defined, but not a shell of the run

BASHPROF_INIT "$workspace"

build() { sleep 0.1; }
BASHPROF_TIMETHIS build build
```

## A coprocess client — this script owns the session

```bash
#!/usr/bin/env bash
# Started by nobody: it starts the server itself, on a workspace it names
# and makes, and holds the session open for as long as it runs.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib/joining.bash"   # the vendored words

declare -- workspace=prof.d
mkdir -p "$workspace"
BC_START bashprof serve --at "$workspace" --into build.times
until BC_UP "$workspace"; do sleep 0.01; done
BC_LOAD "$workspace"
BASHPROF_INIT "$workspace"

build() { sleep 0.1; }
BASHPROF_TIMETHIS build build

BC_LEAVE                   # let go, wait, take the server's status
```

## From the pieces — told the workspace as an argument

```bash
#!/usr/bin/env bash
# Started as:  bash join-and-speak.bash <workspace>
#
# No vendored words, no environment: the two laid files are everything,
# and the coordinate arrives as argv. This is what BC_LOAD does, spelled
# out — and the rig's init function is a raw BC_JOIN here.
set -euo pipefail
declare -- workspace="${1:?the session workspace}"

source "$workspace/prelude.bash"
source "$workspace/rig.bash"
BC_JOIN TELL "$workspace"

BC_INSTR TELL say STEP joined-from-the-pieces
```

## Publishing to child processes — the client authors the startup file

```bash
#!/usr/bin/env bash
# Already joined (any way above); wants the processes it starts joined
# too. No laid file initiates, so it writes its own startup file —
# %q is bash's own quoting — and points BASH_ENV at it: bash sources
# that file in every non-interactive child as it starts.
set -euo pipefail
declare -- workspace="${1:?the session workspace}"

declare -- own="${BASH_SOURCE[0]%/*}/own.bash"
printf 'source %q\nsource %q\nBC_JOIN TELL %q\n' \
    "$workspace/prelude.bash" "$workspace/rig.bash" "$workspace" > "$own"
export BASH_ENV="$own"

bash child.bash            # a fresh bash: sources $BASH_ENV, joins, speaks
```

Which shells the session reaches is therefore always a decision with an
author: the run's, in its environment closure; the provisioning caller's, in
its stated `Provision`; the script's, at its own init line. The core runs no
initiation and prefers no way in.

The proofs behind each way: `tests/proofs/starting.rs` (provisioned, both
ways, and by-hand), `tests/proofs/serving.rs` (coprocess, from the pieces,
the client-authored startup file, an interactive shell typing the same).

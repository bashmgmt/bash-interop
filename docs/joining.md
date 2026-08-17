# Joining

Joining has two halves, kept apart.

Loading brings the definitions into a shell: `source <dir>/prelude.bash` for
the protocol's words, then `source <dir>/rig.bash` for the rig's, both inert.
Initiation opens the channel: one line of client code, `BC_JOIN <label> <dir>
[word…]`, or the rig's init function wrapping it. A shell that loaded and
never initiated has the words and is not part of the run; a shell that
initiates without loading fails at an unknown command.

The exception is stated rather than implied. A run may provision
`<dir>/bash_env.bash`, and whoever provisions it states whether that file
initiates, with `Provision::Joining` putting the rig's joining line at the
end, or only defines, with `Provision::Definitions`. `BASH_ENV` names that
file, so it reaches every non-interactive bash in the subject's tree as it
starts. This is how a subject that has never heard of the session comes to
join, and it is the only place where initiation happens automatically.

Every way in reaches the same end state, the words defined and the channel
open, from a different starting situation. Each is one whole script below,
and since the body of work is the same in each, the prologues carry the
difference.

`bashprof` stands in for any program built on the core. Its `--reach
bash-env|by-hand` flags are that tool's spelling of the two provisioning
choices, and each tool prints this same list in its own words under `run
--help` and `serve --help`. The three bashprof-driven scripts also live in
`bashprof/__fixtures/book/`, where its cli suite runs them as printed, and the
two tool-free ones are the shapes `tests/proofs/serving.rs` proves.

## Driven, provisioned to join

The subject knows nothing about the session.

```bash
#!/usr/bin/env bash
# Started as:  bashprof run --into build.times -- bash provisioned.bash
#
# The provisioned bash_env.bash defined the words and said the join in
# every shell of this tree as it started. Nothing of the protocol appears
# here — this is the way in for programs that never heard of the session.
set -euo pipefail

build() { sleep 0.1; }
BASHPROF_TIMETHIS build build
```

## Driven, definitions only

Under a joining provision every shell of the tree joins, helpers included, so
a dependency fetch or a `./configure` fills the reading with shells nobody
asked about. Here the tool provisions a `Definitions` file instead. Every
shell still gets the words at startup, since `BASH_ENV` reaches them all, and
no shell is joined until its own code says so. The script below joins itself
and leaves its helper out.

```bash
#!/usr/bin/env bash
# Started as:  bashprof run --reach by-hand --into build.times -- bash by-hand.bash
set -euo pipefail
declare -- workspace="${BASHPROF_SESSION:?the workspace, from the tool}"

# fetch-deps.bash is an ordinary helper of this build — not part of the
# protocol. Like every shell in the tree it wakes up with the words
# defined; nobody initiates in it, so it stays outside the session: it
# runs exactly as it would unwrapped, and nothing it does is heard.
bash "${BASH_SOURCE[0]%/*}/fetch-deps.bash"

# From here on, THIS shell is part of the run.
BASHPROF_INIT "$workspace"

build() { sleep 0.1; }
BASHPROF_TIMETHIS build build
```

Staying outside the session holds for a shell that does not call the tool's
words. If `fetch-deps.bash` said `BASHPROF_TIMETHIS` itself, that word would
refuse with `label BASHPROF is not joined`, status 125, at its own call site
and before running the wrapped command, so under `set -e` the helper would
stop there. A call site asked for a measurement, and measuring into nowhere
would be the worse answer. A helper that shares the tool's words joins too, or
is left as it is.

## A coprocess client

This script owns the session. It starts the server itself, on a workspace it
names and makes, and holds the session open for as long as it runs. Everything
here is bash's own — `coproc` is a keyword and the probe is one file test —
and the only files sourced are the two the session laid.

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

The handle is the write end of the server's standard input, which `coproc`
left in `${SERVER[1]}`. `Serving::serve_coprocess` watches that descriptor,
and the session lasts as long as somebody holds it, a subshell that inherited
it included. Closing it is the act of leaving, and `wait` then collects a
server that has seen the session out. The convention's fine print is in
[serving.md](serving.md#the-coprocess-convention).

## From the pieces

The workspace arrives as an argument.

```bash
#!/usr/bin/env bash
# Started as:  bash join-and-speak.bash <workspace>
#
# No environment and no words of our own: the two laid files are
# everything, and the coordinate arrives as argv. The same load as above, without a
# server to start — and the rig's init function is a raw BC_JOIN here.
set -euo pipefail
declare -- workspace="${1:?the session workspace}"

source "$workspace/prelude.bash"
source "$workspace/rig.bash"
BC_JOIN TELL "$workspace"

BC_INSTR TELL say STEP joined-from-the-pieces
```

## Publishing to child processes

The client authors its own startup file.

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

Which shells the session reaches is always a decision with an author: the
run's, in its environment closure; the provisioning caller's, in its stated
`Provision`; the script's, at its own init line. The core runs no initiation.

The proofs behind each way are `tests/proofs/starting.rs` for provisioned,
both ways, and by-hand, and `tests/proofs/serving.rs` for the coprocess, from
the pieces, the client-authored startup file, and an interactive shell typing
the same.

# The client's side of a session it drives itself. A script sources this,
# starts a server with it, and from then on `BC_INSTR` is defined:
#
#     source lib/joining.bash
#     BC_JOIN bashprof serve --into build.times
#
#     BC_INSTR say STEP compile
#     BC_INSTR ask NEXT
#
#     BC_LEAVE
#
# Unlike a tool's own words, this file is only ever vendored: it is what runs
# before there is anything to inject, and what brings the protocol into the
# shell in the first place.
#
# The convention it stands on has a second half in Rust — `Slave::serve_coprocess`
# — and it is this: the client holds the server's standard input, and the
# server writes one line on its standard output, the command that joins.
#
# The session lasts as long as anyone holds that handle. A subshell inherits it
# and keeps the session open for as long as it lives, which is right, because
# it can still speak.

# $@ is the server's command line, program included. `coproc` gives the client
# both ends at once: [0] is where the address arrives, [1] is the handle.
#
# NAME is a literal in `coproc`'s grammar, so there is one session per shell —
# the same count the protocol keeps in `__BC__owner`. What it makes is the
# shell's, not this frame's, and outlives the return.
#
# 125 is what the protocol's own words return when they could not do their job.
# A server that died before announcing gets us end of input rather than a line.
#
# `declare -a` reads the address exactly as `__bc_ask` reads an answer, because
# it is the same kind of thing: one command, as a bash array literal.
BC_JOIN() {
    coproc BC_SESSION { "$@"; }

    local __bc_address
    IFS= read -r __bc_address <&"${BC_SESSION[0]}" || return 125

    local -a __bc_join="$__bc_address"
    "${__bc_join[@]}"
}

# Let go, and wait for what the client started. Whoever initiates cleans up,
# and nothing on the Rust side kills anything.
#
# When this returns, the server has seen the session out and written whatever
# it writes. Its status is this word's, so a client under `set -e` stops on a
# server that failed.
BC_LEAVE() {
    local __bc_handle="${BC_SESSION[1]}"
    exec {__bc_handle}>&-

    wait "$BC_SESSION_PID"
}

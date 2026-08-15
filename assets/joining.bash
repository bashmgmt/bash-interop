# The client's side of a session it drives itself. A script sources this and
# starts a server with it; from then on `BC_INSTR` is defined:
#
#     source lib/joining.bash
#     BC_START bashprof serve --into build.times
#
#     BC_INSTR BASHPROF say STEP compile
#     BC_INSTR BASHPROF ask NEXT
#
#     BC_LEAVE
#
# This file is only ever vendored: it runs before there is anything to inject,
# and is what brings the protocol into the shell in the first place.
#
# The convention has a second half in Rust, `Serving::serve_coprocess`: the
# client holds the server's standard input, and the server writes one line on
# its standard output — the command that joins.
#
# The session lasts as long as anyone holds that handle. A subshell inherits it
# and keeps the session open for as long as it lives, because it can still
# speak.

# $@ is the server's command line, program included. `coproc` gives the client
# both ends at once: [0] is where the address arrives, [1] is the handle. NAME
# is a literal in `coproc`'s grammar, so there is one server per shell.
#
# 125 is what the protocol's own words return when they could not do their job;
# a server that died before announcing gives end of input instead of a line.
# `declare -a` reads the address as `__bc_ask` reads an answer: one command,
# as a bash array literal.
BC_START() {
    coproc BC_SESSION { "$@"; }

    local __bc_address
    IFS= read -r __bc_address <&"${BC_SESSION[0]}" || return 125

    local -a __bc_start="$__bc_address"
    "${__bc_start[@]}"
}

# Let go, and wait for what the client started. Whoever initiates cleans up;
# nothing on the Rust side kills anything. When this returns the server has
# seen the session out and written whatever it writes, and its status is this
# word's.
BC_LEAVE() {
    local __bc_handle="${BC_SESSION[1]}"
    exec {__bc_handle}>&-

    wait "$BC_SESSION_PID"
}

# The client's words for a session it drives itself. A script sources this,
# makes a workspace, starts a server on it and attaches; from then on
# `BC_INSTR` is defined:
#
#     source lib/joining.bash
#     mkdir -p prof.d
#     BC_START bashprof serve --at prof.d --into build.times
#     until BC_UP prof.d; do sleep 0.01; done
#     BC_ATTACH prof.d
#
#     BC_INSTR BASHPROF say STEP compile
#     BC_INSTR BASHPROF ask NEXT
#
#     BC_LEAVE
#
# This file is only ever vendored: it runs before there is anything to inject,
# and is what brings the protocol into the shell in the first place. Every
# other way in is under `bashprof --help` and `bashcap --help`.
#
# The client feeds the same directory to start, probe and attach; nothing is
# read back from the server, which is a complete standalone program. The
# session lasts as long as anyone holds the handle `coproc` gave: a subshell
# inherits it and keeps the session open for as long as it lives.

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
    [[ -p "$1/join" ]]
}

# Join the session at $1: source the file its server laid.
BC_ATTACH() {
    source "$1/session.bash"
}

# Let go, and wait for what the client started. Whoever initiates cleans up;
# nothing on the Rust side kills anything. When this returns the server has
# seen the session out and written whatever it writes, and its status is this
# word's.
BC_LEAVE() {
    local __bc_handle="${BC_SERVER[1]}"
    exec {__bc_handle}>&-

    wait "$BC_SERVER_PID"
}

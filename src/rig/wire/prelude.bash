# The client half of the protocol, sourced through BASH_ENV by every shell in
# the subject's process tree. It is shipped verbatim into the run's workspace
# and finds everything it needs from its own path, so nothing is templated in.

__BC__DIR="${BASH_SOURCE[0]%/*}"
__BC__UP="$__BC__DIR/up"

# Under PIPE_BUF (4096) with room for the frame header and the delimiter, so a
# frame is always one atomic write.
__BC__limit=3900

# `BC_INSTR` could not do its job — as distinct from an answer that ran and
# returned non-zero, which is the subject's own business. 125 is what `env`
# and `timeout` return when the wrapper rather than the payload failed.
__BC__FAILED=125

__BC__owner=""
__BC__parent=""
__BC__up=""
__BC__seq=0
__BC__at=""
__BC__rc=0

# Error flow here is ours, not `set -e`'s: every command that can fail is
# followed by `|| __BC_BAIL` or `|| __BC_THROW`, which exempts it from errexit
# and hands us the status. Aliases rather than functions because `return` must
# act in the frame that failed; `expand_aliases` is therefore on before
# anything using them is parsed.
shopt -s expand_aliases

alias __BC_BAIL='{ __BC__rc=$?; return "$__BC__rc"; }'
alias __BC_THROW='{ __BC__rc=$?; __bc_complain "${FUNCNAME[0]} ($__BC__rc)"; return "$__BC__FAILED"; }'

# One line per fault, naming the call site in the subject rather than a line
# of ours, which is what a reader can act on.
__bc_complain() {
    printf 'BC_INSTR: %s at %s\n' "$1" "${__BC__at:-?}" >&2
}

BC_INSTR() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"

    case "${1-}" in
        say) shift
             { [[ $BASHPID == "$__BC__owner" ]] || __bc_join; } || __BC_BAIL
             __bc_send SAY "$@" || __BC_BAIL ;;
        ask) shift; __bc_ask "$@" ;;
        *)   __bc_complain "unknown verb ${1-}"
             return "$__BC__FAILED" ;;
    esac
}

# One pipe per question, named for the message that asks it — `__bc_send`
# stamps the sequence number read here — and removed by the run once answered,
# so `mkfifo` is one attempt against a name nothing else holds. Read-write, so
# the read waits for an answer instead of seeing end of input.
#
# Running the answer is the one unguarded command here: its status is the
# result a subject asking a question wants.
__bc_ask() {
    { [[ $BASHPID == "$__BC__owner" ]] || __bc_join; } || __BC_BAIL

    local __bc_reply="$__BC__DIR/rep.$BASHPID.$__BC__seq"
    local __bc_fd
    mkfifo "$__bc_reply" || __BC_THROW
    exec {__bc_fd}<>"$__bc_reply" || __BC_THROW

    __bc_send ASK "$@" || __BC_BAIL

    local __bc_line
    IFS= read -r __bc_line <&"$__bc_fd" || __BC_THROW
    exec {__bc_fd}>&- || __BC_THROW

    local -a __bc_answer="$__bc_line"

    "${__bc_answer[@]}"
}

# $1 is the kind; the rest is the client's arglist. The protocol's own words
# go in front of it, and the reader shifts exactly those back off.
__bc_send() {
    set -- "$1" "at=$EPOCHREALTIME" "parent=$__BC__parent" "shlvl=$SHLVL" "${@:2}"

    local __bc_msg
    printf -v __bc_msg '%s ' "${@@Q}"
    __bc_msg="(${__bc_msg% })"

    if (( ${#__bc_msg} <= __BC__limit )); then
        printf '. %s %s %s\n' \
            "$BASHPID" "$((__BC__seq++))" "$__bc_msg" >&"$__BC__up" || __BC_THROW
        return 0
    fi

    __bc_split "$__bc_msg" || __BC_BAIL
}

# A shell announces nothing: its first message carries seq 0, which is what
# says a shell has joined.
__bc_join() {
    __BC__parent=${__BC__owner:-$PPID}

    exec {__BC__up}>"$__BC__UP" || __BC_THROW
    __BC__owner=$BASHPID
    __BC__seq=0
}

# $1 is a message too wide for one frame. Every chunk carries the same
# `pid seq`, the key the reader rejoins them by. The cursor moves by
# assignment: `(( x += n ))` is a command, and its status is false when the
# result is 0.
__bc_split() {
    local __bc_msg="$1"
    local __bc_head="$BASHPID $((__BC__seq++))"
    local __bc_from=0

    while (( __bc_from + __BC__limit < ${#__bc_msg} )); do
        printf '+ %s %s\n' \
            "$__bc_head" "${__bc_msg:__bc_from:__BC__limit}" >&"$__BC__up" || __BC_THROW
        __bc_from=$(( __bc_from + __BC__limit ))
    done

    printf '. %s %s\n' "$__bc_head" "${__bc_msg:__bc_from}" >&"$__BC__up" || __BC_THROW
}

# The rig's own bash, laid down beside this file. The run always writes it, so
# its absence is a broken setup and ours to report; what it does when it runs
# is the rig's, and that status is forwarded as it stands.
[[ -f $__BC__DIR/rig.bash ]] || __BC_THROW
source "$__BC__DIR/rig.bash" || __BC_BAIL

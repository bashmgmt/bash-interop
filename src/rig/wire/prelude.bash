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

# Every command whose failure is a fault is followed by `|| __BC_BAIL` or
# `|| __BC_THROW`. A `||` list suppresses errexit for everything it calls, so
# unguarded a function runs on past its own first failure and one fault
# becomes two. The guards are what make it stop there.
#
# Aliases rather than functions because `return` must act in the frame that
# failed; `expand_aliases` is therefore on before anything using them is
# parsed. `$?` is read in the first command of each, which is the only place
# it survives, and every one of them fires from inside a function.
shopt -s expand_aliases

alias __BC_BAIL='return $?'
alias __BC_THROW='{ __bc_complain "${FUNCNAME[0]} ($?)"; return "$__BC__FAILED"; }'

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
             __bc_send SAY "$(( __BC__seq++ ))" "$@" || __BC_BAIL ;;
        ask) shift; __bc_ask "$@" ;;
        *)   __bc_complain "unknown verb ${1-}"
             return "$__BC__FAILED" ;;
    esac
}

# One pipe per question, named for the message that carries it, and removed
# by the run once answered — so `mkfifo` is one attempt against a name nothing
# else holds. Read-write, so the read waits for an answer instead of seeing
# end of input.
#
# The array assignment is unguarded because it cannot fail: bash puts text it
# cannot read as a literal into one element, and running that is a command not
# found, which is the subject's business. So is the answer's own status, which
# is why running it is unguarded too.
__bc_ask() {
    { [[ $BASHPID == "$__BC__owner" ]] || __bc_join; } || __BC_BAIL

    local __bc_seq=$(( __BC__seq++ ))
    local __bc_reply="$__BC__DIR/rep.$BASHPID.$__bc_seq"
    local __bc_fd
    mkfifo "$__bc_reply" || __BC_THROW
    exec {__bc_fd}<>"$__bc_reply" || __BC_THROW

    __bc_send ASK "$__bc_seq" "$@" || __BC_BAIL

    local __bc_line
    IFS= read -r __bc_line <&"$__bc_fd" || __BC_THROW
    exec {__bc_fd}>&- || __BC_THROW

    local -a __bc_answer="$__bc_line"

    "${__bc_answer[@]}"
}

# $1 the kind, $2 the sequence number this message carries, the rest the
# client's arglist. The counter belongs to the shell and is spent by the
# caller, so this is a function of what it is passed. The protocol's own words
# go in front of the arglist, and the reader shifts exactly those back off.
__bc_send() {
    local __bc_seq="$2"
    set -- "$1" "at=$EPOCHREALTIME" "parent=$__BC__parent" "shlvl=$SHLVL" "${@:3}"

    local __bc_msg
    printf -v __bc_msg '%s ' "${@@Q}"
    __bc_msg="(${__bc_msg% })"

    if (( ${#__bc_msg} <= __BC__limit )); then
        printf '. %s %s %s\n' \
            "$BASHPID" "$__bc_seq" "$__bc_msg" >&"$__BC__up" || __BC_THROW
        return 0
    fi

    __bc_split "$__bc_seq" "$__bc_msg" || __BC_BAIL
}

# A shell announces nothing: its first message carries seq 0, which is what
# says a shell has joined.
__bc_join() {
    __BC__parent=${__BC__owner:-$PPID}

    exec {__BC__up}>"$__BC__UP" || __BC_THROW
    __BC__owner=$BASHPID
    __BC__seq=0
}

# $1 the sequence number, $2 a message too wide for one frame. Every chunk
# repeats `pid seq`, the key the reader rejoins them by. The cursor moves by
# assignment: `(( x += n ))` is a command, and its status is false when the
# result is 0.
__bc_split() {
    local __bc_head="$BASHPID $1"
    local __bc_msg="$2"
    local __bc_from=0

    while (( __bc_from + __BC__limit < ${#__bc_msg} )); do
        printf '+ %s %s\n' \
            "$__bc_head" "${__bc_msg:__bc_from:__BC__limit}" >&"$__BC__up" || __BC_THROW
        __bc_from=$(( __bc_from + __BC__limit ))
    done

    printf '. %s %s\n' "$__bc_head" "${__bc_msg:__bc_from}" >&"$__BC__up" || __BC_THROW
}

# The rig's own bash, laid down beside this file by the run. Unguarded: a
# `BASH_ENV` file's status is discarded by bash and errexit does not reach
# here, so nothing a guard returned could be read by anyone.
source "$__BC__DIR/rig.bash"

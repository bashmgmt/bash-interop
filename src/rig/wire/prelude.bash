# The client half of the protocol, sourced through BASH_ENV by every shell in
# the subject's process tree. It is shipped verbatim into the run's workspace
# and finds everything it needs from its own path, so nothing is templated in.

__BC__DIR="${BASH_SOURCE[0]%/*}"
__BC__UP="$__BC__DIR/up"

# Under PIPE_BUF (4096) with room for the frame header and the delimiter, so a
# frame is always one atomic write.
__BC__limit=3900

__BC__owner=""
__BC__parent=""
__BC__up=""
__BC__seq=0
__BC__reply=""
__BC__replyfd=""

BC_INSTR() {
    local __BC__msg

    case "${1-}" in
        say) shift; [[ $BASHPID == "$__BC__owner" ]] || __bc_join; __bc_send SAY "$@" ;;
        ask) shift; __bc_ask "$@" ;;
        *)   return 2 ;;
    esac
}

__bc_ask() {
    [[ $BASHPID == "$__BC__owner" ]] || __bc_join
    [[ -n $__BC__replyfd ]] || {
        [[ -p $__BC__reply ]] || mkfifo "$__BC__reply"
        exec {__BC__replyfd}<>"$__BC__reply"
    }

    __bc_send ASK "$@"

    local __bc_line
    IFS= read -r __bc_line <&"$__BC__replyfd"
    local -a __bc_answer="$__bc_line"
    "${__bc_answer[@]}"
}

# $1 is the kind; the rest is the client's arglist. The protocol's own words
# go in front of it, and the reader shifts exactly those back off.
__bc_send() {
    set -- "$1" "at=$EPOCHREALTIME" "parent=$__BC__parent" "shlvl=$SHLVL" "${@:2}"

    printf -v __BC__msg '%s ' "${@@Q}"
    __BC__msg="(${__BC__msg% })"

    if (( ${#__BC__msg} <= __BC__limit )); then
        printf '. %s %s %s\n' "$BASHPID" "$((__BC__seq++))" "$__BC__msg" >&"$__BC__up"
        return
    fi

    __bc_split
}

# A shell announces nothing: its first message carries seq 0, which is what
# says a shell has joined.
__bc_join() {
    __BC__parent=${__BC__owner:-$PPID}

    exec {__BC__up}>"$__BC__UP"
    __BC__owner=$BASHPID
    __BC__seq=0
    __BC__reply="$__BC__DIR/rep.$BASHPID"
    __BC__replyfd=""
}

__bc_split() {
    local __bc_head="$BASHPID $((__BC__seq++))"
    local __bc_from=0

    while (( __bc_from + __BC__limit < ${#__BC__msg} )); do
        printf '+ %s %s\n' "$__bc_head" "${__BC__msg:__bc_from:__BC__limit}" >&"$__BC__up"
        (( __bc_from += __BC__limit ))
    done

    printf '. %s %s\n' "$__bc_head" "${__BC__msg:__bc_from}" >&"$__BC__up"
}

# The rig's own bash, laid down beside this file. Always written, possibly
# empty, so sourcing it needs no test and leaves $? alone.
source "$__BC__DIR/rig.bash"

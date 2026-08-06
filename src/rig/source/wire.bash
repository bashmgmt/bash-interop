__BC__owner=""
__BC__up=""
__BC__seq=0
__BC__reply=""
__BC__replyfd=""

BC_INSTR() {
    local __BC__msg __BC__at
    [[ -z $__BC__DEBUG ]] || __bc_where

    case "${1-}" in
        say) shift; [[ $BASHPID == "$__BC__owner" ]] || __bc_join; __bc_send "$@" ;;
        ask) shift; __bc_ask "$@" ;;
        *)   return 2 ;;
    esac
}

__bc_where() {
    __BC__at="${BASH_SOURCE[2]:-}:${BASH_LINENO[1]:-0}:${FUNCNAME[2]:-main}"
}

__bc_ask() {
    [[ $BASHPID == "$__BC__owner" ]] || __bc_join
    [[ -n $__BC__replyfd ]] || {
        [[ -p $__BC__reply ]] || mkfifo "$__BC__reply"
        exec {__BC__replyfd}<>"$__BC__reply"
    }

    set -- __ASK__ "$@"
    __bc_send "$@"

    local __bc_line
    IFS= read -r __bc_line <&"$__BC__replyfd"
    local -a __bc_answer="$__bc_line"
    "${__bc_answer[@]}"
}

__bc_send() {
    printf -v __BC__msg '%s ' "${@@Q}"
    __BC__msg="(${__BC__msg% })"
    [[ -z $__BC__DEBUG ]] || __bc_log "${#__BC__msg}" "$__BC__at"

    if (( ${#__BC__msg} <= __BC__limit )); then
        printf '%s %s %s . %s\n' \
            "$EPOCHREALTIME" "$BASHPID" "$((__BC__seq++))" "$__BC__msg" >&"$__BC__up"
        return
    fi

    __bc_split
}

__bc_join() {
    local __bc_parent=${__BC__owner:-$PPID}

    exec {__BC__up}>"$__BC__UP"
    __BC__owner=$BASHPID
    __BC__seq=0
    __BC__reply="$__BC__DIR/rep.$BASHPID"
    __BC__replyfd=""

    __bc_send __ORIGIN__ parent "$__bc_parent" shlvl "$SHLVL" source "${BASH_SOURCE[-1]:-}"
}

__bc_split() {
    local __bc_head="$EPOCHREALTIME $BASHPID $((__BC__seq++))"
    local __bc_from=0

    while (( __bc_from + __BC__limit < ${#__BC__msg} )); do
        printf '%s + %s\n' "$__bc_head" "${__BC__msg:__bc_from:__BC__limit}" >&"$__BC__up"
        (( __bc_from += __BC__limit ))
    done

    printf '%s . %s\n' "$__bc_head" "${__BC__msg:__bc_from}" >&"$__BC__up"
}

__bc_log() {
    printf '%s %s %s\n' "$EPOCHREALTIME" "$BASHPID" "$*" >> "$__BC__DIR/debug.log"
}

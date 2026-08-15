# The client half of the protocol. Sourced by every shell that joins, through
# the session's invocation file — the address. Generic: nothing here reads the
# environment or this file's own location; the session's one coordinate, its
# workspace, is an argument of BC_JOIN.
#
# One entry per label. `declare -gA` on an array that exists keeps it, so a
# second session's prelude sourced into the same shell adds a label and takes
# nothing away.
declare -gA __BC__DIR __BC__META __BC__FD __BC__REP __BC__OWNER

# `BC_INSTR` could not do its job — as distinct from an answer that ran and
# returned non-zero, which is the subject's business. 125 is what `env` and
# `timeout` return when the wrapper rather than the payload failed.
__BC__FAILED=125
__BC__at=""

# `return` must act in the frame that failed, so the guards are aliases and
# `expand_aliases` is on before anything using them is parsed. `$?` is read in
# the first command of each, the only place it survives.
shopt -s expand_aliases

alias __BC_BAIL='return $?'
alias __BC_THROW='{ __bc_complain "${FUNCNAME[0]} ($?)"; return "$__BC__FAILED"; }'

# One line per fault, naming the subject's call site.
__bc_complain() {
    printf 'BC_INSTR: %s at %s\n' "$1" "${__BC__at:-?}" >&2
}

# $1 the label, $2 the session's workspace, the rest words of the caller's
# own. Binds the name to the coordinate for this shell, keeps the words, and
# attaches this process; a fork inherits the entries and attaches itself on
# its first word, announcing the same words.
BC_JOIN() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"

    [[ -n ${1-} && $1 != */* && $1 != *[[:space:]]* ]] \
        || { __bc_complain "label ${1-} will not name a file"; return "$__BC__FAILED"; }
    [[ ${2-} == /* ]] \
        || { __bc_complain "workspace ${2-} is not an absolute path"; return "$__BC__FAILED"; }
    [[ -z ${__BC__DIR[$1]-} ]] \
        || { __bc_complain "label $1 is already joined from ${__BC__DIR[$1]}"; return "$__BC__FAILED"; }

    __BC__DIR[$1]=$2
    local __bc_label=$1 IFS=' '
    shift 2
    __BC__META[$__bc_label]="${*@Q}"
    __bc_attach "$__bc_label"
}

# $1 the label, $2 the verb, the rest the client's words.
BC_INSTR() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"

    [[ -n ${__BC__DIR[${1-}]-} ]] \
        || { __bc_complain "label ${1-} is not joined"; return "$__BC__FAILED"; }
    [[ $BASHPID == "${__BC__OWNER[$1]}" ]] || __bc_reattach "$1" || __BC_BAIL

    case "${2-}" in
        say) __bc_send "$1" SAY "${@:3}" ;;
        ask) __bc_ask "$1" "${@:3}" ;;
        *)   __bc_complain "unknown verb ${2-}"; return "$__BC__FAILED" ;;
    esac
}

# $1 the label. This process's pipe: make it, announce it with the account,
# open it — the open completes when the run has it in hand, and the run makes
# the reply pipe before that, so both exist by the time this returns.
#
# `>` would create a regular file where no fifo is, so the control fifo is
# checked before it is written: a session that closed unlinked it.
__bc_attach() {
    local __bc_dir=${__BC__DIR[$1]}
    local __bc_tok="$1::$BASHPID.${EPOCHREALTIME#*[.,]}.${SRANDOM:-$RANDOM$RANDOM}"
    local __bc_fd __bc_rep __bc_acct

    [[ -p "$__bc_dir/join" ]] || { __bc_complain "no session at $__bc_dir"; return "$__BC__FAILED"; }
    __bc_account __bc_acct "$1"
    mkfifo "$__bc_dir/up.$__bc_tok"                                 || __BC_THROW
    __bc_announce "$__bc_tok" "$__bc_acct" >"$__bc_dir/join"        || __BC_BAIL
    exec {__bc_fd}>"$__bc_dir/up.$__bc_tok"                         || __BC_THROW
    exec {__bc_rep}<>"$__bc_dir/rep.$__bc_tok"                      || __BC_THROW

    __BC__FD[$1]=$__bc_fd
    __BC__REP[$1]=$__bc_rep
    __BC__OWNER[$1]=$BASHPID
}

# $1 the label. A fork inherited its parent's descriptors; it drops them and
# takes its own, so a parent's pipe is held only by processes that could write
# on it.
__bc_reattach() {
    local __bc_fd=${__BC__FD[$1]} __bc_rep=${__BC__REP[$1]}
    exec {__bc_fd}>&- {__bc_rep}>&- || __BC_THROW
    __bc_attach "$1"
}

# $1 the name to write into, $2 the label: this shell's account of itself,
# one array literal, the clock first. Every value is passed as bash reports
# it; what any of it means is read on the other side. The words the join
# brought ride as one nested literal, the shape `versinfo` takes. `IFS` is
# local so `[*]` joins with a space whatever the subject's is, and the
# subject's — unset included — is back on return.
__bc_account() {
    local __bc_out=$1 IFS=' '
    local -a __bc_meta="(${__BC__META[$2]-})"
    set -- "at=$EPOCHREALTIME" \
        pid       "$BASHPID" \
        shlvl     "$SHLVL" \
        subshell  "$BASH_SUBSHELL" \
        versinfo  "(${BASH_VERSINFO[*]@Q})" \
        bash      "$BASH" \
        zero      "$0" \
        flags     "$-" \
        shellopts "$SHELLOPTS" \
        bashopts  "$BASHOPTS" \
        command   "${BASH_EXECUTION_STRING-}" \
        brought   "(${__bc_meta[*]@Q})"
    printf -v "$__bc_out" '(%s)' "${*@Q}"
}

# $1 the token, $2 the account; standard output is the control fifo. Every
# shell writes there and a write is atomic up to PIPE_BUF, so the account goes
# in frames that each fit it whole: `<token> + <bytes>` for one with more to
# come, `<token> . <bytes>` for the last. Under `LC_ALL=C` `${#2}` and
# `${2:a:b}` count bytes; `local` puts the subject's locale back on return.
__bc_announce() {
    local LC_ALL=C
    local __bc_room=$(( 4096 - ${#1} - 4 )) __bc_from=0
    while (( ${#2} - __bc_from > __bc_room )); do
        printf '%s + %s\n' "$1" "${2:__bc_from:__bc_room}" || __BC_THROW
        __bc_from=$(( __bc_from + __bc_room ))
    done
    printf '%s . %s\n' "$1" "${2:__bc_from}" || __BC_THROW
}

# $1 the label, $2 the verb, the rest the words. One line, one printf: the
# pipe has one writer, so the size of the line does not matter.
__bc_send() {
    local IFS=' ' __bc_fd=${__BC__FD[$1]}
    set -- "$2" "at=$EPOCHREALTIME" "${@:3}"
    printf '(%s)\n' "${*@Q}" >&"$__bc_fd" || __BC_THROW
}

# $1 the label, the rest the question. The reply pipe was opened at attach and
# is read-write, so the read waits for an answer rather than seeing end of
# input. The answer is one command, as a bash array literal; running it is the
# result the caller gets.
__bc_ask() {
    __bc_send "$1" ASK "${@:2}" || __BC_BAIL

    local __bc_line
    IFS= read -r __bc_line <&"${__BC__REP[$1]}" || __BC_THROW

    local -a __bc_answer="$__bc_line"
    "${__bc_answer[@]}"
}

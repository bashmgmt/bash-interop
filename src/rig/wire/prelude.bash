# The client half of the protocol. Sourced by every shell that joins, through
# the session's invocation file — the address. Generic: nothing here reads the
# environment or this file's own location; the session's one coordinate, its
# workspace, is an argument of BC_JOIN.
#
# One entry per label. `declare -gA` on an array that exists keeps it, so a
# second session's prelude sourced into the same shell adds a label and takes
# nothing away.
declare -gA __BC__DIR __BC__META __BC__FD __BC__REP __BC__OWNER

# The answer to the ask in flight, read by BC_ASK in the frame that asked.
declare -ga __BC__ANSWER=()

# A word could not do its job — as distinct from an answer that ran and
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

# `__bc_l` names a session this process holds open. A fork inherited the
# entries without the descriptors and takes its own here.
alias __BC_REACH='
    [[ -n ${__BC__DIR[$__bc_l]-} ]] \
        || { __bc_complain "label $__bc_l is not joined"; return "$__BC__FAILED"; }
    [[ $BASHPID == "${__BC__OWNER[$__bc_l]}" ]] || __bc_reattach "$__bc_l" || __BC_BAIL'

# The one shape a message takes: the positional parameters, quoted as bash
# writes them, on this label's pipe. One writer per pipe, so the length of the
# line does not matter.
alias __BC_WRITE='printf "(%s)\n" "${*@Q}" >&"${__BC__FD[$__bc_l]}" || __BC_THROW'

# One line per fault, naming the word the subject called and its call site.
__bc_complain() {
    printf '%s: %s at %s\n' "${__BC__word:-?}" "$1" "${__BC__at:-?}" >&2
}

# $1 the label, $2 the session's workspace, the rest words of the caller's
# own. Binds the name to the coordinate for this shell, keeps the words, and
# attaches this process; a fork inherits the entries and attaches itself on
# its first word, announcing the same words.
BC_JOIN() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"
    __BC__word=${FUNCNAME[0]}

    [[ -n ${1-} && $1 != */* && $1 != *[[:space:]]* ]] \
        || { __bc_complain "label ${1-} will not name a file"; return "$__BC__FAILED"; }
    [[ ${2-} == /* ]] \
        || { __bc_complain "workspace ${2-} is not an absolute path"; return "$__BC__FAILED"; }
    [[ -z ${__BC__DIR[$1]-} ]] \
        || { __bc_complain "label $1 is already joined from ${__BC__DIR[$1]}"; return "$__BC__FAILED"; }

    __BC__DIR[$1]=$2
    declare __bc_label=$1 IFS=' '
    shift 2 || __BC_THROW
    __BC__META[$__bc_label]="${*@Q}"
    __bc_attach "$__bc_label"
}

# Ship the words and carry on. The channel is named beside the call, and the
# words ride on the right:
#
#     declare -- BC_SAY__ARG_LABEL=DEPLOY
#     BC_SAY STAGE compile
#
# A rig's own saying word is one command over this, so it composes where any
# command does:
#
#     alias STAGE='BC_SAY__ARG_LABEL=DEPLOY BC_SAY STAGE'
__bc_say() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"
    __BC__word=BC_SAY

    declare __bc_l=${BC_SAY__ARG_LABEL:?BC_SAY__ARG_LABEL}
    __BC_REACH

    declare IFS=' '
    set -- SAY "at=$EPOCHREALTIME" "$@"
    __BC_WRITE
}
alias BC_SAY='__bc_say'

# Ship the words, block, and leave the reply in __BC__ANSWER. BC_ASK runs it
# in the frame that asked, so an answer may declare there.
#
#     declare -- BC_ASK__ARG_LABEL=DEPLOY
#     declare -a BC_ASK__ARGS=(which-target)
#     BC_ASK
#
# A fault leaves __bc_no_answer standing, so a shell without `errexit` reports
# it rather than running an answer meant for an earlier question. The reply
# pipe was opened read-write at attach, so the read waits for an answer rather
# than seeing end of input.
__bc_ask() {
    __BC__at="${BASH_SOURCE[1]:-?}:${BASH_LINENO[0]:-?}"
    __BC__word=BC_ASK
    __BC__ANSWER=(__bc_no_answer)

    declare __bc_l=${BC_ASK__ARG_LABEL:?BC_ASK__ARG_LABEL}
    __BC_REACH

    declare IFS=' '
    set -- ASK "at=$EPOCHREALTIME" "${BC_ASK__ARGS[@]}"
    __BC_WRITE

    declare __bc_line
    IFS= read -r __bc_line <&"${__BC__REP[$__bc_l]}" || __BC_THROW

    declare -ga __BC__ANSWER="$__bc_line"
}
alias BC_ASK='__bc_ask; "${__BC__ANSWER[@]}"'

# What an ask runs when no answer arrived.
__bc_no_answer() { return "$__BC__FAILED"; }

# A status, and nothing else. `return` in an answer acts on the frame that
# asked, so a rig meaning only "no" answers with this instead.
__bc_status() { return "${1:?a status}"; }

# $1 the label. This process's pipe: make it, announce it with the account,
# open it — the open completes when the run has it in hand, and the run makes
# the reply pipe before that, so both exist by the time this returns.
#
# `>` would create a regular file where no fifo is, so the control fifo is
# checked before it is written: a session that closed unlinked it.
__bc_attach() {
    declare __bc_dir=${__BC__DIR[$1]}
    declare __bc_tok="$1::$BASHPID.${EPOCHREALTIME#*[.,]}.${SRANDOM:-$RANDOM$RANDOM}"
    declare __bc_fd __bc_rep __bc_acct

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
    declare __bc_fd=${__BC__FD[$1]} __bc_rep=${__BC__REP[$1]}
    exec {__bc_fd}>&- {__bc_rep}>&- || __BC_THROW
    __bc_attach "$1"
}

# $1 the name to write into, $2 the label: this shell's account of itself,
# one array literal, the clock first. Every value is passed as bash reports
# it; what any of it means is read on the other side. The words the join
# brought ride as one nested literal, the shape `versinfo` takes. `IFS` is
# scoped to this frame so `[*]` joins with a space whatever the subject's is,
# and the subject's — unset included — is back on return.
__bc_account() {
    declare __bc_out=$1 IFS=' '
    declare -a __bc_meta="(${__BC__META[$2]-})"
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
# `${2:a:b}` count bytes, and the subject's locale is back on return.
__bc_announce() {
    declare LC_ALL=C
    declare __bc_room=$(( 4096 - ${#1} - 4 )) __bc_from=0
    while (( ${#2} - __bc_from > __bc_room )); do
        printf '%s + %s\n' "$1" "${2:__bc_from:__bc_room}" || __BC_THROW
        __bc_from=$(( __bc_from + __bc_room ))
    done
    printf '%s . %s\n' "$1" "${2:__bc_from}" || __BC_THROW
}

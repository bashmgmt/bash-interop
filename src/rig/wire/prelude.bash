# The client half of the protocol, sourced through BASH_ENV by every shell in
# the subject's process tree. It is shipped verbatim into the run's workspace
# and finds everything it needs from its own path, so nothing is templated in.

__BC__DIR="${BASH_SOURCE[0]%/*}"
__BC__UP="$__BC__DIR/up"

# One frame is one atomic write, so it has to fit PIPE_BUF whole, header and
# delimiter included. Bash measures and slices text in characters and PIPE_BUF
# counts bytes: `__BC__narrow` is the width at which that cannot matter, a
# character being at most four bytes in every locale glibc ships and the
# header and delimiter under 24. Anything wider goes through `__bc_frame`,
# which works in bytes and fills the frame.
__BC__PIPE_BUF=4096
__BC__narrow=$(( (__BC__PIPE_BUF - 24) / 4 ))

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
             __bc_send SAY "$@" || __BC_BAIL ;;
        ask) shift; __bc_ask "$@" ;;
        *)   __bc_complain "unknown verb ${1-}"
             return "$__BC__FAILED" ;;
    esac
}

# The shell's reply pipe, made for one question and removed by the run as it
# answers — a shell is blocked while it asks, so the name is free again before
# the next one. Read-write, so the read waits for an answer rather than seeing
# end of input.
#
# The array assignment is unguarded because it cannot fail: text bash cannot
# read as a literal becomes one element, and running that is a command not
# found. So is the answer's own status, which is the result the caller wants.
__bc_ask() {
    { [[ $BASHPID == "$__BC__owner" ]] || __bc_join; } || __BC_BAIL

    local __bc_reply="$__BC__DIR/rep.$BASHPID"
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

# $1 is the kind, the rest the client's arglist. The protocol's own words go
# in front of it, and the reader shifts exactly those back off. The sequence
# number is spent once here, whichever of the two paths below takes it.
__bc_send() {
    local __bc_seq=$(( __BC__seq++ ))
    set -- "$1" "at=$EPOCHREALTIME" "parent=$__BC__parent" "shlvl=$SHLVL" "${@:2}"

    local __bc_msg
    printf -v __bc_msg '%s ' "${@@Q}"
    __bc_msg="(${__bc_msg% })"

    if (( ${#__bc_msg} <= __BC__narrow )); then
        printf '. %s %s %s\n' \
            "$BASHPID" "$__bc_seq" "$__bc_msg" >&"$__BC__up" || __BC_THROW
        return 0
    fi

    LC_ALL=C __bc_frame "$__bc_seq" "$__bc_msg" || __BC_BAIL
}

# A shell announces nothing: its first message carries seq 0, which is what
# says a shell has joined.
__bc_join() {
    __BC__parent=${__BC__owner:-$PPID}

    exec {__BC__up}>"$__BC__UP" || __BC_THROW
    __BC__owner=$BASHPID
    __BC__seq=0
}

# $1 the sequence number, $2 a message the narrow lane would not take. Every
# chunk repeats `pid seq`, the key the reader rejoins them by.
#
# `LC_ALL=C` rides on the call, which scopes it to this frame and gives the
# subject's back — including one that was unset — without a name to restore. It
# is what makes `${#…}` and `${…:from:room}` count the bytes PIPE_BUF counts;
# the message itself was quoted by the caller, in the subject's own locale, so
# what goes on the wire is unchanged. Taking it costs about 7 µs, which is why
# the narrow lane exists and why it is measured in characters.
#
# A cut may fall inside a character. The reader joins chunks as bytes and
# decodes the message once, for the same reason it buffers reads as bytes.
#
# The cursor moves by assignment: `(( x += n ))` is a command, and its status
# is false when the result is 0.
__bc_frame() {
    local __bc_head="$BASHPID $1"
    local __bc_msg="$2"
    local __bc_size=${#__bc_msg}
    local __bc_room=$(( __BC__PIPE_BUF - ${#__bc_head} - 4 ))

    if (( __bc_size <= __bc_room )); then
        printf '. %s %s\n' "$__bc_head" "$__bc_msg" >&"$__BC__up" || __BC_THROW
        return 0
    fi

    local __bc_from=0
    while (( __bc_from + __bc_room < __bc_size )); do
        printf '+ %s %s\n' \
            "$__bc_head" "${__bc_msg:__bc_from:__bc_room}" >&"$__BC__up" || __BC_THROW
        __bc_from=$(( __bc_from + __bc_room ))
    done

    printf '. %s %s\n' "$__bc_head" "${__bc_msg:__bc_from}" >&"$__BC__up" || __BC_THROW
}

# The rig's own bash, laid down beside this file by the run. Unguarded: a
# `BASH_ENV` file's status is discarded by bash and errexit does not reach
# here, so nothing a guard returned could be read by anyone.
source "$__BC__DIR/rig.bash"

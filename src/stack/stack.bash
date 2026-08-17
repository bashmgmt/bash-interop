# The current call stack, in the sections `bash::stack` reads back.
#
# $1 names an array to append to; $2 is how many leading frames belong to the
# instrument rather than the subject, counting this function's own.
#
# Bash's five arrays go out as they are. Every index — which frames belong to
# the instrument, which line a frame is executing, where a call's arguments sit
# in the flat stack and which way round they are — is arithmetic, and belongs
# on the side that can be checked without running anything.
#
# `$PWD` goes with them because `BASH_SOURCE` holds the path as it was written,
# relative or not, and nothing else records what it was relative to. It changes
# under the subject's feet, which is why it is here and not in what the shell
# said of itself when it joined.
#
# `BASH_ARGC` and `BASH_ARGV` are empty unless the shell is under `extdebug`.
# Expanding an unset array is not an error, including under `set -u`, and the
# reader decides what a short one means.
__bc_stack() {
    declare -n __bc_stack_out="$1"

    __bc_stack_out+=(
        skip    "$2"
        pwd     "$PWD"
        funcs   "(${FUNCNAME[*]@Q})"
        sources "(${BASH_SOURCE[*]@Q})"
        lines   "(${BASH_LINENO[*]@Q})"
        argc    "(${BASH_ARGC[*]@Q})"
        argv    "(${BASH_ARGV[*]@Q})"
    )
}

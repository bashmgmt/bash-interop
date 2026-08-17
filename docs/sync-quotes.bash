#!/usr/bin/env bash
# Keeps the book's Rust quotes true to the tree. A quoted block is declared
#
#     <!-- quote: <path-from-crate-root> anchor=<name> -->
#     ```rust
#     …
#     ```
#
# and its body is the region between `// ANCHOR: <name>` and
# `// ANCHOR_END: <name>` in that file, marker lines excluded, common indent
# stripped. Rust only: marker comments never leave the source tree, where a
# marker in shipped bash would ride into every laid workspace file — bash
# blocks are hand copies by design.
#
#     docs/sync-quotes.bash          rewrite every declared quote in place
#     docs/sync-quotes.bash --check  fail if any quote differs from its source
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

declare -- mode="${1:-sync}"
declare -i errs=0

# The region between the markers. A marker pair that is not exactly one
# start and one end is refused: a missing end would silently quote to the
# end of the file, a duplicate name would silently concatenate regions.
region() {
    declare -- file="${1:?the source file}" anchor="${2:?the anchor name}"
    declare -i starts ends
    starts=$(grep -c "ANCHOR: $anchor\$" "$file" || true)
    ends=$(grep -c "ANCHOR_END: $anchor\$" "$file" || true)
    (( starts == 1 && ends == 1 )) \
        || { echo "ERROR [$file]: anchor $anchor: $starts starts, $ends ends" >&2; exit 1; }
    awk -v a="$anchor" '
        $0 ~ ("ANCHOR_END: " a "$") { on = 0 }
        on { print }
        $0 ~ ("ANCHOR: " a "$") { on = 1 }
    ' "$file" | sed 's/[[:space:]]*$//' | awk '
        { lines[NR] = $0
          if ($0 != "") { n = match($0, /[^ ]/) - 1; if (min == "" || n < min) min = n } }
        END { for (i = 1; i <= NR; i++) print substr(lines[i], min + 1) }'
}

# One pass over one document: report the first stale quote (and fix it,
# unless checking). Returns 0 when every quote is current.
pass() {
    declare -- doc="${1:?the document}"
    while IFS=: read -r nr _; do
        declare -- decl file anchor lang
        decl="$(sed -n "${nr}p" "$doc")"
        file="$(sed 's/.*quote: \([^ ]*\).*/\1/' <<<"$decl")"
        anchor="$(sed 's/.*anchor=\([^ ]*\) .*/\1/' <<<"$decl")"
        [[ -f $file ]] || { echo "ERROR [$doc:$nr]: no such file: $file" >&2; errs+=1; continue; }

        declare -i fence_open fence_close
        fence_open=$((nr + 1))
        lang="$(sed -n "${fence_open}p" "$doc")"
        [[ $lang == '```'* ]] || { echo "ERROR [$doc:$nr]: no fence after quote declaration" >&2; errs+=1; continue; }
        fence_close="$(awk -v s="$fence_open" 'NR > s && /^```$/ { print NR; exit }' "$doc")"

        declare -- want have
        want="$(region "$file" "$anchor")"
        have="$(sed -n "$((fence_open + 1)),$((fence_close - 1))p" "$doc")"
        if [[ "$want" != "$have" ]]; then
            if [[ $mode == --check ]]; then
                echo "STALE [$doc:$nr]: quote of $file:$anchor differs" >&2; errs+=1
            else
                { sed -n "1,${fence_open}p" "$doc"
                  printf '%s\n' "$want"
                  sed -n "${fence_close},\$p" "$doc"; } > "$doc.tmp"
                mv "$doc.tmp" "$doc"
                echo "synced [$doc:$nr] from $file:$anchor"
                return 1    # line numbers moved: the caller starts this doc over
            fi
        fi
    done < <(grep -n '<!-- quote: ' "$doc" || true)
    return 0
}

for doc in docs/*.md; do
    until pass "$doc"; do :; done
done

(( errs == 0 )) || { echo "$errs quote error(s)" >&2; exit 1; }

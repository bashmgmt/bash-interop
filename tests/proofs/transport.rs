//! Every shell reaches the wire, and what it writes arrives whole.

use mb_resolver::bash::rig::{forest, shells, ExitStatus, ShellNode};

use crate::{behind, heard, report, script, ENTRY};

/// Nothing is inherited, and the forest follows the emitting parent — which
/// `$PPID` would get wrong inside a subshell.
#[test]
fn every_descendant_shell_reaches_the_wire() {
    let (seen, _) = heard(&[
        (
            ENTRY,
            r#"
            BC_INSTR say REC top
            ( BC_INSTR say REC paren )
            value=$( BC_INSTR say REC cmdsubst; echo hi )
            bash "${BASH_SOURCE[0]%/*}/child.bash"
            BC_INSTR say REC after
            "#,
        ),
        ("child.bash", "BC_INSTR say REC child\n( BC_INSTR say REC grandchild )\n"),
    ]);

    assert_eq!(
        behind(&seen, "REC"),
        [["top"], ["paren"], ["cmdsubst"], ["child"], ["grandchild"], ["after"]],
        "{}",
        report(&seen)
    );

    let forest = forest(&shells(&seen));
    assert_eq!(forest.len(), 1, "one root: nothing is orphaned{}", report(&seen));

    let tree = descend(&forest, None);
    assert_eq!(tree.len(), 5, "the shell, two subshells, the child, its subshell");
    assert_eq!(tree.iter().map(|(shell, _)| shell.depth).max(), Some(3), "main, child, grandchild");
    assert!(
        tree.iter().all(|(shell, above)| above.is_none_or(|up| shell.shlvl >= up.shlvl)),
        "SHLVL never drops toward a descendant{}",
        report(&seen)
    );
}

#[derive(Copy, Clone)]
struct Descendant {
    depth: usize,
    shlvl: u32,
}

/// Every shell under `nodes`, with the one that started it where there is one.
fn descend(
    nodes: &[ShellNode<'_>],
    above: Option<Descendant>,
) -> Vec<(Descendant, Option<Descendant>)> {
    nodes
        .iter()
        .flat_map(|node| {
            let shlvl = node.shell.shlvl;
            let shell = Descendant { depth: above.map_or(1, |up| up.depth + 1), shlvl };

            std::iter::once((shell, above)).chain(descend(&node.children, Some(shell)))
        })
        .collect()
}

/// One pipe, many writers: frames stay under `PIPE_BUF` so they cannot
/// interleave, and anything longer is split and rejoined by `(pid, seq)`.
#[test]
fn concurrent_writers_never_interleave() {
    let (seen, _) = heard(&[
        (
            ENTRY,
            r#"
            here="${BASH_SOURCE[0]%/*}"
            for name in a b c d e f g h; do bash "$here/child.bash" "$name" & done
            wait
            "#,
        ),
        (
            "child.bash",
            r#"
            small="$(printf 'S%.0s' {1..500})"
            large="$(printf 'L%.0s' {1..9000})"
            for index in $(seq 1 40); do
                BC_INSTR say REC "$1-$index-$small"
                BC_INSTR say REC "$1-$index-$large"
            done
            "#,
        ),
    ]);

    let records = behind(&seen, "REC");
    assert_eq!(records.len(), 8 * 80);
    assert_eq!(
        records.iter().filter(|words| words[0].len() > 9000).count(),
        8 * 40,
        "oversized messages rejoined intact"
    );
}

/// Messages written immediately before the last writer exits must still be
/// readable once the subject is gone.
#[test]
fn nothing_is_lost_at_the_end() {
    for _ in 0..10 {
        let (seen, status) =
            script("for i in $(seq 1 200); do BC_INSTR say REC \"r$i\"; done\nexit 3");
        assert_eq!(behind(&seen, "REC").len(), 200);
        assert_eq!(status, ExitStatus::Code(3));
    }
}

/// The delimiter separates frames and is part of none of them, so a value
/// carrying one arrives whole — and as one word, beside the next.
#[test]
fn a_newline_inside_a_value_is_escaped_not_framed() {
    let (seen, _) =
        script("payload=$'first\\nsecond\\tthird\\\\fourth'\nBC_INSTR say REC \"$payload\" plain\n");

    assert_eq!(
        behind(&seen, "REC"),
        [["first\nsecond\tthird\\fourth", "plain"]],
        "{}",
        report(&seen)
    );
    assert_eq!(seen.len(), 1, "one message, not two frames of nonsense");
}

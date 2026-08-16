//! Every shell reaches the run, and what it writes arrives whole.

use bash_interop::rig::ExitStatus;

use crate::{behind, report, running, script, ENTRY};

/// Every kind of descendant is a shell of its own: a subshell, a command
/// substitution, a child process, and its subshell.
#[tokio::test]
async fn every_descendant_shell_reaches_the_run() {
    let ran = running(&[
        (
            ENTRY,
            r#"
            BC_INSTR KEEP say REC top
            ( BC_INSTR KEEP say REC paren )
            value=$( BC_INSTR KEEP say REC cmdsubst; echo hi )
            bash "${BASH_SOURCE[0]%/*}/child.bash"
            BC_INSTR KEEP say REC after
            "#,
        ),
        (
            "child.bash",
            r#"
            BC_INSTR KEEP say REC child
            ( BC_INSTR KEEP say REC grandchild )
            "#,
        ),
    ])
    .await;

    assert_eq!(
        behind(&ran.shells, "REC"),
        [["top"], ["paren"], ["cmdsubst"], ["child"], ["grandchild"], ["after"]],
        "{}",
        report(&ran.shells)
    );
    assert_eq!(ran.shells.len(), 5, "the shell, two subshells, the child, its subshell");

    let shlvl: Vec<u32> = ran.shells.iter().map(|at| at.shell.shlvl).collect();
    assert!(shlvl[3] > shlvl[0], "the child is one level down: {shlvl:?}");
    assert_eq!(shlvl[1], shlvl[0], "a subshell is not: {shlvl:?}");
}

/// Many shells at once, each on a pipe of its own, each writing lines far
/// wider than a pipe's atomic write. Nothing crosses.
#[tokio::test]
async fn many_shells_at_once_arrive_whole_and_apart() {
    let ran = running(&[
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
                BC_INSTR KEEP say REC "$1" "$index" "$small"
                BC_INSTR KEEP say REC "$1" "$index" "$large"
            done
            "#,
        ),
    ])
    .await;

    let records = behind(&ran.shells, "REC");
    assert_eq!(records.len(), 8 * 80);
    assert_eq!(records.iter().filter(|words| words[2].len() == 9000).count(), 8 * 40);

    for at in &ran.shells[1..] {
        let names: std::collections::HashSet<&str> =
            at.kept.iter().map(|message| message.words[1].as_str()).collect();
        assert_eq!(names.len(), 1, "one shell's pipe carries one shell's words: {names:?}");
    }
}

/// A line is text and a pipe hands it over in bytes: a message of wide
/// characters, longer than the pipe's atomic write, comes back character for
/// character.
#[tokio::test]
async fn a_message_of_wide_characters_arrives_whole() {
    let ran = script(
        r#"
        wide="$(printf '€%.0s' {1..6000})"
        for name in a b c d; do ( BC_INSTR KEEP say REC "$name" "$wide" ) & done
        wait
        "#,
    )
    .await;

    let records = behind(&ran.shells, "REC");
    assert_eq!(records.len(), 4, "one per writer{}", report(&ran.shells));

    for words in &records {
        assert_eq!(words[1].chars().count(), 6000, "every character back, and only those");
        assert!(words[1].chars().all(|glyph| glyph == '€'), "and each of them itself");
    }
}

/// Messages written immediately before the subject exits are read after it is
/// gone: the group is killed first, and every pipe is read to its end.
#[tokio::test]
async fn nothing_is_lost_at_the_end() {
    for _ in 0..10 {
        let ran = script(
            r#"
            for i in $(seq 1 200); do BC_INSTR KEEP say REC "r$i"; done
            exit 3
            "#,
        )
        .await;
        assert_eq!(behind(&ran.shells, "REC").len(), 200);
        assert_eq!(ran.subject, ExitStatus::Code(3));
    }
}

/// The delimiter separates lines and is part of none of them, so a value
/// carrying one arrives whole — and as one word, beside the next.
#[tokio::test]
async fn a_newline_inside_a_value_is_escaped_not_a_line() {
    let ran = script(
        r#"
        payload=$'first\nsecond\tthird\\fourth'
        BC_INSTR KEEP say REC "$payload" plain
        "#,
    )
    .await;

    assert_eq!(
        behind(&ran.shells, "REC"),
        [["first\nsecond\tthird\\fourth", "plain"]],
        "{}",
        report(&ran.shells)
    );
}

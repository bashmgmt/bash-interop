//! What the subject keeps: its own exit status, its own trap, its own `IFS`.

use mb_resolver::bash::rig::ExitStatus;

use crate::{behind, report, script};

/// Reported as signalled rather than flattened into a code, and nothing said
/// before the signal is lost. The rig installs no handler.
#[test]
fn a_signalled_subject_is_reported_and_loses_nothing() {
    let ran = script(
        r#"
        BC_INSTR say REC before
        kill -TERM $$
        BC_INSTR say REC never
        "#,
    );

    assert_eq!(ran.subject, ExitStatus::Signal(15));
    assert_eq!(ran.subject.shell_code(), 143, "128 + signal, the shell convention");
    assert_eq!(behind(&ran.shells, "REC"), [["before"]], "{}", report(&ran.shells));
}

/// The subject keeps its own trap and its own `IFS`. The prelude installs no
/// handler and shadows no builtin, so both survive a message going out — and
/// the account a shell gives of itself, which joins an array with `[*]`, takes
/// an `IFS` of its own for that frame rather than the subject's.
#[test]
fn a_clients_own_trap_and_ifs_are_untouched() {
    let ran = script(
        r#"
        trap 'echo mine' EXIT
        IFS=,
        BC_INSTR say REC one two
        "#,
    );

    assert_eq!(behind(&ran.shells, "REC"), [["one", "two"]], "{}", report(&ran.shells));
    assert!(
        ran.shells[0].shell.bash.version.at_least(4, 4, 0),
        "the version is an array, and it read back{}",
        report(&ran.shells)
    );
}

/// A message wider than the narrow lane is framed in bytes, which takes
/// `LC_ALL` for as long as that frame lasts. It is a `local`, so it is gone
/// before the send returns — and the subject, which runs everything of its own
/// between sends, never sees it.
///
/// Observed rather than asserted about: `${#text}` counts characters in a
/// UTF-8 locale and bytes in `C`, so the subject can say which one it is in.
#[test]
fn a_clients_own_locale_is_untouched_by_a_wide_message() {
    let ran = script(
        r#"
        export LC_ALL=C.UTF-8
        wide="ä"

        BC_INSTR say REC before "${#wide}" "$LC_ALL"
        BC_INSTR say REC "$(printf 'x%.0s' {1..9000})"
        BC_INSTR say REC after "${#wide}" "$LC_ALL"
        "#,
    );

    let said = behind(&ran.shells, "REC");
    assert_eq!(said[0], ["before", "1", "C.UTF-8"], "{}", report(&ran.shells));
    assert_eq!(said[2], ["after", "1", "C.UTF-8"], "{}", report(&ran.shells));
}

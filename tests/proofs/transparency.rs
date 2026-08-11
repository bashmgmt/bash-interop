//! What the subject keeps: its own exit status, its own trap, its own `IFS`.

use mb_resolver::bash::rig::ExitStatus;

use crate::{behind, report, script};

/// Reported as signalled rather than flattened into a code, and nothing said
/// before the signal is lost. The rig installs no handler.
#[test]
fn a_signalled_subject_is_reported_and_loses_nothing() {
    let (seen, status) = script(
        r#"
        BC_INSTR say REC before
        kill -TERM $$
        BC_INSTR say REC never
        "#,
    );

    assert_eq!(status, ExitStatus::Signal(15));
    assert_eq!(status.shell_code(), 143, "128 + signal, the shell convention");
    assert_eq!(behind(&seen, "REC"), [["before"]], "{}", report(&seen));
}

/// The subject keeps its own trap and its own `IFS`. The prelude installs no
/// handler and shadows no builtin, so both survive a message going out.
#[test]
fn a_clients_own_trap_and_ifs_are_untouched() {
    let (seen, _) = script(
        r#"
        trap 'echo mine' EXIT
        IFS=,
        BC_INSTR say REC one two
        "#,
    );

    assert_eq!(behind(&seen, "REC"), [["one", "two"]], "{}", report(&seen));
}

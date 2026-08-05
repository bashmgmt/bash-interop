//! Bash-level proofs. Each spawns real bash and covers one mechanism that
//! cannot be checked by reading generated source.
//!
//! Nothing here injects any bash of its own: a fixture says what it has to say
//! through `BC_INSTR say`, which is the whole client surface.

use std::fs;

use super::{Behaviour, Capture, ExitStatus, Rig};

struct Run {
    capture: Capture,
    status: ExitStatus,
    debug: Vec<String>,
    _temp: tempfile::TempDir,
}

impl Run {
    /// The words behind `lead` in every message that begins with it, in global
    /// time order.
    fn args(&self, lead: &str) -> Vec<String> {
        self.capture
            .chronological()
            .into_iter()
            .filter_map(|line| line.value.behind(lead))
            .map(|rest| rest.join(" "))
            .collect()
    }

    fn report(&self) -> String {
        let lines: Vec<String> = self
            .capture
            .chronological()
            .into_iter()
            .map(|line| format!("  {} {}", line.stamp.pid, line.value.words.join(" ")))
            .collect();
        format!("capture:\n{}\ndebug:\n  {}", lines.join("\n"), self.debug.join("\n  "))
    }
}

fn run(files: &[(&str, &str)]) -> Run {
    let temp = tempfile::tempdir().unwrap();
    for (name, body) in files {
        fs::write(temp.path().join(name), body).unwrap();
    }
    let entry = temp.path().join(files[0].0).to_string_lossy().into_owned();
    let outcome = Rig::new(Behaviour::new()).debug(true).run(&[entry]).unwrap();
    let run =
        Run { capture: outcome.capture, status: outcome.status, debug: outcome.debug, _temp: temp };
    assert!(run.capture.damage.is_empty(), "{}", run.report());
    run
}

fn script(body: &str) -> Run {
    run(&[("main.bash", body)])
}

/// Each shell joins the pipe by name, so subshells, command substitutions and
/// child processes all reach it with nothing inherited. The forest follows the
/// emitting parent, which `$PPID` would get wrong inside a subshell.
#[test]
fn every_descendant_shell_reaches_the_wire() {
    let result = run(&[
        (
            "main.bash",
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
        result.args("REC"),
        ["top", "paren", "cmdsubst", "child", "grandchild", "after"],
        "{}",
        result.report()
    );
    assert_eq!(result.capture.forest().len(), 1, "{}", result.report());
}

/// One pipe, many writers: frames stay under `PIPE_BUF` so they cannot
/// interleave, and anything longer is split and rejoined by `(pid, seq)`.
#[test]
fn concurrent_writers_never_interleave() {
    let result = run(&[
        (
            "main.bash",
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

    let records = result.args("REC");
    assert_eq!(records.len(), 8 * 80);
    assert_eq!(
        records.iter().filter(|record| record.len() > 9000).count(),
        8 * 40,
        "oversized messages rejoined intact"
    );
}

#[test]
fn exit_status_is_reported_and_untouched() {
    assert_eq!(script("BC_INSTR say REC one\nexit 7").status, ExitStatus::Code(7));
    assert_eq!(script("BC_INSTR say REC one").status, ExitStatus::Code(0));
}

/// A signalled subject is reported as signalled rather than flattened into a
/// code, and everything it said before the signal is still there — the rig
/// installs no handler and needs none, because nothing was being held back.
#[test]
fn a_signalled_subject_is_reported_and_loses_nothing() {
    let result = script("BC_INSTR say REC before\nkill -TERM $$\nBC_INSTR say REC never");
    assert_eq!(result.status, ExitStatus::Signal(15));
    assert_eq!(result.status.code(), 143, "128 + signal, the shell convention");
    assert_eq!(result.args("REC"), ["before"], "{}", result.report());
}

/// Messages written immediately before the last writer exits must still be
/// readable once the child is gone.
#[test]
fn nothing_is_lost_at_the_end() {
    for _ in 0..10 {
        let result = script("for i in $(seq 1 200); do BC_INSTR say REC \"r$i\"; done\nexit 3");
        assert_eq!(result.args("REC").len(), 200);
        assert_eq!(result.status, ExitStatus::Code(3));
    }
}

/// The subject's shell is left as it was found, and the prelude needs nothing
/// but itself.
#[test]
fn the_prelude_is_non_invasive_and_self_reliant() {
    let temp = tempfile::tempdir().unwrap();
    let prelude =
        Rig::new(Behaviour::new()).prelude(temp.path(), &temp.path().join("up")).unwrap();
    let code: Vec<&str> = prelude
        .as_str()
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    let text = code.join("\n");

    for forbidden in ["eval", "trap", "export", "shopt"] {
        assert!(!text.contains(forbidden), "prelude must not contain {forbidden:?}:\n{text}");
    }
    // `set --` replaces a function's positional parameters and is scoped to
    // that call; `set -e`, `set -o` and friends change the shell the subject
    // is running in, and are what this forbids.
    for line in &code {
        let after_set = line.split("set -").nth(1);
        assert!(
            after_set.is_none_or(|rest| rest.starts_with('-')),
            "prelude must not change a shell option: {line}"
        );
    }
    // A line that *starts* with `NAME=value` and nothing further sets a
    // variable that persists. `IFS= read …` is a command prefix: it binds
    // only for that command, so it is not a global.
    for line in &code {
        let trimmed = line.trim_start();
        let Some((name, rest)) = trimmed.split_once('=') else { continue };
        let persists = name.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !name.is_empty()
            && rest.split_whitespace().count() <= 1;
        assert!(
            !persists || name.starts_with("__BC__"),
            "prelude sets a variable outside its namespace: {line}"
        );
    }

    let result = script("trap 'echo mine' EXIT\nIFS=,\nBC_INSTR say REC one two");
    assert_eq!(result.args("REC"), ["one two"], "{}", result.report());
}

/// The debug side channel is a file, not the wire, so it still says what
/// happened when the wire is what went wrong — including where each message
/// was sent from, which `BC_INSTR` takes at its own root.
#[test]
fn the_debug_channel_records_where_each_message_came_from() {
    let result = script("BC_INSTR say REC one\n( BC_INSTR say REC two )");
    assert!(result.debug.iter().any(|line| line.contains("main.bash")), "{}", result.report());
    assert!(result.debug.len() >= 3, "{}", result.report());
}

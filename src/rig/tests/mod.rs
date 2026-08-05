//! Bash-level proofs. Each spawns real bash and covers one mechanism that
//! cannot be checked by reading generated source.

use std::fs;

use super::*;

struct Run {
    capture: Capture,
    status: ExitStatus,
    debug: Vec<String>,
    _temp: tempfile::TempDir,
}

impl Run {
    fn args(&self, tag: &str) -> Vec<String> {
        self.capture
            .chronological()
            .into_iter()
            .filter(|line| line.value.tag == tag)
            .map(|line| line.value.args.join(" "))
            .collect()
    }

    fn report(&self) -> String {
        let lines: Vec<String> = self
            .capture
            .chronological()
            .into_iter()
            .map(|line| format!("  {} {}", line.stamp.pid, line.value.words().join(" ")))
            .collect();
        format!("capture:\n{}\ndebug:\n  {}", lines.join("\n"), self.debug.join("\n  "))
    }
}

fn recorder() -> Dispatch {
    Dispatch::new().on(&["REC"], "REC")
}

fn run(files: &[(&str, &str)], build: impl FnOnce(Rig) -> Rig) -> Run {
    let temp = tempfile::tempdir().unwrap();
    for (name, body) in files {
        fs::write(temp.path().join(name), body).unwrap();
    }
    let entry = temp.path().join(files[0].0).to_string_lossy().into_owned();
    let outcome = build(Rig::new().debug(true)).run(&[entry]).unwrap();
    let run = Run {
        capture: outcome.capture,
        status: outcome.status,
        debug: outcome.debug,
        _temp: temp,
    };
    assert!(run.capture.damage.is_empty(), "{}", run.report());
    run
}

fn script(body: &str, build: impl FnOnce(Rig) -> Rig) -> Run {
    run(&[("main.bash", body)], build)
}

/// Each shell joins the pipe by name, so subshells, command substitutions and
/// child processes all reach it with nothing inherited. The forest follows the
/// emitting parent, which `$PPID` would get wrong inside a subshell.
#[test]
fn every_descendant_shell_reaches_the_wire() {
    let result = run(
        &[
            (
                "main.bash",
                r#"
                REC top
                ( REC paren )
                value=$( REC cmdsubst; echo hi )
                bash "${BASH_SOURCE[0]%/*}/child.bash"
                REC after
                "#,
            ),
            ("child.bash", "REC child\n( REC grandchild )\n"),
        ],
        |rig| rig.with(recorder()),
    );

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
    let result = run(
        &[
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
                    REC "$1-$index-$small"
                    REC "$1-$index-$large"
                done
                "#,
            ),
        ],
        |rig| rig.with(recorder()),
    );

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
    assert_eq!(script("REC one\nexit 7", |rig| rig.with(recorder())).status, ExitStatus::Code(7));
    assert_eq!(script("REC one", |rig| rig.with(recorder())).status, ExitStatus::Code(0));
}

/// Messages written immediately before the last writer exits must still be
/// readable once the child is gone.
#[test]
fn nothing_is_lost_at_the_end() {
    for _ in 0..10 {
        let result = script("for i in $(seq 1 200); do REC \"r$i\"; done\nexit 3", |rig| {
            rig.with(recorder())
        });
        assert_eq!(result.args("REC").len(), 200);
        assert_eq!(result.status, ExitStatus::Code(3));
    }
}

/// The subject's shell is left as it was found, and the prelude needs nothing
/// but itself.
#[test]
fn the_prelude_is_non_invasive_and_self_reliant() {
    let temp = tempfile::tempdir().unwrap();
    let prelude = Rig::new()
        .with(recorder())
        .prelude(temp.path(), &temp.path().join("up"))
        .unwrap();
    let code: Vec<&str> = prelude
        .as_str()
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    let text = code.join("\n");

    for forbidden in ["eval", "trap", "export", "set -"] {
        assert!(!text.contains(forbidden), "prelude must not contain {forbidden:?}:\n{text}");
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

    let result = script("trap 'echo mine' EXIT\nREC one", |rig| rig.with(recorder()));
    assert_eq!(result.args("REC"), ["one"]);
}

/// The debug side channel is a file, not the wire, so it still says what
/// happened when the wire is what went wrong.
#[test]
fn the_debug_channel_records_what_the_shell_did() {
    let result = script("REC one\n( REC two )", |rig| rig.with(recorder()));
    assert!(result.debug.iter().any(|line| line.contains("send .")), "{}", result.report());
    assert!(result.debug.len() >= 3, "{}", result.report());
}

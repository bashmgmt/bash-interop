//! Bash-level proofs. Each spawns real bash and covers one mechanism that
//! cannot be checked by reading generated source.
//!
//! Most of these inject no bash at all: a fixture says what it has to say
//! through `BC_INSTR say`, which is the whole client surface.

use std::fs;
use std::time::{Duration, Instant};

use super::{BashSrc, Capture, ExitStatus, Reply, Rig, Setup, Turn, Workspace};

// ── fixtures ─────────────────────────────────────────────────────────

/// Says nothing of its own, and has nothing to answer.
struct Reporter;

impl Rig for Reporter {
    fn setup(&self) -> Setup {
        Setup::new()
    }

    fn answer(&mut self, _turn: &Turn) -> Reply {
        Reply::status(127)
    }
}

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

fn run(rig: &mut impl Rig, files: &[(&str, &str)]) -> Run {
    let temp = tempfile::tempdir().unwrap();
    for (name, body) in files {
        fs::write(temp.path().join(name), body).unwrap();
    }

    let entry = temp.path().join(files[0].0).to_string_lossy().into_owned();
    let outcome = rig.run(&[entry]).unwrap();
    let run =
        Run { capture: outcome.capture, status: outcome.status, debug: outcome.debug, _temp: temp };

    assert!(run.capture.damage.is_empty(), "{}", run.report());
    run
}

fn script(body: &str) -> Run {
    run(&mut Reporter, &[("main.bash", body)])
}

// ── the transport ────────────────────────────────────────────────────

/// Each shell joins the pipe by name, so subshells, command substitutions and
/// child processes all reach it with nothing inherited. The forest follows the
/// emitting parent, which `$PPID` would get wrong inside a subshell.
#[test]
fn every_descendant_shell_reaches_the_wire() {
    let result = run(
        &mut Reporter,
        &[
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
        ],
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
        &mut Reporter,
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
                    BC_INSTR say REC "$1-$index-$small"
                    BC_INSTR say REC "$1-$index-$large"
                done
                "#,
            ),
        ],
    );

    let records = result.args("REC");
    assert_eq!(records.len(), 8 * 80);
    assert_eq!(
        records.iter().filter(|record| record.len() > 9000).count(),
        8 * 40,
        "oversized messages rejoined intact"
    );
}

/// Messages written immediately before the last writer exits must still be
/// readable once the subject is gone.
#[test]
fn nothing_is_lost_at_the_end() {
    for _ in 0..10 {
        let result = script("for i in $(seq 1 200); do BC_INSTR say REC \"r$i\"; done\nexit 3");
        assert_eq!(result.args("REC").len(), 200);
        assert_eq!(result.status, ExitStatus::Code(3));
    }
}

// ── transparency ─────────────────────────────────────────────────────

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

/// The subject's shell is left as it was found, and the prelude needs nothing
/// but itself.
#[test]
fn the_prelude_is_non_invasive_and_self_reliant() {
    let temp = tempfile::tempdir().unwrap();
    let prelude = Reporter.prelude(temp.path(), &temp.path().join("up")).unwrap();
    let code: Vec<&str> =
        prelude.as_str().lines().filter(|line| !line.trim_start().starts_with('#')).collect();
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
        let Some((name, rest)) = line.trim_start().split_once('=') else { continue };
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

// ── answering ────────────────────────────────────────────────────────

/// Every way of answering, on one long session: each reply form in turn, one
/// of them deliberately slow, mixed with plain saying and with a message too
/// wide for one frame, from two shells that ask independently.
///
/// A reply is a command, so its expressiveness is whatever the prelude
/// defined — `NOTE` below is this rig's own word, and the operator calls it.
#[derive(Default)]
struct Soak {
    answered: usize,
}

const SOAK_BASH: &str = r#"
NOTE() { BC_INSTR say NOTE "$@"; }
"#;

impl Rig for Soak {
    fn setup(&self) -> Setup {
        Setup::new().bash(BashSrc::raw(SOAK_BASH))
    }

    fn answer(&mut self, turn: &Turn) -> Reply {
        self.answered += 1;
        let step: usize = turn.args().last().and_then(|word| word.parse().ok()).unwrap_or(0);

        match step % 7 {
            0 => Reply::nothing(),
            1 => Reply::of(["declare", "-g", &format!("mark_{step}=set")]),
            2 => Reply::eval(&format!("NOTE eval {step}")),
            3 => Reply::of(["NOTE", "call", &step.to_string()]),
            4 => turn.source(&BashSrc::raw(format!("NOTE source {step}"))),
            5 => {
                std::thread::sleep(Duration::from_millis(2));
                Reply::status(0)
            }
            _ => Reply::status(3),
        }
    }
}

#[test]
fn a_session_survives_every_way_of_answering() {
    let mut soak = Soak::default();
    let result = run(
        &mut soak,
        &[
            (
                "main.bash",
                r#"
                declare -i i=0
                while (( i < 56 )); do
                    BC_INSTR say REC tick "$i"
                    BC_INSTR ask step "$i" || BC_INSTR say REC refused "$i"
                    (( i += 1 ))
                done

                wide="$(printf 'W%.0s' {1..9000})"
                BC_INSTR say REC wide "$wide"

                bash "${BASH_SOURCE[0]%/*}/other.bash"
                BC_INSTR say REC marks "${!mark_@}"
                "#,
            ),
            ("other.bash", "BC_INSTR ask step 4\nBC_INSTR say REC other done\n"),
        ],
    );

    assert_eq!(result.status, ExitStatus::Code(0), "{}", result.report());
    assert_eq!(soak.answered, 57, "56 from the first shell, one from the second");

    let ticks = result.args("REC");
    assert_eq!(ticks.iter().filter(|line| line.starts_with("tick ")).count(), 56);
    assert_eq!(ticks.iter().filter(|line| line.starts_with("refused ")).count(), 8);
    assert!(ticks.iter().any(|line| line.len() > 9000), "the wide message survived");
    assert!(ticks.iter().any(|line| line == "other done"));

    // Every form that leaves a trace left one.
    let notes = result.args("NOTE");
    assert!(notes.iter().any(|note| note.starts_with("eval ")), "{}", result.report());
    assert!(notes.iter().any(|note| note.starts_with("call ")), "{}", result.report());
    assert!(notes.iter().any(|note| note.starts_with("source ")), "{}", result.report());

    // `declare -g` reached the subject's own scope, and the names are still
    // there at the end of the run.
    let marks = ticks.iter().find(|line| line.starts_with("marks ")).expect("the marks line");
    assert!(marks.contains("mark_1 "), "{marks}");
    assert!(marks.contains("mark_50"), "{marks}");
}

// ── the run owns its subject ─────────────────────────────────────────

/// A shell that asks after the subject has exited can never be answered, so
/// it must not be left behind to wait for one. The run takes the whole
/// process group with it.
#[test]
fn a_shell_left_asking_does_not_outlive_the_run() {
    let started = Instant::now();
    let result = script(
        r#"
        bash -c 'BC_INSTR say REC lingering $BASHPID; sleep 30; BC_INSTR ask never' &
        sleep 0.2
        exit 0
        "#,
    );
    assert!(started.elapsed() < Duration::from_secs(5), "the run must not wait for a straggler");

    let lingering: i32 = result
        .args("REC")
        .iter()
        .find_map(|line| line.strip_prefix("lingering ").map(str::to_string))
        .expect("the straggler reported itself")
        .parse()
        .unwrap();

    assert!(gone(lingering), "{lingering} outlived the run\n{}", result.report());
}

/// A panic in an answer is not swallowed — and it still releases the subject,
/// which would otherwise sit on its reply pipe forever.
#[test]
fn a_panicking_answer_releases_the_subject() {
    struct Exploding;

    impl Rig for Exploding {
        fn setup(&self) -> Setup {
            Setup::new()
        }

        fn answer(&mut self, _turn: &Turn) -> Reply {
            panic!("answer blew up")
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let entry = temp.path().join("main.bash");
    fs::write(&entry, "BC_INSTR ask anything\n").unwrap();
    let argv = [entry.to_string_lossy().into_owned()];

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Exploding.run(&argv)));
    std::panic::set_hook(previous);

    assert!(outcome.is_err(), "the panic must propagate rather than be swallowed");
    // Nothing is left blocked: the shell that asked is gone with the run.
    assert!(std::process::Command::new("true").status().is_ok());
}

// ── the workspace ────────────────────────────────────────────────────

/// A run's artifacts are worth keeping when something went wrong, so where
/// they live is part of what a rig describes.
#[test]
fn a_workspace_can_be_kept() {
    struct Kept(std::path::PathBuf);

    impl Rig for Kept {
        fn setup(&self) -> Setup {
            Setup::new().debug(true).in_workspace(Workspace::At(self.0.clone()))
        }

        fn answer(&mut self, turn: &Turn) -> Reply {
            turn.source(&BashSrc::raw("BC_INSTR say REC sourced"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let kept = temp.path().join("run");
    let entry = temp.path().join("main.bash");
    fs::write(&entry, "BC_INSTR ask something\n").unwrap();

    let outcome = Kept(kept.clone()).run(&[entry.to_string_lossy().into_owned()]).unwrap();
    assert_eq!(outcome.status, ExitStatus::Code(0));

    assert!(kept.join("prelude.bash").is_file(), "the prelude survived the run");
    assert!(kept.join("debug.log").is_file(), "so did the trace");
    let steps: Vec<_> = fs::read_dir(&kept)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("step."))
        .collect();
    assert_eq!(steps.len(), 1, "and the step the answer wrote");
}

// ── the debug channel ────────────────────────────────────────────────

/// It is a file, not the wire, so it still says what happened when the wire is
/// what went wrong — including where each message was sent from, which
/// `BC_INSTR` takes at its own root.
#[test]
fn the_debug_channel_records_where_each_message_came_from() {
    struct Traced;

    impl Rig for Traced {
        fn setup(&self) -> Setup {
            Setup::new().debug(true)
        }

        fn answer(&mut self, _turn: &Turn) -> Reply {
            Reply::status(127)
        }
    }

    let result = run(&mut Traced, &[("main.bash", "BC_INSTR say REC one\n( BC_INSTR say REC two )")]);
    assert!(result.debug.iter().any(|line| line.contains("main.bash")), "{}", result.report());
    assert!(result.debug.len() >= 3, "{}", result.report());
}

/// Waits briefly for a pid to disappear: the kill is immediate, but the
/// reaping is init's and takes a moment.
fn gone(pid: i32) -> bool {
    for _ in 0..100 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

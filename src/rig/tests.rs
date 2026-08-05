//! Bash-level proofs: each spawns real bash to cover one mechanism that
//! cannot be checked by reading the generated source.

use std::fs;
use std::time::{Duration, Instant};

use super::{BashSrc, Capture, ExitStatus, Line, Reply, Rig, RigError, Setup, Turn, Workspace};

// ── fixtures ─────────────────────────────────────────────────────────

/// Says nothing of its own, and has nothing to answer.
struct Reporter;

impl Rig for Reporter {
    type Session = Capture;
    type Output = (Capture, ExitStatus);

    fn start(&self) -> Result<(Setup, Capture), RigError> {
        Ok((Setup::new(), Capture::default()))
    }

    fn heard(&self, seen: &mut Capture, said: Line) -> Result<(), RigError> {
        seen.lines.push(said);
        Ok(())
    }

    fn answer(&self, _seen: &mut Capture, _asked: &Turn) -> Result<Reply, RigError> {
        Ok(Reply::status(127))
    }

    fn ended(&self, seen: Capture, status: ExitStatus) -> Result<Self::Output, RigError> {
        Ok((seen, status))
    }
}

/// Writes `files` to a scratch directory and runs the first one.
fn run<R: Rig>(rig: &R, files: &[(&str, &str)]) -> R::Output {
    let temp = tempfile::tempdir().unwrap();
    for (name, body) in files {
        fs::write(temp.path().join(name), body).unwrap();
    }

    let entry = temp.path().join(files[0].0).to_string_lossy().into_owned();
    rig.run(&[entry]).unwrap_or_else(|error| panic!("{error}"))
}

fn script(body: &str) -> (Capture, ExitStatus) {
    run(&Reporter, &[("main.bash", body)])
}

/// The words behind `lead` in every message that begins with it, in global
/// time order.
fn args(capture: &Capture, lead: &str) -> Vec<String> {
    capture
        .chronological()
        .into_iter()
        .filter_map(|line| line.value.behind(lead))
        .map(|rest| rest.join(" "))
        .collect()
}

fn report(capture: &Capture) -> String {
    capture
        .chronological()
        .into_iter()
        .map(|line| format!("  {} {}", line.stamp.pid, line.value.words.join(" ")))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── the transport ────────────────────────────────────────────────────

/// Nothing is inherited, and the forest follows the emitting parent — which
/// `$PPID` would get wrong inside a subshell.
#[test]
fn every_descendant_shell_reaches_the_wire() {
    let (seen, _) = run(
        &Reporter,
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
        args(&seen, "REC"),
        ["top", "paren", "cmdsubst", "child", "grandchild", "after"],
        "{}",
        report(&seen)
    );
    assert_eq!(seen.forest().len(), 1, "{}", report(&seen));
}

/// One pipe, many writers: frames stay under `PIPE_BUF` so they cannot
/// interleave, and anything longer is split and rejoined by `(pid, seq)`.
#[test]
fn concurrent_writers_never_interleave() {
    let (seen, _) = run(
        &Reporter,
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

    let records = args(&seen, "REC");
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
        let (seen, status) =
            script("for i in $(seq 1 200); do BC_INSTR say REC \"r$i\"; done\nexit 3");
        assert_eq!(args(&seen, "REC").len(), 200);
        assert_eq!(status, ExitStatus::Code(3));
    }
}

/// The delimiter separates frames and is part of none of them, so a value
/// carrying one arrives whole rather than as two frames of nonsense.
#[test]
fn a_newline_inside_a_value_is_escaped_not_framed() {
    let (seen, _) =
        script("payload=$'first\\nsecond\\tthird\\\\fourth'\nBC_INSTR say REC \"$payload\" plain\n");

    assert_eq!(args(&seen, "REC"), ["first\nsecond\tthird\\fourth plain"], "{}", report(&seen));
    assert_eq!(seen.lines.len(), 2, "one origin and one record, not three");
}

// ── transparency ─────────────────────────────────────────────────────

#[test]
fn exit_status_is_reported_and_untouched() {
    assert_eq!(script("BC_INSTR say REC one\nexit 7").1, ExitStatus::Code(7));
    assert_eq!(script("BC_INSTR say REC one").1, ExitStatus::Code(0));
}

/// Reported as signalled rather than flattened into a code, and nothing said
/// before the signal is lost. The rig installs no handler.
#[test]
fn a_signalled_subject_is_reported_and_loses_nothing() {
    let (seen, status) = script("BC_INSTR say REC before\nkill -TERM $$\nBC_INSTR say REC never");

    assert_eq!(status, ExitStatus::Signal(15));
    assert_eq!(status.code(), 143, "128 + signal, the shell convention");
    assert_eq!(args(&seen, "REC"), ["before"], "{}", report(&seen));
}

/// The subject's shell is left as it was found, and the prelude needs nothing
/// but itself.
#[test]
fn the_prelude_is_non_invasive_and_self_reliant() {
    let temp = tempfile::tempdir().unwrap();
    let (setup, _) = Reporter.start().unwrap();
    let prelude = super::run::prelude(&setup, temp.path(), &temp.path().join("up")).unwrap();
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

    let (seen, _) = script("trap 'echo mine' EXIT\nIFS=,\nBC_INSTR say REC one two");
    assert_eq!(args(&seen, "REC"), ["one two"], "{}", report(&seen));
}

// ── answering ────────────────────────────────────────────────────────

const SOAK_BASH: &str = r#"
NOTE() { BC_INSTR say NOTE "$@"; }
"#;

struct Soak;

#[derive(Default)]
struct Soaking {
    answered: usize,
    seen: Capture,
}

impl Rig for Soak {
    type Session = Soaking;
    type Output = (Soaking, ExitStatus);

    fn start(&self) -> Result<(Setup, Soaking), RigError> {
        Ok((Setup::new().bash(BashSrc::raw(SOAK_BASH)), Soaking::default()))
    }

    fn heard(&self, session: &mut Soaking, said: Line) -> Result<(), RigError> {
        session.seen.lines.push(said);
        Ok(())
    }

    fn answer(&self, session: &mut Soaking, asked: &Turn) -> Result<Reply, RigError> {
        session.answered += 1;
        let step: usize = asked.args().last().and_then(|word| word.parse().ok()).unwrap_or(0);

        Ok(match step % 7 {
            0 => Reply::status(0),
            1 => Reply::of(["declare", "-g", &format!("mark_{step}=set")]),
            2 => Reply::eval(&format!("NOTE eval {step}")),
            3 => Reply::of(["NOTE", "call", &step.to_string()]),
            4 => asked.source(&BashSrc::raw(format!("NOTE source {step}")))?,
            5 => {
                std::thread::sleep(Duration::from_millis(2));
                Reply::status(0)
            }
            _ => Reply::status(3),
        })
    }

    fn ended(&self, session: Soaking, status: ExitStatus) -> Result<Self::Output, RigError> {
        Ok((session, status))
    }
}

/// Every reply form in turn, one deliberately slow, mixed with saying and
/// with a message too wide for one frame, from two shells asking
/// independently. `NOTE` is this rig's own word, called by the operator.
#[test]
fn a_session_survives_every_way_of_answering() {
    let (soaked, status) = run(
        &Soak,
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
    let seen = &soaked.seen;

    assert_eq!(status, ExitStatus::Code(0), "{}", report(seen));
    assert_eq!(soaked.answered, 57, "56 from the first shell, one from the second");

    let ticks = args(seen, "REC");
    assert_eq!(ticks.iter().filter(|line| line.starts_with("tick ")).count(), 56);
    assert_eq!(ticks.iter().filter(|line| line.starts_with("refused ")).count(), 8);
    assert!(ticks.iter().any(|line| line.len() > 9000), "the wide message survived");
    assert!(ticks.iter().any(|line| line == "other done"));

    // Every form that leaves a trace left one.
    let notes = args(seen, "NOTE");
    assert!(notes.iter().any(|note| note.starts_with("eval ")), "{}", report(seen));
    assert!(notes.iter().any(|note| note.starts_with("call ")), "{}", report(seen));
    assert!(notes.iter().any(|note| note.starts_with("source ")), "{}", report(seen));

    // `declare -g` reached the subject's own scope, and the names are still
    // there at the end of the run.
    let marks = ticks.iter().find(|line| line.starts_with("marks ")).expect("the marks line");
    assert!(marks.contains("mark_1 "), "{marks}");
    assert!(marks.contains("mark_50"), "{marks}");
}

// ── the run owns its subject ─────────────────────────────────────────

/// A shell asking after the subject exited can never be answered, so the run
/// takes the whole process group with it.
#[test]
fn a_shell_left_asking_does_not_outlive_the_run() {
    let started = Instant::now();
    let (seen, _) = script(
        r#"
        bash -c 'BC_INSTR say REC lingering $BASHPID; sleep 30; BC_INSTR ask never' &
        sleep 0.2
        exit 0
        "#,
    );
    assert!(started.elapsed() < Duration::from_secs(5), "the run must not wait for a straggler");

    let lingering: i32 = args(&seen, "REC")
        .iter()
        .find_map(|line| line.strip_prefix("lingering ").map(str::to_string))
        .expect("the straggler reported itself")
        .parse()
        .unwrap();

    assert!(gone(lingering), "{lingering} outlived the run\n{}", report(&seen));
}

/// The panic propagates and the subject is still killed. Nothing comes back
/// from the run, so the subject writes its own pid to a file before asking.
#[test]
fn a_panicking_answer_kills_the_subject() {
    struct Exploding;

    impl Rig for Exploding {
        type Session = ();
        type Output = ExitStatus;

        fn start(&self) -> Result<(Setup, ()), RigError> {
            Ok((Setup::new(), ()))
        }

        fn heard(&self, _session: &mut (), _said: Line) -> Result<(), RigError> {
            Ok(())
        }

        fn answer(&self, _session: &mut (), _asked: &Turn) -> Result<Reply, RigError> {
            panic!("answer blew up")
        }

        fn ended(&self, _session: (), status: ExitStatus) -> Result<ExitStatus, RigError> {
            Ok(status)
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("pid");
    let entry = temp.path().join("main.bash");
    fs::write(&entry, format!("echo $BASHPID > {}\nBC_INSTR ask anything\n", marker.display()))
        .unwrap();
    let argv = [entry.to_string_lossy().into_owned()];

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Exploding.run(&argv)));
    std::panic::set_hook(previous);

    assert!(outcome.is_err(), "the panic must propagate rather than be swallowed");

    let blocked: i32 = fs::read_to_string(&marker)
        .expect("the subject reported its pid before asking")
        .trim()
        .parse()
        .unwrap();
    assert!(gone(blocked), "{blocked} was left waiting for an answer that will never come");
}

// ── the workspace ────────────────────────────────────────────────────

/// One directory holds the prelude, each step an answer wrote, and the debug
/// trace — a file rather than the wire, so it survives the wire failing.
#[test]
fn a_kept_workspace_holds_the_prelude_the_steps_and_the_trace() {
    struct Kept(std::path::PathBuf);

    impl Rig for Kept {
        type Session = ();
        type Output = ExitStatus;

        fn start(&self) -> Result<(Setup, ()), RigError> {
            Ok((Setup::new().debug(true).workspace(Workspace::At(self.0.clone())), ()))
        }

        fn heard(&self, _session: &mut (), _said: Line) -> Result<(), RigError> {
            Ok(())
        }

        fn answer(&self, _session: &mut (), asked: &Turn) -> Result<Reply, RigError> {
            asked.source(&BashSrc::raw("BC_INSTR say REC sourced"))
        }

        fn ended(&self, _session: (), status: ExitStatus) -> Result<ExitStatus, RigError> {
            Ok(status)
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let kept = temp.path().join("run");
    let entry = temp.path().join("main.bash");
    fs::write(&entry, "BC_INSTR ask something\n( BC_INSTR say REC subshell )\n").unwrap();

    let status = Kept(kept.clone()).run(&[entry.to_string_lossy().into_owned()]).unwrap();
    assert_eq!(status, ExitStatus::Code(0));
    assert!(kept.join("prelude.bash").is_file(), "the prelude survived the run");

    let steps = fs::read_dir(&kept)
        .unwrap()
        .filter(|entry| {
            entry.as_ref().is_ok_and(|it| it.file_name().to_string_lossy().starts_with("step."))
        })
        .count();
    assert_eq!(steps, 1, "the step the answer wrote");

    // Every message, with the call site `__bc_where` took.
    let trace = fs::read_to_string(kept.join("debug.log")).expect("the trace");
    assert!(trace.contains("main.bash"), "{trace}");
    assert!(trace.lines().count() >= 4, "{trace}");
}

/// The kill is immediate; the reaping is init's and takes a moment.
fn gone(pid: i32) -> bool {
    for _ in 0..100 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

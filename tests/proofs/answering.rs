//! Every form an answer can take, under load, from two shells at once.

use std::path::PathBuf;
use std::time::Duration;

use mb_resolver::bash::rig::{run, Answer, ExitStatus, Failure, Line, Rig, Run, Startup};

use crate::support::{bash, sourcing, Scripts};
use crate::{beginning, behind, report, ENTRY};

const SOAK_BASH: &str = r#"
NOTE() { BC_INSTR say NOTE "$@"; }
"#;

/// Answers each question a different way, cycling through every form.
struct Answering {
    steps: PathBuf,
}

/// Its session keeps what it heard *and* counts what it answered — a rig is
/// `&self`, so anything that changes belongs here.
#[derive(Default)]
struct Soak {
    heard: Vec<Line>,
    answered: usize,
}

impl Rig for Answering {
    type Session = Soak;

    /// `NOTE` is this rig's own word, called back by several of the answers.
    fn startup(&self) -> Startup {
        Startup { bash: SOAK_BASH.to_string(), ..Default::default() }
    }

    fn open(&self) -> Result<Soak, Failure> {
        Ok(Soak::default())
    }

    fn hear(&self, soak: &mut Soak, said: Line) -> Result<(), Failure> {
        soak.heard.push(said);

        Ok(())
    }

    fn answer(&self, soak: &mut Soak, asked: Line) -> Result<Answer, Failure> {
        let step: usize = asked.words.last().and_then(|word| word.parse().ok()).unwrap_or(0);

        soak.answered += 1;
        soak.heard.push(asked);

        Ok(match step % 7 {
            0 => Answer::status(0),
            1 => Answer::of(["declare", "-g", &format!("mark_{step}=set")]),
            2 => Answer::of(["eval", &format!("NOTE eval {step}")]),
            3 => Answer::of(["NOTE", "call", &step.to_string()]),
            4 => {
                let step_bash = self.steps.join(format!("step.{step}.bash"));
                return sourcing(&step_bash, &format!("NOTE source {step}"));
            }
            5 => {
                std::thread::sleep(Duration::from_millis(2));
                Answer::status(0)
            }
            _ => Answer::status(3),
        })
    }
}

/// Every answer form in turn, one deliberately slow, mixed with saying and
/// with a message too wide for one frame, from two shells asking
/// independently.
#[test]
fn a_session_survives_every_way_of_answering() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
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
            BC_INSTR say REC marks ${!mark_@}
            "#,
        ),
        ("other.bash", "BC_INSTR ask step 4\nBC_INSTR say REC other done\n"),
    ]);

    let answering = Answering { steps: scripts.dir().to_path_buf() };
    let (soak, status) = run(&answering, &bash(scripts.at(ENTRY)))
        .and_then(Run::whole)
        .unwrap_or_else(|error| panic!("{error}"));

    let seen = soak.heard;
    assert_eq!(status, ExitStatus::Code(0), "{}", report(&seen));
    assert_eq!(soak.answered, 57, "56 from the first shell, one from the second");

    let said = behind(&seen, "REC");
    assert_eq!(beginning(&said, "tick"), 56);
    assert_eq!(beginning(&said, "refused"), 8);
    assert_eq!(beginning(&said, "other"), 1, "the second shell got its answer too");
    assert!(
        said.iter().any(|words| words.iter().any(|word| word.len() == 9000)),
        "the split message rejoined to exactly what was written"
    );

    let notes = behind(&seen, "NOTE");
    for form in ["eval", "call", "source"] {
        assert!(beginning(&notes, form) > 0, "no answer arrived by {form}{}", report(&seen));
    }

    let marks = said
        .iter()
        .find(|words| words.first().is_some_and(|first| first == "marks"))
        .expect("the marks message");
    for name in ["mark_1", "mark_50"] {
        assert!(
            marks.iter().any(|word| word == name),
            "`declare -g` reached the subject's own scope, but {name} is missing from {marks:?}"
        );
    }
}

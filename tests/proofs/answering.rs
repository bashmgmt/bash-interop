//! Every form an answer can take, from two shells at once — and an answer that
//! waits on another shell's word, which only serving concurrently can give.

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use mb_resolver::bash::rig::{
    Answer, Driving, ExitStatus, Failure, Layout, Message, Reaching, Reacting, Rig, Run, Setup,
    Shell,
};
use tokio::sync::Notify;

use crate::support::{bash, sourcing, Scripts};
use crate::{beginning, behind, report, ENTRY};

const SOAK_BASH: &str = r#"
NOTE() { BC_INSTR SOAK say NOTE "$@"; }
"#;

/// Answers each question a different way, cycling through every form.
struct Answering {
    steps: PathBuf,
}

/// One shell's turn of it: what that shell said, and how many of its questions
/// were answered.
struct Soak {
    steps: PathBuf,
    heard: Vec<Message>,
    answered: usize,
}

/// What makes the run's messages reachable through `heard` and `behind`.
impl AsRef<[Message]> for Soak {
    fn as_ref(&self) -> &[Message] {
        &self.heard
    }
}

impl Rig for Answering {
    type Reaction = Soak;

    /// `NOTE` is this rig's own word, called back by several of the answers.
    fn setup(&self) -> Setup {
        Setup { label: "SOAK".to_string(), bash: SOAK_BASH.to_string() }
    }

    async fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Soak, Failure> {
        Ok(Soak { steps: self.steps.clone(), heard: Vec::new(), answered: 0 })
    }
}

impl Reacting for Soak {
    type Kept = Self;

    async fn hear(&mut self, said: Message) -> Result<(), Failure> {
        self.heard.push(said);

        Ok(())
    }

    async fn answer(&mut self, asked: Message) -> Result<Answer, Failure> {
        let step: usize = asked.words.last().and_then(|word| word.parse().ok()).unwrap_or(0);

        self.answered += 1;
        self.heard.push(asked);

        Ok(match step % 7 {
            0 => Answer::status(0),
            1 => Answer::of("declare", ["-g".to_string(), format!("mark_{step}=set")]),
            2 => Answer::of("eval", [format!("NOTE eval {step}")]),
            3 => Answer::of("NOTE", ["call".to_string(), step.to_string()]),
            4 => {
                let step_bash = self.steps.join(format!("step.{step}.bash"));
                return sourcing(&step_bash, &format!("NOTE source {step}"));
            }
            5 => {
                std::thread::sleep(Duration::from_millis(2));
                Answer::status(0)
            }
            6 => Answer::of("printf", ["%s".to_string(), "x".repeat(100_000)]),
            _ => Answer::status(3),
        })
    }

    async fn finish(self) -> Result<Self, Failure> {
        Ok(self)
    }
}

impl Driving for Answering {
    fn environment(&self, at: &Layout) -> Vec<(OsString, OsString)> {
        Reaching::BashEnv.environment(at)
    }
}

/// Every answer form in turn — one deliberately slow, one past the pipe's
/// buffer — mixed with saying and with a message too wide for one write, from
/// two shells asking independently.
#[tokio::test]
async fn a_session_survives_every_way_of_answering() {
    let scripts = Scripts::of(&[
        (
            ENTRY,
            r#"
            declare -i i=0
            while (( i < 56 )); do
                BC_INSTR SOAK say REC tick "$i"
                if (( i % 7 == 6 )); then
                    got="$(BC_INSTR SOAK ask step "$i")"
                    BC_INSTR SOAK say REC big "${#got}"
                else
                    BC_INSTR SOAK ask step "$i" || BC_INSTR SOAK say REC refused "$i"
                fi
                (( i += 1 ))
            done

            wide="$(printf 'W%.0s' {1..9000})"
            BC_INSTR SOAK say REC wide "$wide"

            bash "${BASH_SOURCE[0]%/*}/other.bash"
            BC_INSTR SOAK say REC marks ${!mark_@}
            "#,
        ),
        (
            "other.bash",
            r#"
            BC_INSTR SOAK ask step 4
            BC_INSTR SOAK say REC other done
            "#,
        ),
    ]);

    let answering = Answering { steps: scripts.dir().to_path_buf() };
    let ran = answering
        .run(&bash(scripts.at(ENTRY)))
        .await
        .and_then(Run::whole)
        .unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(ran.subject, ExitStatus::Code(0), "{}", report(&ran.shells));
    assert_eq!(
        ran.shells.iter().map(|at| at.kept.answered).collect::<Vec<_>>(),
        [48, 1, 1, 1, 1, 1, 1, 1, 1, 1],
        "each shell's own questions, counted where they were answered — the \
         eight command substitutions are shells of their own, then the child"
    );

    let said = behind(&ran.shells, "REC");
    assert_eq!(beginning(&said, "tick"), 56);
    assert_eq!(beginning(&said, "refused"), 0);
    assert_eq!(beginning(&said, "big"), 8, "{}", report(&ran.shells));
    assert!(said.iter().filter(|words| words[0] == "big").all(|words| words[1] == "100000"));
    assert_eq!(beginning(&said, "other"), 1, "the second shell got its answer too");
    assert!(
        said.iter().any(|words| words.iter().any(|word| word.len() == 9000)),
        "the wide message arrived as exactly what was written"
    );

    let notes = behind(&ran.shells, "NOTE");
    for form in ["eval", "call", "source"] {
        assert!(beginning(&notes, form) > 0, "no answer arrived by {form}{}", report(&ran.shells));
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

/// Answers a question only once another shell has spoken.
struct Gated {
    open: Rc<Notify>,
}

struct Gate {
    open: Rc<Notify>,
}

impl Rig for Gated {
    type Reaction = Gate;

    fn setup(&self) -> Setup {
        Setup { label: "GATE".to_string(), bash: String::new() }
    }

    async fn joined(&self, _at: &Layout, _shell: Arc<Shell>) -> Result<Gate, Failure> {
        Ok(Gate { open: Rc::clone(&self.open) })
    }
}

impl Reacting for Gate {
    type Kept = ();

    /// Any word from any shell opens the gate.
    async fn hear(&mut self, _said: Message) -> Result<(), Failure> {
        self.open.notify_one();

        Ok(())
    }

    /// The answer waits for that word. Every other shell keeps being served
    /// while it does — or the word never arrives, and this never returns.
    async fn answer(&mut self, _asked: Message) -> Result<Answer, Failure> {
        self.open.notified().await;

        Ok(Answer::of("echo", ["opened"]))
    }

    async fn finish(self) -> Result<(), Failure> {
        Ok(())
    }
}

impl Driving for Gated {
    fn environment(&self, at: &Layout) -> Vec<(OsString, OsString)> {
        Reaching::BashEnv.environment(at)
    }
}

/// One shell blocks on an answer that depends on another shell's word. Serving
/// each shell on a task of its own is what lets the second shell be heard while
/// the first is waiting.
#[tokio::test]
async fn an_answer_may_wait_on_another_shells_word() {
    let scripts = Scripts::of(&[(
        ENTRY,
        r#"
        got="$(BC_INSTR GATE ask open-please)" &
        sleep 0.2
        BC_INSTR GATE say REC opening
        wait
        "#,
    )]);
    let gated = Gated { open: Rc::new(Notify::new()) };
    let argv = bash(scripts.at(ENTRY));

    let ran = tokio::time::timeout(Duration::from_secs(10), gated.run(&argv))
        .await
        .expect("served concurrently, or this would never return")
        .unwrap();

    assert_eq!(ran.subject, ExitStatus::Code(0));
    assert_eq!(ran.shells.len(), 2, "the asker in its subshell, and the script");
    assert!(ran.failed.is_none());
}

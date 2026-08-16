//! What a real shell puts in a walk, and what the reader makes of it.
//!
//! Bash writes two of its own words into `FUNCNAME` and two more into
//! `BASH_SOURCE`, and it keeps a source path exactly as it was written. None
//! of that is visible in generated source, so it is read off a shell.

use std::sync::Arc;

use crate::rig::{
    Answer, Driving, Failure, Layout, Message, Provision, Reacting, Rig, Shell,
};
use crate::stack::{self, Columns, Site, Source, Stack};
use crate::scratch::{bash, Scripts};

/// The whole instrument: a word that walks and says what it found. The 2 is
/// `__bc_stack`'s own frame and `WALK`'s, so the walk starts at the subject.
/// `stack` tests itself with nothing but `stack`.
const BASH: &str = r#"
WALK() {
    local -a __w=()
    __bc_stack __w 2
    BC_INSTR WALK say WALK "${__w[@]}"
}
"#;

struct Walking;

/// One shell's walks. A walk is read against the shell it was taken in, and
/// this reaction was handed that shell before its first message could arrive.
struct Walks {
    shell: Arc<Shell>,
    seen: Vec<Stack>,
}

impl Rig for Walking {
    type Reaction = Walks;

    fn bash(&self, _at: &Layout) -> String {
        stack::with_walk(&[BASH])
    }

    fn joining(&self, at: &Layout) -> String {
        format!("BC_JOIN WALK {}\n", bash_strings::emit_scalar(at.text()))
    }

    async fn joined(&self, _at: &Layout, shell: Arc<Shell>) -> Result<Walks, Failure> {
        Ok(Walks { shell, seen: Vec::new() })
    }
}

impl Reacting for Walks {
    type Kept = Vec<Stack>;

    async fn hear(&mut self, said: Message) -> Result<(), Failure> {
        let Some(words) = said.behind("WALK") else { return Ok(()) };

        self.seen.push(Columns::of(words)?.frames(&self.shell)?);

        Ok(())
    }

    async fn answer(&mut self, asked: Message) -> Result<Answer, Failure> {
        self.hear(asked).await?;

        Ok(Answer::unknown())
    }

    async fn finish(self) -> Result<Vec<Stack>, Failure> {
        Ok(self.seen)
    }
}

impl Driving for Walking {}

/// Every walk a command line produced, shell by shell in the order they joined.
async fn walks_in<A: AsRef<std::ffi::OsStr>>(argv: &[A]) -> Vec<Stack> {
    let ran = Walking
        .run(argv, |at| Ok(vec![at.bash_env(Provision::Joining(&Walking.joining(at)))?]))
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    ran.whole().unwrap().shells.into_iter().flat_map(|at| at.kept).collect()
}

/// The same over a script — and the scripts, which stay alive: a source path
/// is only as readable as the file it names.
async fn walks(files: &[(&str, &str)]) -> (Scripts, Vec<Stack>) {
    let scripts = Scripts::of(files);
    let seen = walks_in(&bash(scripts.at("main.bash"))).await;

    (scripts, seen)
}

/// `main` is the top level of the script bash was given and `source` the top
/// level of a file it sourced. Neither is a function, and the reader says so
/// rather than handing on a name that is not one.
#[tokio::test]
async fn bashs_own_words_for_a_frame_come_back_as_what_they_are() {
    let (_scripts, seen) = walks(&[
        ("lib.bash", "WALK\n"),
        (
            "main.bash",
            r#"
            source "$(dirname "${BASH_SOURCE[0]}")/lib.bash"
            WALK
            "#,
        ),
    ])
    .await;

    let sites = |at: usize| -> Vec<Site> {
        seen[at].frames().map(|frame| frame.site.clone()).collect()
    };

    assert_eq!(seen.len(), 2);
    assert_eq!(sites(0), [Site::Sourced, Site::Script], "the sourced file, then the script");
    assert_eq!(sites(1), [Site::Script], "the script's own body alone");
}

/// Bash pushes a frame for the top level of a script file and for nothing
/// else. Where it pushed none, the walk still ends where it was entered: the
/// line is the cell the shift leaves over, and `$0` — which bash writes into
/// `BASH_SOURCE` for code it was given rather than read — is not a path.
#[tokio::test]
async fn a_shell_given_no_script_file_ends_at_the_frame_bash_never_pushed() {
    let inline = "outer() { WALK; }; outer";
    let on_stdin = format!("bash -s <<< {inline:?}");

    for (form, code) in [("a command line", inline), ("standard input", on_stdin.as_str())] {
        let seen = walks_in(&["bash", "-c", code]).await;
        assert_eq!(seen.len(), 1, "{form}");

        let sites: Vec<Site> = seen[0].frames().map(|frame| frame.site.clone()).collect();
        assert_eq!(sites, [Site::Function("outer".into()), Site::Shell], "{form}");

        let entered = &seen[0].below()[0];
        assert_eq!(entered.source, Source::Shell, "{form}");
        assert!(entered.lineno > 0, "the line the walk was entered from: {form}");
        assert!(entered.args.is_none(), "bash keeps no argument group for it: {form}");
    }
}

/// The same shape with nothing of the subject's left: every frame bash reported
/// belongs to the instrument, and the walk is the shell it was entered from.
/// This is the form a `make` recipe takes, and the one that used to be refused.
#[tokio::test]
async fn a_word_said_at_that_top_level_walks_to_the_shell_alone() {
    let seen = walks_in(&["bash", "-c", "WALK"]).await;

    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].frames().count(), 1);
    assert_eq!(seen[0].top().site, Site::Shell);
    assert_eq!(seen[0].top().source, Source::Shell);
}

/// A source path comes back absolute however the subject wrote it, by joining
/// the walk's own `$PWD`. Nothing is resolved: no symlink is followed and no
/// `..` collapsed, so the path is bash's own text under a known root.
#[tokio::test]
async fn a_relative_source_comes_back_absolute() {
    let (_scripts, seen) = walks(&[
        ("lib.bash", "where() { WALK; }\n"),
        (
            "main.bash",
            r#"
            cd "$(dirname "${BASH_SOURCE[0]}")"
            mkdir -p sub
            source ./sub/../lib.bash
            where
            "#,
        ),
    ])
    .await;

    let Source::File(path) = &seen[0].top().source else {
        panic!("a sourced file is a file, not one of bash's own words")
    };

    assert!(path.is_absolute(), "{}", path.display());
    assert!(path.ends_with("sub/../lib.bash"), "bash's own text, uncollapsed: {}", path.display());
    assert_eq!(
        seen[0].top().source.found(),
        Some(path.as_path()),
        "and `..` and all, it is there"
    );
}

/// Bash keeps the path as it was written, so a shell that changes directory
/// after sourcing leaves a relative one pointing nowhere. That is reported as
/// a path the run names and does not have, rather than silently guessed right.
#[tokio::test]
async fn a_subject_that_moved_leaves_a_source_that_is_not_there() {
    let (_scripts, seen) = walks(&[
        ("lib.bash", "where() { WALK; }\n"),
        (
            "main.bash",
            r#"
            cd "$(dirname "${BASH_SOURCE[0]}")"
            source ./lib.bash
            cd /
            where
            "#,
        ),
    ])
    .await;

    let source = &seen[0].top().source;

    assert_eq!(source, &Source::File("/lib.bash".into()), "joined onto the $PWD it now has");
    assert!(source.found().is_none(), "and there is nothing there");
    assert_eq!(source.missing(), Some(std::path::Path::new("/lib.bash")));
}

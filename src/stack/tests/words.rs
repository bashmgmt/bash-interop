//! What a real shell puts in a walk, and what the reader makes of it.
//!
//! Bash writes two of its own words into `FUNCNAME` and two more into
//! `BASH_SOURCE`, and it keeps a source path exactly as it was written. None
//! of that is visible in generated source, so it is read off a shell.

use crate::bash::rig::{Failure, Line, Master, Rig};
use crate::bash::stack::{self, Columns, Site, Source, Stack};
use crate::tests::scripts::{bash, Scripts};

/// The whole instrument: a word that walks and says what it found. The 2 is
/// `__bc_stack`'s own frame and `WALK`'s, so the walk starts at the subject.
/// `stack` tests itself with nothing but `stack`.
const BASH: &str = r#"
WALK() {
    local -a __w=()
    __bc_stack __w 2
    BC_INSTR say WALK "${__w[@]}"
}
"#;

struct Walking;

impl Rig for Walking {
    type Session = Vec<Stack>;

    fn bash(&self) -> String {
        stack::with(&[BASH])
    }

    fn open(&self) -> Result<Self::Session, Failure> {
        Ok(Vec::new())
    }

    fn hear(&self, seen: &mut Self::Session, said: Line) -> Result<(), Failure> {
        let Some(words) = said.behind("WALK") else { return Ok(()) };

        seen.push(Columns::of(words)?.frames()?);

        Ok(())
    }
}

impl Master for Walking {}

/// Every walk the run heard — and the scripts, which stay alive: a source path
/// is only as readable as the file it names.
fn walks(files: &[(&str, &str)]) -> (Scripts, Vec<Stack>) {
    let scripts = Scripts::of(files);
    let ran = Walking.run(&bash(scripts.at("main.bash"))).unwrap_or_else(|e| panic!("{e}"));

    (scripts, ran.whole().unwrap().0)
}

/// `main` is the top level of the script bash was given and `source` the top
/// level of a file it sourced. Neither is a function, and the reader says so
/// rather than handing on a name that is not one.
#[test]
fn bashs_own_words_for_a_frame_come_back_as_what_they_are() {
    let (_scripts, seen) = walks(&[
        ("lib.bash", "WALK\n"),
        ("main.bash", "source \"$(dirname \"${BASH_SOURCE[0]}\")/lib.bash\"\nWALK\n"),
    ]);

    let sites = |at: usize| -> Vec<Site> {
        seen[at].frames().map(|frame| frame.site.clone()).collect()
    };

    assert_eq!(seen.len(), 2);
    assert_eq!(sites(0), [Site::Sourced, Site::Script], "the sourced file, then the script");
    assert_eq!(sites(1), [Site::Script], "the script's own body alone");
}

/// A source path comes back absolute however the subject wrote it, by joining
/// the walk's own `$PWD`. Nothing is resolved: no symlink is followed and no
/// `..` collapsed, so the path is bash's own text under a known root.
#[test]
fn a_relative_source_comes_back_absolute() {
    let (_scripts, seen) = walks(&[
        ("lib.bash", "where() { WALK; }\n"),
        (
            "main.bash",
            "cd \"$(dirname \"${BASH_SOURCE[0]}\")\"\n\
             mkdir -p sub\n\
             source ./sub/../lib.bash\n\
             where\n",
        ),
    ]);

    let Source::File(path) = &seen[0].at().source else {
        panic!("a sourced file is a file, not one of bash's own words")
    };

    assert!(path.is_absolute(), "{}", path.display());
    assert!(path.ends_with("sub/../lib.bash"), "bash's own text, uncollapsed: {}", path.display());
    assert_eq!(
        seen[0].at().source.found(),
        Some(path.as_path()),
        "and `..` and all, it is there"
    );
}

/// Bash keeps the path as it was written, so a shell that changes directory
/// after sourcing leaves a relative one pointing nowhere. That is reported as
/// a path the run names and does not have, rather than silently guessed right.
#[test]
fn a_subject_that_moved_leaves_a_source_that_is_not_there() {
    let (_scripts, seen) = walks(&[
        ("lib.bash", "where() { WALK; }\n"),
        (
            "main.bash",
            "cd \"$(dirname \"${BASH_SOURCE[0]}\")\"\n\
             source ./lib.bash\n\
             cd /\n\
             where\n",
        ),
    ]);

    let source = &seen[0].at().source;

    assert_eq!(source, &Source::File("/lib.bash".into()), "joined onto the $PWD it now has");
    assert!(source.found().is_none(), "and there is nothing there");
    assert_eq!(source.missing(), Some(std::path::Path::new("/lib.bash")));
}

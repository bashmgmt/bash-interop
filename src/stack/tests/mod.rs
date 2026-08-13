//! The two halves of a walk: the columns, and the words bash puts in them.

mod words;


use std::path::Path;

use super::*;
use crate::bash::value::emit_array;

/// The layout bash produced for this stack, verified against a real shell:
///
/// ```text
/// probe() { … }          called as `probe solo`      from inner, line 9
/// inner() { probe solo; }        `inner i1 i2`       from outer, line 10
/// outer() { inner i1 i2; }       `outer o1 o2 o3`    from top,   line 11
/// top()   { outer o1 o2 o3; }    `top t1`            from main,  line 12
/// ```
fn real() -> [String; 5] {
    [
        emit_array(&words(&["probe", "inner", "outer", "top", "main"])),
        emit_array(&words(&["/x.bash"; 5])),
        emit_array(&words(&["9", "10", "11", "12", "0"])),
        emit_array(&words(&["1", "2", "3", "1", "0"])),
        emit_array(&words(&["solo", "i2", "i1", "o3", "o2", "o1", "t1"])),
    ]
}

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(ToString::to_string).collect()
}

fn columns(at: &[String; 5], skip: usize, traced: bool) -> Columns<'_> {
    Columns {
        skip,
        pwd: "/w",
        funcs: &at[0],
        sources: &at[1],
        lines: &at[2],
        args: traced.then(|| Args { argc: &at[3], argv: &at[4] }),
    }
}

/// Every index the columns encode, undone at once: the instrument's own
/// frames dropped, each line taken from the frame below, and each group of
/// arguments found by its offset and turned back around.
#[test]
fn a_walk_comes_back_as_the_calls_that_were_written() {
    let raw = real();
    let walk = columns(&raw, 1, true).frames().unwrap();
    let frames: Vec<&Frame> = walk.frames().collect();

    assert_eq!(frames.len(), 4, "probe's own frame is the instrument's");
    assert_eq!(walk.at().to_string(), "inner@x.bash:9 ('i1' 'i2')", "where the walk was taken");
    assert_eq!(frames[1].to_string(), "outer@x.bash:10 ('o1' 'o2' 'o3')");
    assert_eq!(frames[2].to_string(), "top@x.bash:11 ('t1')");
    assert_eq!(frames[3].args.as_deref(), Some([].as_slice()), "called with none");
    assert_eq!(frames[3].to_string(), "main@x.bash:12 ()", "which prints as none, not absent");
}

/// `skip` moves the boundary and nothing else: the frames it leaves are
/// the same frames, with the same lines and the same arguments.
#[test]
fn skipping_further_drops_frames_without_shifting_them() {
    let raw = real();
    let one = columns(&raw, 1, true).frames().unwrap();
    let three = columns(&raw, 3, true).frames().unwrap();

    assert_eq!(
        three.frames().collect::<Vec<_>>(),
        one.frames().skip(2).collect::<Vec<_>>(),
        "the same frames, two fewer"
    );
}

/// Without the argument columns a frame says it does not know, which is
/// not the same as knowing it was called with none.
#[test]
fn an_unrecorded_argument_stack_is_absent_not_empty() {
    let raw = real();

    assert!(columns(&raw, 1, false).frames().unwrap().frames().all(|f| f.args.is_none()));
    assert_eq!(columns(&raw, 1, false).frames().unwrap().at().to_string(), "inner@x.bash:9");
}

/// `extdebug` turned on part-way leaves `BASH_ARGC` short, and short means
/// every width belongs to a different frame. That is carried as absent
/// rather than read as if it lined up.
#[test]
fn a_short_argument_column_is_absent_rather_than_misread() {
    let raw = real();
    let short = emit_array(&words(&["3", "1", "0"]));
    let at = Columns {
        args: Some(Args { argc: &short, argv: &raw[4] }),
        ..columns(&raw, 1, false)
    };

    assert!(at.frames().unwrap().frames().all(|f| f.args.is_none()));
}

#[test]
fn a_record_that_does_not_line_up_is_refused() {
    let raw = real();
    let ragged = emit_array(&words(&["a", "b"]));

    let uneven = Columns { sources: &ragged, ..columns(&raw, 1, true) };
    assert!(uneven.frames().is_err(), "columns of different lengths");

    let over = emit_array(&words(&["9", "9", "9", "9", "9"]));
    let wide = Columns {
        args: Some(Args { argc: &over, argv: &raw[4] }),
        ..columns(&raw, 1, false)
    };
    assert!(wide.frames().is_err(), "widths claiming more arguments than there are");

    assert!(columns(&raw, 0, true).frames().is_err(), "skip is at least the emitter's own");
    assert!(columns(&raw, 5, true).frames().is_err(), "and never the whole walk");
}

/// Bash's own words for a frame that is not a function call, and for a
/// source that is not a file. Measured against bash 5.3.9: `main` and
/// `source` in `FUNCNAME`, `environment` and `main` in `BASH_SOURCE`.
#[test]
fn bashs_own_words_are_read_as_what_they_are() {
    let pwd = Path::new("/w");

    assert_eq!(Site::of("f__A"), Site::Function("f__A".into()));
    assert_eq!(Site::of("main"), Site::Script);
    assert_eq!(Site::of("source"), Site::Sourced);

    assert_eq!(Source::of("environment", pwd), Source::Environment);
    assert_eq!(Source::of("main", pwd), Source::Prompt);
    assert_eq!(Source::of("/abs/x.bash", pwd), Source::File("/abs/x.bash".into()));

    // A function defined inline in `bash -c` is sourced from `$0`, which
    // is a path like any other and is read as one.
    assert_eq!(Source::of("/bin/bash", pwd), Source::File("/bin/bash".into()));
}

/// A relative source joins the walk's own `$PWD`, and nothing is resolved:
/// `..` stays where bash wrote it and no symlink is followed.
#[test]
fn a_relative_source_joins_the_walk_s_own_directory() {
    let pwd = Path::new("/w/here");

    assert_eq!(Source::of("sub/x.bash", pwd), Source::File("/w/here/sub/x.bash".into()));
    assert_eq!(Source::of("sub/../x.bash", pwd), Source::File("/w/here/sub/../x.bash".into()));
    assert_eq!(Source::of("x.bash", pwd), Source::File("/w/here/x.bash".into()));

    // Absolute wins over the base, which is `Path::join`'s own rule.
    assert_eq!(Source::of("/x.bash", pwd), Source::File("/x.bash".into()));
}

/// A path this run cannot read is a path it names and does not have —
/// which is what a subject that changed directory after sourcing leaves
/// behind, and what neither `found` nor `missing` will say of a word that
/// was never a path.
#[test]
fn only_a_file_is_ever_found_or_missing() {
    let here = Source::of(file!(), Path::new(env!("CARGO_MANIFEST_DIR")));
    let gone = Source::of("nowhere/at/all.bash", Path::new("/w"));

    assert!(here.found().is_some() && here.missing().is_none());
    assert!(gone.found().is_none() && gone.missing() == Some(Path::new("/w/nowhere/at/all.bash")));

    for word in [Source::Environment, Source::Prompt] {
        assert!(word.found().is_none() && word.missing().is_none(), "{word:?} is not a path");
    }
}

#[test]
fn the_sections_are_read_off_a_payload() {
    let raw = real();
    let payload: Vec<String> = ["skip", "1", "pwd", "/w", "funcs", &raw[0], "sources",
        &raw[1], "lines", &raw[2], "argc", &raw[3], "argv", &raw[4]]
        .iter()
        .map(ToString::to_string)
        .collect();

    let read = Columns::of(&payload).unwrap().frames().unwrap();
    assert_eq!(read, columns(&raw, 1, true).frames().unwrap());

    assert!(Columns::of(&payload[2..]).is_err(), "no skip");
    assert!(Columns::of(&payload[..12]).is_err(), "argc without argv");
}

//! The sections `stack.bash` writes, and the index arithmetic that undoes
//! them.
//!
//! Bash keeps a stack in five parallel arrays and the instrument ships all
//! five as they are — no slicing, no walk. Three things have to be undone, and
//! all three are arithmetic:
//!
//! ```text
//! FUNCNAME     ('__bc_stack' 'BASHCAP' 'f__C' 'f__B' 'main')
//! BASH_SOURCE  (…)                                            aligned 1:1
//! BASH_LINENO  ('4' '8' '9' '10' '0')                          shifted by one
//! BASH_ARGC    ('2' '0' '0' '0' '1')                           aligned 1:1
//! BASH_ARGV    ('2' 'payload' 'x')             one flat stack, groups reversed
//! ```
//!
//! - **`skip`** — the leading frames belong to the instrument, not the
//!   subject. It is at least 1, the emitter's own.
//! - **the line shift** — `BASH_LINENO[i]` is where frame `i` *was called
//!   from*, so where frame `i` is *executing* is `BASH_LINENO[i - 1]`. Since
//!   `skip >= 1`, that index is in range for every reported frame.
//! - **the argument stack** — `BASH_ARGC[i]` is the width of group `i`, whose
//!   offset is the sum of the widths before it, and whose contents are
//!   reversed within the group.
//!
//! The shift leaves `BASH_LINENO[n - 1]` over, and that cell is the whole of
//! the last thing to undo. It is where the walk itself was entered. Bash
//! pushes a frame for the top level of a script file and for nothing else, so
//! for a script it is `0` — `main` was called from nowhere — and for a shell
//! given a `-c` command line or fed on standard input it is a real line, the
//! one frame `FUNCNAME` never names. A subject function called `main` does not
//! change it either way.
//!
//! The columns say nothing about the shell they were taken in, and one of
//! bash's own words needs it: `$0` stands in `BASH_SOURCE` for code bash was
//! given rather than read from a file. So [`Columns::frames`] is handed the
//! account that shell gave of itself when it joined, rather than shipping a
//! fact that cannot change with every walk that can.

use std::path::Path;

use crate::bash::rig::field;
use crate::bash::shell::Bash;
use crate::bash::value::parse_array;
use crate::failure::{Doing, Failure};

use super::{Frame, Site, Source, Stack};

/// `BASH_ARGC` and `BASH_ARGV`, as one instrument reported them.
pub struct Args<'a> {
    pub argc: &'a str,
    pub argv: &'a str,
}

/// One frame walk, in the sections `stack.bash` writes.
pub struct Columns<'a> {
    /// How many leading frames belong to the instrument rather than the
    /// subject. At least one — the emitter's own.
    pub skip: usize,

    /// The sending shell's `$PWD`, which a relative `BASH_SOURCE` is relative
    /// to as far as anything can know.
    pub pwd: &'a str,

    pub funcs: &'a str,
    pub sources: &'a str,
    pub lines: &'a str,

    /// Absent where an instrument does not report arguments at all.
    pub args: Option<Args<'a>>,
}

impl<'a> Columns<'a> {
    /// The sections out of a message's `key value` payload, which is the
    /// shape `stack.bash` appends them in.
    pub fn of(words: &'a [String]) -> Result<Self, Failure> {
        let at = |key: &str| field(words, key).ok_or_else(|| broken(format!("no {key:?} section")));

        let skip = at("skip")?;

        Ok(Self {
            skip: skip.parse().map_err(|_| broken(format!("skip {skip:?} is not a count")))?,
            pwd: at("pwd")?,
            funcs: at("funcs")?,
            sources: at("sources")?,
            lines: at("lines")?,
            args: match (field(words, "argc"), field(words, "argv")) {
                (Some(argc), Some(argv)) => Some(Args { argc, argv }),
                (None, None) => None,
                _ => return Err(broken("one of \"argc\"/\"argv\" without the other")),
            },
        })
    }

    /// The subject's walk, read against the shell it was taken in.
    ///
    /// `shell` is what that shell said of itself when it joined, and it is
    /// needed because `BASH_SOURCE` alone cannot say what its own words mean:
    /// `$0` is a path in one shell and a stand-in for code in another.
    pub fn frames(&self, shell: &Bash) -> Result<Stack, Failure> {
        let column = |name: &str, text: &str| {
            parse_array(text).doing(|| format!("reading the {name:?} column"))
        };

        let funcs = column("funcs", self.funcs)?;
        let sources = column("sources", self.sources)?;
        let lines = column("lines", self.lines)?;

        // Bash keeps these three at one length, always.
        if funcs.len() != sources.len() || funcs.len() != lines.len() {
            return Err(broken(format!(
                "columns of {} funcs, {} sources and {} lines",
                funcs.len(),
                sources.len(),
                lines.len()
            )));
        }

        // At least the emitter's own frame, and never past the end. Equal to
        // the end is every reported frame being the instrument's, which happens
        // where bash pushed none of its own — the entry line is what is left.
        if self.skip < 1 || self.skip > funcs.len() {
            return Err(broken(format!("skip {} of {} frames", self.skip, funcs.len())));
        }

        let numbered = |what: &str, text: &str| {
            text.parse::<u32>().map_err(|_| broken(format!("{what} {text:?}")))
        };
        let entered_at = numbered("entry line", &lines[funcs.len() - 1])?;

        let arguments = match &self.args {
            Some(args) => arguments(args, funcs.len())?,
            None => None,
        };

        let pwd = Path::new(self.pwd);
        let mut frames: Vec<Frame> = (self.skip..funcs.len())
            .map(|at| {
                Ok(Frame {
                    site: Site::of(&funcs[at]),
                    source: Source::of(&sources[at], pwd, shell),
                    // Where this frame is executing: the call site of the one
                    // below it.
                    lineno: numbered("line number", &lines[at - 1])?,
                    args: arguments.as_ref().map(|groups| groups[at].clone()),
                })
            })
            .collect::<Result<_, Failure>>()?;

        // The frame bash did not push, outermost and last. `BASH_ARGC` has no
        // group for it, so its arguments are absent in the field's own sense.
        if entered_at != 0 {
            frames.push(Frame {
                site: Site::Shell,
                source: Source::Shell,
                lineno: entered_at,
                args: None,
            });
        }

        Stack::of(frames).ok_or_else(|| broken("a walk with no frames"))
    }
}

/// One group per frame, in the order each call was written.
///
/// `BASH_ARGC` aligns 1:1 with `FUNCNAME` only where the shell was recording
/// arguments; enabling `extdebug` part-way leaves it short, and short means
/// every width belongs to a different frame. Alignment is the test, and an
/// unaligned record is **absent** rather than wrong — which is what keeps
/// "not recorded" distinct from "called with none".
fn arguments(args: &Args<'_>, frames: usize) -> Result<Option<Vec<Vec<String>>>, Failure> {
    let widths = parse_array(args.argc).doing(|| "reading the \"argc\" column".to_string())?;
    if widths.len() != frames {
        return Ok(None);
    }

    let flat = parse_array(args.argv).doing(|| "reading the \"argv\" column".to_string())?;
    let mut groups = Vec::with_capacity(frames);
    let mut from = 0usize;

    for width in &widths {
        let width: usize =
            width.parse().map_err(|_| broken(format!("argument count {width:?}")))?;
        let upto = from
            .checked_add(width)
            .filter(|&upto| upto <= flat.len())
            .ok_or_else(|| broken(format!("a group of {width} past {} arguments", flat.len())))?;

        // A group is reversed within itself, so counting back down undoes it.
        groups.push(flat[from..upto].iter().rev().cloned().collect());
        from = upto;
    }

    if from != flat.len() {
        return Err(broken(format!("{} arguments belong to no frame", flat.len() - from)));
    }

    Ok(Some(groups))
}

fn broken(what: impl Into<String>) -> Failure {
    Failure::new("reading a call stack", what.into())
}


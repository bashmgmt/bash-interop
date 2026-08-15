//! What comes out of a walk: one frame — what it is, where its code came from,
//! and what it was called with — and the [`Stack`] the frames make.

use std::fmt;
use std::iter::once;
use std::path::{Path, PathBuf};

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use crate::bash::shell::Shell;
use crate::bash::value::emit_q_words;

/// What a frame is, as `FUNCNAME` names it. Two of bash's words are not
/// function names, and a script that defines a function called `main` or
/// `source` is indistinguishable from them — bash reports the same word.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Site {
    Function(String),

    /// `main`: the top level of the script bash was given.
    Script,

    /// `source`: the top level of a file the subject sourced.
    Sourced,

    /// The top level of a shell bash was given no script file for — `bash -c`,
    /// or a shell fed on standard input. `FUNCNAME` has no entry for it, so it
    /// is not a word of bash's; where it sits is read off the line the walk was
    /// entered from.
    Shell,
}

impl Site {
    pub(super) fn of(funcname: &str) -> Self {
        match funcname {
            "main" => Self::Script,
            "source" => Self::Sourced,
            name => Self::Function(name.to_string()),
        }
    }
}

impl fmt::Display for Site {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Function(name) => f.write_str(name),
            Self::Script => f.write_str("main"),
            Self::Sourced => f.write_str("source"),
            Self::Shell => f.write_str("shell"),
        }
    }
}

/// Where a frame's code came from, as `BASH_SOURCE` names it. Two of bash's
/// words are not paths at all.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Absolute, by joining what bash reported onto the walk's own `$PWD`.
    /// Nothing is resolved: no symlink is followed and no `..` collapsed.
    ///
    /// Whether the file is there is [`found`](Source::found) and not this:
    /// bash keeps a source path as it was written, and a shell that has since
    /// changed directory leaves a relative one pointing nowhere.
    File(PathBuf),

    /// `environment`: the function came in through the environment.
    Environment,

    /// `main`: the function was defined at an interactive prompt.
    Prompt,

    /// The code bash was given rather than read: a `-c` command line, or
    /// standard input. Bash writes `$0` here, which is a word and not a path.
    Shell,
}

impl Source {
    /// Read against the shell the walk was taken in, which is the only thing
    /// that can say what `$0` means here: where bash was handed a file, `$0`
    /// is that file and reads as the path it is; where bash was given its code
    /// directly, the same word stands in `BASH_SOURCE` for that code.
    pub(super) fn of(source: &str, pwd: &Path, shell: &Shell) -> Self {
        match source {
            "environment" => Self::Environment,
            "main" => Self::Prompt,
            word if word == shell.bash.zero && !shell.bash.invocation.from_a_file() => Self::Shell,
            // An absolute path replaces the base; a relative one joins it.
            path => Self::File(pwd.join(path)),
        }
    }

    /// The file, if this names one and it is there.
    pub fn found(&self) -> Option<&Path> {
        match self {
            Self::File(path) if path.is_file() => Some(path),
            _ => None,
        }
    }

    /// The path this names but does not have, which is a source the run cannot
    /// be read against.
    pub fn missing(&self) -> Option<&Path> {
        match self {
            Self::File(path) if !path.is_file() => Some(path),
            _ => None,
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(path) => {
                f.write_str(path.file_name().map_or("?", |name| name.to_str().unwrap_or("?")))
            }
            Self::Environment => f.write_str("environment"),
            Self::Prompt => f.write_str("main"),
            Self::Shell => f.write_str("-"),
        }
    }
}

/// One frame: what it is, where its code came from, which line it is
/// executing, and what it was called with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Frame {
    pub site: Site,
    pub source: Source,
    pub lineno: u32,

    /// The call's arguments, when the shell was recording them. `None` is
    /// "not recorded", never "called with none": bash keeps these only under
    /// `extdebug`.
    pub args: Option<Vec<String>>,
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}:{}", self.site, self.source, self.lineno)?;

        match &self.args {
            Some(args) => write!(f, " ({})", emit_q_words(args)),
            None => Ok(()),
        }
    }
}

/// A walk, innermost first. Never empty: the frame it was taken in is always
/// one of them, and a walk that reaches no frame is refused where it is read.
///
/// One array in JSON, and one field wherever an instrument reports where it
/// was. Which frame is the call site is [`at`](Stack::at), not a second field
/// beside the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    at: Frame,
    outer: Vec<Frame>,
}

impl Stack {
    /// `None` for no frames at all, which is not a walk.
    pub fn of(frames: Vec<Frame>) -> Option<Self> {
        let mut frames = frames.into_iter();

        Some(Self { at: frames.next()?, outer: frames.collect() })
    }

    /// The frame the walk was taken in.
    pub fn top(&self) -> &Frame {
        &self.at
    }

    /// The frames above it, outermost last.
    pub fn below(&self) -> &[Frame] {
        &self.outer
    }

    pub fn frames(&self) -> impl Iterator<Item = &Frame> {
        once(&self.at).chain(&self.outer)
    }
}

impl Serialize for Stack {
    fn serialize<S: Serializer>(&self, into: S) -> Result<S::Ok, S::Error> {
        into.collect_seq(self.frames())
    }
}

impl<'de> Deserialize<'de> for Stack {
    fn deserialize<D: Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        Stack::of(Vec::deserialize(from)?)
            .ok_or_else(|| de::Error::custom("a call stack with no frames"))
    }
}

//! One frame of a bash call stack: what it is, where its code came from, and
//! what it was called with.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
    /// `shell` is the shell's own `$0`, and only where bash was given no script
    /// file. There it stands in `BASH_SOURCE` for code that came from the
    /// command line or from standard input; where bash was given a file, `$0`
    /// is that file and reads as one.
    pub(super) fn of(source: &str, pwd: &Path, shell: Option<&str>) -> Self {
        match source {
            "environment" => Self::Environment,
            "main" => Self::Prompt,
            word if shell == Some(word) => Self::Shell,
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


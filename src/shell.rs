//! What a bash shell is: which bash, how it was given its code, and what it
//! had switched on.
//!
//! Everything here is a shell's own account of itself, read back from the words
//! it wrote when it joined. Nothing is inferred from the shape of what it went
//! on to say — bash reports one word for several different things, `main`
//! standing for a script's top level in `FUNCNAME`, an interactive prompt in
//! `BASH_SOURCE`, and any function a subject cares to name that way, so a
//! reading that guessed from one of them would be right until it was not.
//!
//! A general utility about bash, like [`value`](super::value): nothing here
//! knows about the wire it arrived on.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::rig::field;
use super::value::parse_array;
use crate::failure::Failure;

/// `$BASH_VERSINFO`, all six elements. What bash behaves like is a function of
/// this, so a reading that has to bend for an older shell has what it needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub build: u32,

    /// `release`, `beta`, `rc1` — what bash calls its release status.
    pub status: String,

    /// `$MACHTYPE`, which is element five and not a separate fact.
    pub machine: String,
}

impl Version {
    /// The comparison that matters: `(major, minor, patch)` against a
    /// behaviour's first release.
    pub fn at_least(&self, major: u32, minor: u32, patch: u32) -> bool {
        (self.major, self.minor, self.patch) >= (major, minor, patch)
    }

    fn of(literal: &str) -> Result<Self, Failure> {
        let parts = parse_array(literal)
            .map_err(|cause| broken(format!("the version {literal:?}: {cause}")))?;

        let [major, minor, patch, build, status, machine] = parts.as_slice() else {
            return Err(broken(format!("a version of {} parts", parts.len())));
        };
        let count = |what: &str, text: &String| {
            text.parse().map_err(|_| broken(format!("{what} {text:?}")))
        };

        Ok(Self {
            major: count("a major version", major)?,
            minor: count("a minor version", minor)?,
            patch: count("a patch level", patch)?,
            build: count("a build number", build)?,
            status: status.clone(),
            machine: machine.clone(),
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}({})-{}", self.major, self.minor, self.patch, self.build, self.status)
    }
}

/// How bash was given the code it runs.
///
/// `set` refuses `-i`, `-c` and `-s`, so all three are settled when the shell
/// starts and cannot have changed by the time anything reads them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Started {
    /// `-c`, with the text bash was given. Absent for any other invocation.
    pub command: Option<String>,

    /// `-s`: the code arrives on standard input.
    pub standard_input: bool,

    /// `-i`: bash reads from a terminal, keeps history, and writes `main` into
    /// `BASH_SOURCE` for whatever is defined at the prompt.
    pub interactive: bool,
}

impl Started {
    /// Whether bash was handed a file to read, which is what makes
    /// [`Bash::zero`] a path rather than a word standing in for code bash was
    /// given directly.
    pub fn from_a_file(&self) -> bool {
        self.command.is_none() && !self.standard_input
    }
}

/// `$-`, as bash wrote it.
///
/// The letters are of two kinds and this is the string, not the reading: how
/// bash was started is [`Started`], asked once, and what follows here is only
/// the options a subject can turn on and off while it runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags(String);

impl Flags {
    pub fn has(&self, flag: char) -> bool {
        self.0.contains(flag)
    }

    pub fn errexit(&self) -> bool {
        self.has('e')
    }

    pub fn nounset(&self) -> bool {
        self.has('u')
    }

    pub fn xtrace(&self) -> bool {
        self.has('x')
    }

    pub fn letters(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Flags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a shell had switched on, at the moment it said so.
///
/// A snapshot and not an identity: a subject may `set -e` at any point, so this
/// is true of when it was taken and of nothing else.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub flags: Flags,

    /// `$SHELLOPTS`, split — the `set -o` options that were on.
    pub shellopts: Vec<String>,

    /// `$BASHOPTS`, split — the `shopt` options that were on.
    pub bashopts: Vec<String>,
}

impl State {
    /// Read off the words a shell wrote about itself.
    pub fn of(words: &[String]) -> Result<Self, Failure> {
        let split = |key: &str| -> Result<Vec<String>, Failure> {
            Ok(word(words, key)?.split(':').filter(|opt| !opt.is_empty()).map(String::from).collect())
        };

        Ok(Self {
            flags: Flags(word(words, "flags")?.to_string()),
            shellopts: split("shellopts")?,
            bashopts: split("bashopts")?,
        })
    }
}

/// Which bash a shell is, and how it was started. Constant for as long as the
/// shell lives — a fork inherits it, and anything that could change it makes a
/// new shell instead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bash {
    pub version: Version,

    /// `$BASH`: the binary this shell is running.
    pub binary: PathBuf,

    /// `$0`. A path where bash was handed a file, and otherwise the word bash
    /// also writes into `BASH_SOURCE` for the code it was given —
    /// [`Started::from_a_file`] is which.
    pub zero: String,

    /// `$BASH_SUBSHELL`. A subshell has a `$BASHPID` of its own and so joins as
    /// a shell of its own, which is why this is fixed for the shell's life.
    pub subshell: u32,

    pub started: Started,
}

impl Bash {
    /// Read off the words a shell wrote about itself.
    pub fn of(words: &[String]) -> Result<Self, Failure> {
        let flags = word(words, "flags")?;
        let command = word(words, "command")?;
        let subshell = word(words, "subshell")?;

        Ok(Self {
            version: Version::of(word(words, "versinfo")?)?,
            binary: PathBuf::from(word(words, "bash")?),
            zero: word(words, "zero")?.to_string(),
            subshell: subshell
                .parse()
                .map_err(|_| broken(format!("a subshell depth of {subshell:?}")))?,
            started: Started {
                command: flags.contains('c').then(|| command.to_string()),
                standard_input: flags.contains('s'),
                interactive: flags.contains('i'),
            },
        })
    }
}

fn word<'a>(words: &'a [String], key: &str) -> Result<&'a str, Failure> {
    field(words, key).ok_or_else(|| broken(format!("no {key:?}")))
}

fn broken(what: String) -> Failure {
    Failure::new("reading what a shell said of itself", what)
}

//! What a shell is: which bash, how it was given its code, and what it had
//! switched on when it joined.
//!
//! A shell's own account of itself, read back from the words it wrote. Nothing
//! is inferred from the shape of what it went on to say — bash reports one word
//! for several different things, `main` standing for a script's top level in
//! `FUNCNAME`, an interactive prompt in `BASH_SOURCE`, and any function a
//! subject cares to name that way.
//!
//! [`Bash`] and what it holds are description alone. [`Shell`] pairs that with
//! where the shell sits in the run.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::rig::wire::{field, Account, Pid, Stamp};
use bash_strings::parse_array;
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
        let count =
            |what: &str, text: &str| text.parse().map_err(|_| broken(format!("{what} {text:?}")));

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

/// How bash was given the code it runs. `set` refuses `-i`, `-c` and `-s`, so
/// all three are settled when the shell starts and cannot have changed by the
/// time anything reads them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    /// `-c`, with the text bash was given. Absent for any other invocation.
    pub command: Option<String>,

    /// `-s`: the code arrives on standard input.
    pub standard_input: bool,

    /// `-i`: bash reads from a terminal, keeps history, and writes `main` into
    /// `BASH_SOURCE` for whatever is defined at the prompt.
    pub interactive: bool,
}

impl Invocation {
    /// Whether bash was handed a file to read, which is what makes
    /// [`Bash::zero`] a path rather than a word standing in for code bash was
    /// given directly.
    pub fn from_a_file(&self) -> bool {
        self.command.is_none() && !self.standard_input
    }
}

/// `$-`, as bash wrote it.
///
/// The string, not the reading: how bash was started is [`Invocation`], asked
/// once, and what is left here are the options a subject turns on and off while
/// it runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags(String);

impl Flags {
    pub fn has(&self, flag: char) -> bool {
        self.0.contains(flag)
    }
}

impl fmt::Display for Flags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a shell had switched on at the moment it said so. A snapshot: a subject
/// may `set -e` at any point.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub flags: Flags,

    /// `$SHELLOPTS`, split — the `set -o` options that were on.
    pub shellopts: Vec<String>,

    /// `$BASHOPTS`, split — the `shopt` options that were on.
    pub bashopts: Vec<String>,
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
    /// [`Invocation::from_a_file`] is which.
    pub zero: String,

    pub invocation: Invocation,
}

/// A shell in a run: which bash it is, where it sits, and what it had switched
/// on when it joined.
///
/// Made once, from the account a shell gives before it says anything else. A
/// reaction is handed one at construction, so nothing about a shell is ever a
/// parameter afterwards.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shell {
    /// The order it joined in, counting from zero.
    pub nth: usize,

    pub pid: Pid,

    pub shlvl: u32,

    /// `$BASH_SUBSHELL`. A subshell has a `$BASHPID` of its own and so joins as
    /// a shell of its own, which is why this is fixed for the shell's life.
    pub subshell: u32,

    /// When it joined, on both clocks.
    pub joined: Stamp,

    pub bash: Bash,
    pub options: Options,

    /// The words its join carried — `BC_JOIN <label> <dir> word…` — verbatim,
    /// and empty where the join brought none. An arglist like a message's:
    /// `key value` pairs are a convention a client reads with
    /// [`field`].
    pub brought: Vec<String>,
}

impl Shell {
    /// Read off the words a shell wrote about itself.
    pub(crate) fn of(nth: usize, account: Account) -> Result<Self, Failure> {
        let Account { stamp: joined, words } = account;
        let word = |key: &str| {
            field(&words, key).ok_or_else(|| broken(format!("no {key:?}"))).map(str::to_string)
        };
        let count = |key: &str| -> Result<u32, Failure> {
            let text = word(key)?;
            text.parse().map_err(|_| broken(format!("{key} {text:?}")))
        };
        let split = |key: &str| -> Result<Vec<String>, Failure> {
            Ok(word(key)?.split(':').filter(|opt| !opt.is_empty()).map(String::from).collect())
        };

        let flags = word("flags")?;
        let command = word("command")?;
        let brought = parse_array(&word("brought")?)
            .map_err(|cause| broken(format!("the brought words: {cause}")))?;

        Ok(Self {
            nth,
            pid: Pid(count("pid")?),
            shlvl: count("shlvl")?,
            subshell: count("subshell")?,
            joined,
            bash: Bash {
                version: Version::of(&word("versinfo")?)?,
                binary: PathBuf::from(word("bash")?),
                zero: word("zero")?,
                invocation: Invocation {
                    command: flags.contains('c').then_some(command),
                    standard_input: flags.contains('s'),
                    interactive: flags.contains('i'),
                },
            },
            options: Options {
                shellopts: split("shellopts")?,
                bashopts: split("bashopts")?,
                flags: Flags(flags),
            },
            brought,
        })
    }
}

fn broken(what: String) -> Failure {
    Failure::new("reading what a shell said of itself", what)
}

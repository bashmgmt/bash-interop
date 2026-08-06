//! The process tree a run reveals: where each shell came from, and who
//! started whom.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use crate::bash::rig::wire::{field, Line, Micros, Pid};

/// Reserved: it opens a shell's stream rather than carrying any payload.
const ORIGIN_TAG: &str = "__ORIGIN__";

/// The preamble a shell writes before its first message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Origin {
    pub parent: Option<Pid>,

    pub shlvl: u32,

    pub source: Option<PathBuf>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BadField(pub &'static str);

impl fmt::Display for BadField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing or unreadable field {:?}", self.0)
    }
}

impl std::error::Error for BadField {}

impl Origin {
    /// `None` for a line that is not one.
    pub fn of(line: &Line) -> Option<Result<Self, BadField>> {
        Some(Self::decode(line.behind(ORIGIN_TAG)?))
    }

    fn decode(words: &[String]) -> Result<Self, BadField> {
        let set = |key| field(words, key).filter(|value| !value.is_empty());
        let number = |key| match set(key) {
            Some(value) => value.parse().map(Some).map_err(|_| BadField(key)),
            None => Ok(None),
        };

        Ok(Self {
            parent: number("parent")?.map(Pid),
            shlvl: number("shlvl")?.ok_or(BadField("shlvl"))?,
            source: set("source").map(PathBuf::from),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Shell<'a> {
    pub pid: Pid,
    pub opened_at: Micros,
    pub origin: Option<Origin>,
    pub lines: Vec<&'a Line>,
}

#[derive(Clone, Debug)]
pub struct ShellNode<'a> {
    pub shell: Shell<'a>,
    pub children: Vec<ShellNode<'a>>,
}

/// One shell per `__ORIGIN__`; every later line with the same pid joins the
/// newest shell carrying it, so a reused pid opens a new shell rather than
/// reopening the first.
///
/// Fails on an `__ORIGIN__` that will not decode: the transport's own message
/// is wrong, and a forest built from it would be quietly incomplete.
pub fn shells(lines: &[Line]) -> Result<Vec<Shell<'_>>, BadField> {
    let mut shells: Vec<Shell<'_>> = Vec::new();
    let mut newest: HashMap<Pid, usize> = HashMap::new();

    for line in lines {
        let origin = Origin::of(line).transpose()?;
        match newest.get(&line.pid) {
            Some(&index) if origin.is_none() => shells[index].lines.push(line),
            _ => {
                newest.insert(line.pid, shells.len());
                shells.push(Shell {
                    pid: line.pid,
                    opened_at: line.sent_at,
                    origin,
                    lines: vec![line],
                });
            }
        }
    }
    Ok(shells)
}

/// Shells linked through `Origin::parent`, choosing the newest candidate that
/// opened no later than the child. A shell whose parent never emitted is a
/// root.
pub fn forest<'a>(shells: &[Shell<'a>]) -> Vec<ShellNode<'a>> {
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();

    for (index, shell) in shells.iter().enumerate() {
        match parent_of(shells, index, shell) {
            Some(parent) => children.entry(parent).or_default().push(index),
            None => roots.push(index),
        }
    }
    roots.into_iter().map(|index| node(shells, &children, index)).collect()
}

fn parent_of(shells: &[Shell<'_>], index: usize, shell: &Shell<'_>) -> Option<usize> {
    let parent_pid = shell.origin.as_ref()?.parent?;
    shells
        .iter()
        .enumerate()
        .filter(|(other, candidate)| {
            *other != index
                && candidate.pid == parent_pid
                && candidate.opened_at <= shell.opened_at
        })
        .max_by_key(|(_, candidate)| candidate.opened_at)
        .map(|(other, _)| other)
}

fn node<'a>(
    shells: &[Shell<'a>],
    children: &HashMap<usize, Vec<usize>>,
    index: usize,
) -> ShellNode<'a> {
    ShellNode {
        shell: shells[index].clone(),
        children: children
            .get(&index)
            .into_iter()
            .flatten()
            .map(|&child| node(shells, children, child))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(at: u64, pid: u32, words: &[&str]) -> Line {
        Line {
            sent_at: Micros(at),
            heard_at: Micros(at + 1),
            pid: Pid(pid),
            seq: 0,
            words: words.iter().map(|word| word.to_string()).collect(),
        }
    }

    fn origin(at: u64, pid: u32, parent: &str) -> Line {
        line(at, pid, &[ORIGIN_TAG, "parent", parent, "shlvl", "5", "source", "/s.bash"])
    }

    #[test]
    fn origin_decodes_its_own_and_declines_the_rest() {
        let said = line(0, 1, &[ORIGIN_TAG, "parent", "", "shlvl", "6", "source", "/x.bash"]);
        let origin = Origin::of(&said).unwrap().unwrap();
        assert_eq!(origin.parent, None, "an empty parent is absent, not zero");
        assert_eq!(origin.shlvl, 6);
        assert_eq!(origin.source, Some(PathBuf::from("/x.bash")));

        let short = line(0, 1, &[ORIGIN_TAG, "parent", "3"]);
        assert_eq!(Origin::of(&short), Some(Err(BadField("shlvl"))));

        let wrong = line(0, 1, &[ORIGIN_TAG, "parent", "many", "shlvl", "6"]);
        assert_eq!(Origin::of(&wrong), Some(Err(BadField("parent"))));

        assert!(Origin::of(&line(0, 1, &["OTHER"])).is_none());
    }

    #[test]
    fn shells_and_the_forest_over_one_arrival_order() {
        let heard = vec![
            origin(100, 7, ""),
            line(110, 7, &["A"]),
            origin(130, 8, "7"),
            line(140, 8, &["B"]),
            origin(150, 9, "8"),
            origin(200, 7, ""),
        ];

        let shells = shells(&heard).unwrap();
        assert_eq!(shells.len(), 4, "the reused pid opens a fourth shell");
        assert_eq!(shells[0].lines.len(), 2);
        assert_eq!(shells[1].lines.len(), 2);
        assert_eq!(shells[3].pid, Pid(7));

        let forest = forest(&shells);
        assert_eq!(forest.len(), 2, "the root and the pid-reusing shell");
        assert_eq!(forest[0].shell.pid, Pid(7));
        assert_eq!(forest[0].children[0].shell.pid, Pid(8));
        assert_eq!(forest[0].children[0].children[0].shell.pid, Pid(9));
    }

    #[test]
    fn a_malformed_origin_is_not_silently_dropped() {
        let broken = vec![line(100, 7, &[ORIGIN_TAG, "parent", "many", "shlvl", "5"])];

        assert_eq!(shells(&broken).unwrap_err(), BadField("parent"));
    }
}

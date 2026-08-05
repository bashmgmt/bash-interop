//! Everything read from a run: flat, in read order. Chronological ordering,
//! per-shell grouping, the process forest, and typed decoding are all views.
//!
//! Lines that failed to decode are carried in `damage` rather than dropped.

use std::collections::HashMap;

use super::origin::Origin;
use super::record::{FromRecord, Line, Micros, Pid, Record, Stamp, Stamped};
use super::wire::Damage;

#[derive(Debug, Default)]
pub struct Capture {
    pub lines: Vec<Line>,
    pub damage: Vec<Damage>,
}

/// One emitting shell: the `__ORIGIN__` that opened it and everything it
/// wrote afterwards.
#[derive(Clone, Debug)]
pub struct Shell<'a> {
    pub pid: Pid,
    pub opened_at: Stamp,
    pub origin: Option<Origin>,
    pub lines: Vec<&'a Line>,
}

#[derive(Clone, Debug)]
pub struct ShellNode<'a> {
    pub shell: Shell<'a>,
    pub children: Vec<ShellNode<'a>>,
}

impl Capture {
    pub fn new(lines: Vec<Line>, damage: Vec<Damage>) -> Self {
        Self { lines, damage }
    }

    /// Records that did not come from a shell — read from a file, say. They
    /// carry pid 0 so their provenance never claims one produced them.
    pub fn literal(records: impl IntoIterator<Item = Record>) -> Self {
        let lines = records
            .into_iter()
            .enumerate()
            .map(|(index, value)| Stamped {
                stamp: Stamp { at: Micros(0), pid: Pid(0), seq: index as u32 },
                value,
            })
            .collect();
        Self { lines, damage: Vec::new() }
    }

    pub fn concat(parts: impl IntoIterator<Item = Capture>) -> Self {
        parts.into_iter().fold(Self::default(), |mut all, part| {
            all.lines.extend(part.lines);
            all.damage.extend(part.damage);
            all
        })
    }

    pub fn chronological(&self) -> Vec<&Line> {
        let mut ordered: Vec<&Line> = self.lines.iter().collect();
        ordered.sort_by_key(|line| line.stamp.order());
        ordered
    }

    /// A record tagged `__ORIGIN__` opens a shell; later records join the
    /// most recent shell with the same pid.
    pub fn shells(&self) -> Vec<Shell<'_>> {
        let mut shells: Vec<Shell<'_>> = Vec::new();
        let mut newest: HashMap<Pid, usize> = HashMap::new();

        for line in self.chronological() {
            let pid = line.stamp.pid;
            let opens = line.value.tag == Origin::TAG;
            match newest.get(&pid) {
                Some(&index) if !opens => shells[index].lines.push(line),
                _ => {
                    newest.insert(pid, shells.len());
                    shells.push(Shell {
                        pid,
                        opened_at: line.stamp,
                        origin: opens.then(|| Origin::from_record(&line.value).ok()).flatten(),
                        lines: vec![line],
                    });
                }
            }
        }
        shells
    }

    /// Roots are shells whose parent never emitted. A child attaches to the
    /// newest shell of its parent pid that opened no later than it did.
    pub fn forest(&self) -> Vec<ShellNode<'_>> {
        let shells = self.shells();
        let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut roots: Vec<usize> = Vec::new();

        for (index, shell) in shells.iter().enumerate() {
            match parent_of(&shells, index, shell) {
                Some(parent) => children.entry(parent).or_default().push(index),
                None => roots.push(index),
            }
        }
        roots.into_iter().map(|index| node(&shells, &children, index)).collect()
    }

    pub fn of<T: FromRecord>(&self) -> impl Iterator<Item = Stamped<Result<T, T::Err>>> + '_ {
        self.chronological()
            .into_iter()
            .filter(|line| line.value.tag == T::TAG)
            .map(|line| Stamped { stamp: line.stamp, value: T::from_record(&line.value) })
    }

    pub fn decoded<T: FromRecord>(&self) -> impl Iterator<Item = Stamped<T>> + '_ {
        self.of::<T>().filter_map(|entry| {
            entry.value.ok().map(|value| Stamped { stamp: entry.stamp, value })
        })
    }
}

fn parent_of(shells: &[Shell<'_>], index: usize, shell: &Shell<'_>) -> Option<usize> {
    let parent_pid = shell.origin.as_ref()?.parent?;
    shells
        .iter()
        .enumerate()
        .filter(|(other, candidate)| {
            *other != index
                && candidate.pid == parent_pid
                && candidate.opened_at.at <= shell.opened_at.at
        })
        .max_by_key(|(_, candidate)| candidate.opened_at.at)
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
    use crate::bash::rig::origin::ORIGIN_TAG;
    use crate::bash::rig::record::{Micros, Record};

    fn line(at: u64, pid: u32, seq: u32, tag: &str, args: &[&str]) -> Line {
        Stamped {
            stamp: Stamp { at: Micros(at), pid: Pid(pid), seq },
            value: Record::new(tag, args.iter().map(|arg| arg.to_string())),
        }
    }

    fn origin(at: u64, pid: u32, parent: &str) -> Line {
        line(at, pid, 0, ORIGIN_TAG, &["parent", parent, "shlvl", "5", "source", "/s.bash"])
    }

    /// Ordering, grouping, pid reuse, and the forest share one fixture: two
    /// children under a root, plus a later shell reusing the root's pid.
    #[test]
    fn views_over_one_capture() {
        let capture = Capture::new(
            vec![
                line(140, 8, 1, "B", &[]),
                origin(100, 7, ""),
                line(110, 7, 1, "A", &[]),
                origin(130, 8, "7"),
                origin(150, 9, "8"),
                origin(200, 7, ""),
            ],
            vec![],
        );

        let tags: Vec<&str> =
            capture.chronological().iter().map(|line| line.value.tag.as_str()).collect();
        assert_eq!(tags, [ORIGIN_TAG, "A", ORIGIN_TAG, "B", ORIGIN_TAG, ORIGIN_TAG]);

        let shells = capture.shells();
        assert_eq!(shells.len(), 4, "the reused pid opens a fourth shell");
        assert_eq!(shells[0].lines.len(), 2);
        assert_eq!(shells[1].lines.len(), 2);

        let forest = capture.forest();
        assert_eq!(forest.len(), 2, "the root and the pid-reusing shell");
        assert_eq!(forest[0].shell.pid, Pid(7));
        assert_eq!(forest[0].children[0].shell.pid, Pid(8));
        assert_eq!(forest[0].children[0].children[0].shell.pid, Pid(9));

        let parents: Vec<Option<Pid>> =
            capture.decoded::<Origin>().map(|entry| entry.value.parent).collect();
        assert_eq!(parents, [None, Some(Pid(7)), Some(Pid(8)), None]);
    }
}

//! The process tree a run reveals: which shell said what, and who started
//! whom. Both views take a slice, so they work over whatever a session kept.
//!
//! Nothing is decoded here. Every line already carries its own provenance, so
//! these are arrangements of it and cannot fail.

use std::collections::HashMap;

use crate::bash::rig::wire::{Line, Micros, Pid};

#[derive(Clone, Debug)]
pub struct Shell<'a> {
    pub pid: Pid,

    /// The shell that emitted before this one forked.
    pub parent: Pid,

    pub shlvl: u32,

    pub opened_at: Micros,

    pub lines: Vec<&'a Line>,
}

#[derive(Clone, Debug)]
pub struct ShellNode<'a> {
    pub shell: Shell<'a>,
    pub children: Vec<ShellNode<'a>>,
}

/// One shell per `seq == 0`, which is what a shell writes on joining. Every
/// later line joins the newest shell carrying its pid, so a pid reused across
/// a long run opens a new shell rather than reopening the first.
pub fn shells(lines: &[Line]) -> Vec<Shell<'_>> {
    let mut shells: Vec<Shell<'_>> = Vec::new();
    let mut newest: HashMap<Pid, usize> = HashMap::new();

    for line in lines {
        match newest.get(&line.pid) {
            Some(&index) if line.seq > 0 => shells[index].lines.push(line),
            _ => {
                newest.insert(line.pid, shells.len());
                shells.push(Shell {
                    pid: line.pid,
                    parent: line.parent,
                    shlvl: line.shlvl,
                    opened_at: line.sent_at,
                    lines: vec![line],
                });
            }
        }
    }
    shells
}

/// For each shell, the shell it was forked from: the newest generation of its
/// parent pid that had opened by then. `None` is a shell whose parent never
/// emitted — the outermost one, whose parent is the run itself.
///
/// A parent's index is always below its child's, so walking this upward
/// terminates: a shell can only name a parent that had already spoken, and
/// speaking is what puts a shell in this list.
fn forked_from(shells: &[Shell<'_>]) -> Vec<Option<usize>> {
    let generations = generations(shells);

    (0..shells.len()).map(|index| parent_of(shells, &generations, index)).collect()
}

/// Shells linked through `forked_from`, every root carrying what it started.
pub fn forest<'a>(shells: &[Shell<'a>]) -> Vec<ShellNode<'a>> {
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();

    for (index, forked_from) in forked_from(shells).into_iter().enumerate() {
        match forked_from {
            Some(parent) => children.entry(parent).or_default().push(index),
            None => roots.push(index),
        }
    }
    roots.into_iter().map(|index| node(shells, &children, index)).collect()
}

/// Every shell that carried a given pid, oldest first. A pid reused across a
/// run has several, and one process writes its own messages in sequence, so
/// the order `shells` produced them is already the order they opened in.
fn generations(shells: &[Shell<'_>]) -> HashMap<Pid, Vec<usize>> {
    let mut generations: HashMap<Pid, Vec<usize>> = HashMap::new();

    for (index, shell) in shells.iter().enumerate() {
        generations.entry(shell.pid).or_default().push(index);
    }
    generations
}

/// Which generation of the parent pid this shell belongs to: the last one that
/// opened no later than the child did. The candidates are ordered by
/// `opened_at`, so the boundary is a partition rather than a scan.
///
/// A parent spoke before it forked, so its own first message is ahead of the
/// child's in the stream and its index below. Taking that as the rule rather
/// than merely excluding the shell itself is what makes the relation acyclic.
fn parent_of(
    shells: &[Shell<'_>],
    generations: &HashMap<Pid, Vec<usize>>,
    index: usize,
) -> Option<usize> {
    let shell = &shells[index];
    let candidates = generations.get(&shell.parent)?;
    let upto = candidates.partition_point(|&at| shells[at].opened_at <= shell.opened_at);

    candidates[..upto].iter().rev().find(|&&at| at < index).copied()
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
    use crate::bash::rig::wire::Kind;

    fn line(at: u64, pid: u32, parent: u32, seq: u32) -> Line {
        Line {
            kind: Kind::Say,
            sent_at: Micros(at),
            heard_at: Micros(at + 1),
            pid: Pid(pid),
            parent: Pid(parent),
            shlvl: 5,
            seq,
            words: Vec::new(),
        }
    }

    #[test]
    fn shells_and_the_forest_over_one_arrival_order() {
        let heard = vec![
            line(100, 7, 1, 0),  // the outermost shell; the run is its parent
            line(110, 7, 1, 1),
            line(130, 8, 7, 0),  // a child of it
            line(140, 8, 7, 1),
            line(150, 9, 8, 0),  // a child of that
            line(200, 7, 1, 0),  // pid 7 again, freshly joined
        ];

        let shells = shells(&heard);
        assert_eq!(shells.len(), 4, "the reused pid opens a fourth shell");
        assert_eq!(shells[0].lines.len(), 2);
        assert_eq!(shells[1].lines.len(), 2);
        assert_eq!(shells[3].pid, Pid(7));

        let forest = forest(&shells);
        assert_eq!(forest.len(), 2, "the outermost shell, and the pid-reusing one");
        assert_eq!(forest[0].shell.pid, Pid(7));
        assert_eq!(forest[0].children[0].shell.pid, Pid(8));
        assert_eq!(forest[0].children[0].children[0].shell.pid, Pid(9));
    }

    /// A child names a pid, not a generation of one. Two shells carried pid 7,
    /// so each child attaches to the one that was alive when it opened —
    /// never to a later generation that had not started yet.
    #[test]
    fn a_child_attaches_to_the_generation_that_was_alive() {
        let heard = vec![
            line(100, 7, 1, 0), // pid 7, first generation
            line(150, 8, 7, 0), // opened while that one was alive
            line(200, 7, 1, 0), // pid 7 again, a second generation
            line(250, 9, 7, 0), // opened after the reuse
        ];

        let shells = shells(&heard);
        let forest = forest(&shells);

        assert_eq!(forest.len(), 2, "two generations of pid 7, both roots");
        assert_eq!(forest[0].shell.opened_at, Micros(100));
        assert_eq!(forest[0].children.len(), 1, "only the earlier child");
        assert_eq!(forest[0].children[0].shell.pid, Pid(8));

        assert_eq!(forest[1].shell.opened_at, Micros(200));
        assert_eq!(forest[1].children.len(), 1);
        assert_eq!(forest[1].children[0].shell.pid, Pid(9), "the later child");
    }

    /// A shell can only have been forked from one that had already spoken, so
    /// the relation points strictly backwards and a walk up it ends. Two
    /// shells naming each other's pid in one instant is the input that would
    /// otherwise close a loop.
    #[test]
    fn the_fork_relation_points_strictly_backwards() {
        let heard = [line(100, 7, 8, 0), line(100, 8, 7, 0)];
        let shells = shells(&heard);

        assert_eq!(forked_from(&shells), [None, Some(0)]);
    }

    /// A shell whose parent pid never emitted is a root: nothing is invented
    /// for it, and it is not silently attached to whatever else was running.
    #[test]
    fn a_shell_whose_parent_never_spoke_is_a_root() {
        let heard = [line(100, 7, 1, 0), line(150, 8, 99, 0)];
        let shells = shells(&heard);
        let forest = forest(&shells);

        assert_eq!(forest.len(), 2, "neither is anyone's child");
        assert!(forest.iter().all(|node| node.children.is_empty()));
    }
}

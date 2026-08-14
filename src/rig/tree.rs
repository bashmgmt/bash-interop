//! Who started whom.
//!
//! Nothing is decoded here. Every shell said what it was on joining, so this is
//! an arrangement of that and cannot fail.

use std::collections::HashMap;
use std::sync::Arc;

use super::{Attended, Pid, Shell};

#[derive(Clone, Debug)]
pub struct ShellNode {
    pub shell: Arc<Shell>,
    pub children: Vec<ShellNode>,
}

/// The shells of a run, linked through the fork relation, every root carrying
/// what it started.
pub fn forest<K>(shells: &[Attended<K>]) -> Vec<ShellNode> {
    let shells: Vec<&Arc<Shell>> = shells.iter().map(|at| &at.shell).collect();

    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();

    for (index, forked_from) in forked_from(&shells).into_iter().enumerate() {
        match forked_from {
            Some(parent) => children.entry(parent).or_default().push(index),
            None => roots.push(index),
        }
    }
    roots.into_iter().map(|index| node(&shells, &children, index)).collect()
}

/// For each shell, the shell it was forked from: the newest generation of its
/// parent pid that had opened by then. `None` is a shell whose parent never
/// emitted — the outermost one, whose parent is the run itself.
///
/// A parent's index is always below its child's, so walking this upward
/// terminates: a shell can only name a parent that had already spoken, and
/// speaking is what puts a shell in this list.
fn forked_from(shells: &[&Arc<Shell>]) -> Vec<Option<usize>> {
    let generations = generations(shells);

    (0..shells.len()).map(|index| parent_of(shells, &generations, index)).collect()
}

/// Every shell that carried a given pid, oldest first. A pid reused across a
/// run has several, and one process writes its own messages in sequence, so the
/// order they joined in is already the order they opened in.
fn generations(shells: &[&Arc<Shell>]) -> HashMap<Pid, Vec<usize>> {
    let mut generations: HashMap<Pid, Vec<usize>> = HashMap::new();

    for (index, shell) in shells.iter().enumerate() {
        generations.entry(shell.pid).or_default().push(index);
    }
    generations
}

/// Which generation of the parent pid this shell belongs to: the last one that
/// opened no later than the child did. The candidates are ordered by when they
/// joined, so the boundary is a partition rather than a scan.
///
/// A parent spoke before it forked, so its own first message is ahead of the
/// child's in the stream and its index below. Taking that as the rule rather
/// than merely excluding the shell itself is what makes the relation acyclic.
fn parent_of(
    shells: &[&Arc<Shell>],
    generations: &HashMap<Pid, Vec<usize>>,
    index: usize,
) -> Option<usize> {
    let shell = shells[index];
    let candidates = generations.get(&shell.parent)?;
    let upto = candidates.partition_point(|&at| shells[at].opened_at() <= shell.opened_at());

    candidates[..upto].iter().rev().find(|&&at| at < index).copied()
}

fn node(
    shells: &[&Arc<Shell>],
    children: &HashMap<usize, Vec<usize>>,
    index: usize,
) -> ShellNode {
    ShellNode {
        shell: Arc::clone(shells[index]),
        children: children
            .get(&index)
            .into_iter()
            .flatten()
            .map(|&child| node(shells, children, child))
            .collect(),
    }
}

#[cfg(test)]
mod tests;

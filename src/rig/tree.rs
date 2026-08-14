//! Which shells a run reveals, what each said of itself, and who started whom.
//!
//! Two questions, and two answers that stand apart. [`Shells`] is the register:
//! which shells there are and what they are, and nothing about what they went
//! on to say. [`shells`] is the arrangement: the same shells with their
//! messages grouped under them. The register is built by one rule, in one
//! place, and the arrangement is built on it — a decoder reading a run as it
//! arrives and one reading it afterwards get the same answer because they ask
//! the same code.
//!
//! Nothing is decoded twice and nothing is guessed. Every shell says what it is
//! when it joins; a message from a pid that never did is a fault.

use std::collections::HashMap;

use crate::bash::rig::wire::{Kind, Line, Pid, Sent};
use crate::bash::shell::{Bash, State};
use crate::failure::Failure;

/// Which shell, among those that have joined. A pid reused across a long run
/// opens a new one rather than reopening the first, so this is a generation
/// and not a pid.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct At(usize);

/// What one shell said of itself when it joined, under the provenance the
/// protocol put in front of it.
///
/// The two halves are not the same kind of fact and are not mixed: [`Bash`] is
/// what the shell *is*, fixed for as long as it lives, and [`State`] is what it
/// had switched on at that moment, which a subject may change afterwards.
#[derive(Clone, Debug)]
pub struct Joined {
    pub opened: Sent,
    pub bash: Bash,
    pub at_join: State,
}

/// Every shell that has joined, in the order they did.
#[derive(Default, Debug)]
pub struct Shells {
    joined: Vec<Joined>,

    /// The newest generation carrying each pid, which is what a later message
    /// from that pid belongs to.
    newest: HashMap<Pid, usize>,
}

impl Shells {
    /// The register a whole run reveals.
    pub fn of(lines: &[Line]) -> Result<Self, Failure> {
        let mut known = Self::default();

        for line in lines {
            known.hear(line)?;
        }

        Ok(known)
    }

    /// One message, and which shell it came from — opening a shell where the
    /// message is its own account of itself.
    ///
    /// This is the only rule, so a decoder reading a run as it arrives keeps
    /// the same register as one reading it afterwards.
    pub fn hear(&mut self, line: &Line) -> Result<At, Failure> {
        if line.kind == Kind::Join {
            let joined = Joined {
                opened: line.sent.clone(),
                bash: Bash::of(&line.words)?,
                at_join: State::of(&line.words)?,
            };

            self.newest.insert(line.sent.pid, self.joined.len());
            self.joined.push(joined);

            return Ok(At(self.joined.len() - 1));
        }

        self.newest.get(&line.sent.pid).copied().map(At).ok_or_else(|| {
            Failure::new(
                "placing a message",
                format!("pid {} spoke without ever joining", line.sent.pid),
            )
        })
    }

    pub fn at(&self, At(index): At) -> &Joined {
        &self.joined[index]
    }

    /// Every shell, in the order they joined.
    pub fn all(&self) -> &[Joined] {
        &self.joined
    }
}

/// One shell, what it said of itself, and everything it went on to say.
#[derive(Clone, Debug)]
pub struct Shell<'a> {
    pub joined: Joined,
    pub lines: Vec<&'a Line>,
}

#[derive(Clone, Debug)]
pub struct ShellNode<'a> {
    pub shell: Shell<'a>,
    pub children: Vec<ShellNode<'a>>,
}

/// The shells a run's messages reveal, each carrying its own.
///
/// The grouping walks [`Shells::hear`] rather than deciding anything of its
/// own, which is what keeps the two readings of one run in step.
pub fn shells(lines: &[Line]) -> Result<Vec<Shell<'_>>, Failure> {
    let mut known = Shells::default();
    let mut said: Vec<Vec<&Line>> = Vec::new();

    for line in lines {
        let At(index) = known.hear(line)?;

        if index == said.len() {
            said.push(Vec::new());
        }
        said[index].push(line);
    }

    Ok(known.joined.into_iter().zip(said).map(|(joined, lines)| Shell { joined, lines }).collect())
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
        generations.entry(shell.joined.opened.pid).or_default().push(index);
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
    let opened = &shells[index].joined.opened;
    let candidates = generations.get(&opened.parent)?;
    let upto =
        candidates.partition_point(|&at| shells[at].joined.opened.sent_at <= opened.sent_at);

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
mod tests;

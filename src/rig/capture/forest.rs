//! Who said what, and who started whom.

use std::collections::HashMap;

use super::{Capture, Origin};
use crate::bash::rig::wire::{FromRecord, Line, Pid, Stamp};

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
    pub fn shells(&self) -> Vec<Shell<'_>> {
        let mut shells: Vec<Shell<'_>> = Vec::new();
        let mut newest: HashMap<Pid, usize> = HashMap::new();

        for line in self.chronological() {
            let pid = line.stamp.pid;
            let origin = Origin::from_record(&line.value);
            match newest.get(&pid) {
                Some(&index) if origin.is_none() => shells[index].lines.push(line),
                _ => {
                    newest.insert(pid, shells.len());
                    shells.push(Shell {
                        pid,
                        opened_at: line.stamp,
                        origin: origin.and_then(Result::ok),
                        lines: vec![line],
                    });
                }
            }
        }
        shells
    }

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

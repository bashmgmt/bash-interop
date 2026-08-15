use super::*;
use crate::bash::rig::Micros;
use crate::tests::accounts;

/// A shell that joined at `at`, under `pid`, forked from `parent`. The order of
/// the list is the order they joined in, which is what a run produces.
fn shells(joined: &[(u64, u32, u32)]) -> Vec<Attended<()>> {
    joined
        .iter()
        .enumerate()
        .map(|(nth, &(at, pid, parent))| {
            let mut shell = (*accounts::shell(nth, pid, "subject.bash", "hB")).clone();
            shell.parent = Pid(parent);
            shell.joined.sent_at = Micros(at);

            Attended { shell: Arc::new(shell), kept: () }
        })
        .collect()
}

fn pids(nodes: &[ShellNode]) -> Vec<u32> {
    nodes.iter().map(|node| node.shell.pid.0).collect()
}

#[test]
fn shells_link_through_the_one_that_emitted_before_forking() {
    let forest = forest(&shells(&[
        (100, 7, 1), // the outermost shell; the run is its parent
        (130, 8, 7), // a child of it
        (150, 9, 8), // a child of that
        (200, 7, 1), // pid 7 again, freshly joined
    ]));

    assert_eq!(pids(&forest), [7, 7], "the outermost shell, and the pid-reusing one");
    assert_eq!(pids(&forest[0].children), [8]);
    assert_eq!(pids(&forest[0].children[0].children), [9]);
}

/// A child names a pid, not a generation of one. Two shells carried pid 7, so
/// each child attaches to the one that was alive when it opened — never to a
/// later generation that had not started yet.
#[test]
fn a_child_attaches_to_the_generation_that_was_alive() {
    let forest = forest(&shells(&[
        (100, 7, 1), // pid 7, first generation
        (150, 8, 7), // opened while that one was alive
        (200, 7, 1), // pid 7 again, a second generation
        (250, 9, 7), // opened after the reuse
    ]));

    assert_eq!(forest.len(), 2, "two generations of pid 7, both roots");
    assert_eq!(pids(&forest[0].children), [8], "only the earlier child");
    assert_eq!(pids(&forest[1].children), [9], "the later child");
}

/// A shell can only have been forked from one that had already spoken, so the
/// relation points strictly backwards and a walk up it ends. Two shells naming
/// each other's pid in one instant is the input that would otherwise close a
/// loop.
#[test]
fn the_fork_relation_points_strictly_backwards() {
    let forest = forest(&shells(&[(100, 7, 8), (100, 8, 7)]));

    assert_eq!(pids(&forest), [7], "one root, and no cycle to walk");
    assert_eq!(pids(&forest[0].children), [8]);
}

/// A shell whose parent pid never emitted is a root: nothing is invented for
/// it, and it is not silently attached to whatever else was running.
#[test]
fn a_shell_whose_parent_never_spoke_is_a_root() {
    let forest = forest(&shells(&[(100, 7, 1), (150, 8, 99)]));

    assert_eq!(pids(&forest), [7, 8], "neither is anyone's child");
    assert!(forest.iter().all(|node| node.children.is_empty()));
}

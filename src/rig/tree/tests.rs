use super::*;
use crate::bash::rig::wire::Micros;
use crate::tests::accounts::account;

fn line(at: u64, pid: u32, parent: u32, kind: Kind, seq: u32) -> Line {
    Line {
        kind,
        sent: Sent {
            pid: Pid(pid),
            parent: Pid(parent),
            shlvl: 5,
            seq,
            sent_at: Micros(at),
            heard_at: Micros(at + 1),
        },
        words: match kind {
            Kind::Join => account("subject.bash", "hB"),
            _ => Vec::new(),
        },
    }
}

/// A shell opening: its account of itself, which always carries seq 0.
fn joins(at: u64, pid: u32, parent: u32) -> Line {
    line(at, pid, parent, Kind::Join, 0)
}

fn says(at: u64, pid: u32, parent: u32, seq: u32) -> Line {
    line(at, pid, parent, Kind::Say, seq)
}

#[test]
fn shells_and_the_forest_over_one_arrival_order() {
    let heard = vec![
        joins(100, 7, 1), // the outermost shell; the run is its parent
        says(110, 7, 1, 1),
        joins(130, 8, 7), // a child of it
        says(140, 8, 7, 1),
        joins(150, 9, 8), // a child of that
        joins(200, 7, 1), // pid 7 again, freshly joined
    ];

    let shells = shells(&heard).unwrap();
    assert_eq!(shells.len(), 4, "the reused pid opens a fourth shell");
    assert_eq!(shells[0].lines.len(), 2);
    assert_eq!(shells[1].lines.len(), 2);
    assert_eq!(shells[3].joined.opened.pid, Pid(7));

    let forest = forest(&shells);
    assert_eq!(forest.len(), 2, "the outermost shell, and the pid-reusing one");
    assert_eq!(forest[0].shell.joined.opened.pid, Pid(7));
    assert_eq!(forest[0].children[0].shell.joined.opened.pid, Pid(8));
    assert_eq!(forest[0].children[0].children[0].shell.joined.opened.pid, Pid(9));
}

/// A child names a pid, not a generation of one. Two shells carried pid 7,
/// so each child attaches to the one that was alive when it opened — never to
/// a later generation that had not started yet.
#[test]
fn a_child_attaches_to_the_generation_that_was_alive() {
    let heard = vec![
        joins(100, 7, 1), // pid 7, first generation
        joins(150, 8, 7), // opened while that one was alive
        joins(200, 7, 1), // pid 7 again, a second generation
        joins(250, 9, 7), // opened after the reuse
    ];

    let shells = shells(&heard).unwrap();
    let forest = forest(&shells);

    assert_eq!(forest.len(), 2, "two generations of pid 7, both roots");
    assert_eq!(forest[0].shell.joined.opened.sent_at, Micros(100));
    assert_eq!(forest[0].children.len(), 1, "only the earlier child");
    assert_eq!(forest[0].children[0].shell.joined.opened.pid, Pid(8));

    assert_eq!(forest[1].shell.joined.opened.sent_at, Micros(200));
    assert_eq!(forest[1].children.len(), 1);
    assert_eq!(forest[1].children[0].shell.joined.opened.pid, Pid(9), "the later child");
}

/// A shell can only have been forked from one that had already spoken, so the
/// relation points strictly backwards and a walk up it ends. Two shells naming
/// each other's pid in one instant is the input that would otherwise close a
/// loop.
#[test]
fn the_fork_relation_points_strictly_backwards() {
    let heard = [joins(100, 7, 8), joins(100, 8, 7)];
    let shells = shells(&heard).unwrap();

    assert_eq!(forked_from(&shells), [None, Some(0)]);
}

/// A shell whose parent pid never emitted is a root: nothing is invented for
/// it, and it is not silently attached to whatever else was running.
#[test]
fn a_shell_whose_parent_never_spoke_is_a_root() {
    let heard = [joins(100, 7, 1), joins(150, 8, 99)];
    let shells = shells(&heard).unwrap();
    let forest = forest(&shells);

    assert_eq!(forest.len(), 2, "neither is anyone's child");
    assert!(forest.iter().all(|node| node.children.is_empty()));
}

/// The register is what a shell said, not what was guessed from how it went on
/// to speak. A message from a pid that never joined has no shell to belong to,
/// and saying so is the only honest answer.
#[test]
fn a_pid_that_never_joined_has_no_shell() {
    let mut known = Shells::default();

    assert!(known.hear(&says(100, 7, 1, 1)).is_err(), "no account of pid 7");

    let at = known.hear(&joins(110, 7, 1)).unwrap();
    assert_eq!(known.hear(&says(120, 7, 1, 1)).unwrap(), at, "and now it belongs to that one");
    assert_eq!(known.at(at).bash.zero, "subject.bash");
    assert_eq!(known.all().len(), 1);
}

/// A pid that joins again is a new shell, and the messages after it belong to
/// the new one.
#[test]
fn joining_again_under_one_pid_opens_a_second_shell() {
    let mut known = Shells::default();

    let first = known.hear(&joins(100, 7, 1)).unwrap();
    let second = known.hear(&joins(200, 7, 1)).unwrap();

    assert_ne!(first, second);
    assert_eq!(known.hear(&says(210, 7, 1, 1)).unwrap(), second);
    assert_eq!(known.all().len(), 2);
}

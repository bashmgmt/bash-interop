//! Everything read from a run, flat and in read order. Ordering, per-shell
//! grouping, the process forest and typed decoding are all views over it.

mod forest;
mod origin;

pub use forest::{Shell, ShellNode};
pub use origin::{Origin, ORIGIN_TAG};

use crate::bash::rig::wire::{FromRecord, Line, Micros, Pid, Record, Stamp, Stamped};

#[derive(Debug, Default)]
pub struct Capture {
    pub lines: Vec<Line>,
}

impl Capture {
    /// Records that came from no shell. Stamped pid 0, so their provenance
    /// never claims one produced them.
    pub fn literal(records: impl IntoIterator<Item = Record>) -> Self {
        Self {
            lines: records
                .into_iter()
                .enumerate()
                .map(|(index, value)| Stamped {
                    stamp: Stamp { at: Micros(0), pid: Pid(0), seq: index as u32 },
                    value,
                })
                .collect(),
        }
    }

    /// Order is a view, so this only appends.
    pub fn concat(parts: impl IntoIterator<Item = Capture>) -> Self {
        Self { lines: parts.into_iter().flat_map(|part| part.lines).collect() }
    }

    pub fn chronological(&self) -> Vec<&Line> {
        let mut ordered: Vec<&Line> = self.lines.iter().collect();
        ordered.sort_by_key(|line| line.stamp.order());
        ordered
    }


    /// Every record the family recognised, successes and failures alike.
    pub fn of<T: FromRecord>(&self) -> impl Iterator<Item = Stamped<Result<T, T::Err>>> + '_ {
        self.chronological().into_iter().filter_map(|line| {
            T::from_record(&line.value).map(|value| Stamped { stamp: line.stamp, value })
        })
    }

    pub fn decoded<T: FromRecord>(&self) -> impl Iterator<Item = Stamped<T>> + '_ {
        self.of::<T>().filter_map(|entry| {
            entry.value.ok().map(|value| Stamped { stamp: entry.stamp, value })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::origin::ORIGIN_TAG;

    fn line(at: u64, pid: u32, seq: u32, lead: &str, rest: &[&str]) -> Line {
        Stamped {
            stamp: Stamp { at: Micros(at), pid: Pid(pid), seq },
            value: Record::new(std::iter::once(lead).chain(rest.iter().copied())),
        }
    }

    fn origin(at: u64, pid: u32, parent: &str) -> Line {
        line(at, pid, 0, ORIGIN_TAG, &["parent", parent, "shlvl", "5", "source", "/s.bash"])
    }

    /// Ordering, grouping, pid reuse, and the forest share one fixture: two
    /// children under a root, plus a later shell reusing the root's pid.
    #[test]
    fn views_over_one_capture() {
        let capture = Capture {
            lines: vec![
                line(140, 8, 1, "B", &[]),
                origin(100, 7, ""),
                line(110, 7, 1, "A", &[]),
                origin(130, 8, "7"),
                origin(150, 9, "8"),
                origin(200, 7, ""),
            ],
        };

        let tags: Vec<&str> =
            capture.chronological().iter().map(|line| line.value.words[0].as_str()).collect();
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

//! Lane layout for the commit graph. Pure logic: commits in, per-row drawing instructions out.
//!
//! Commits come newest first with children before parents (git log --date-order). Each lane waits for one commit; a
//! commit takes the lane waiting for it (or a free one if nothing is: a branch tip), lanes that were also waiting for
//! it merge into it, and its parents take over: the first parent continues in the same lane, further parents
//! (merges) get lanes of their own.

use crate::queries::Commit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// Row top to the commit's dot.
    Top,
    /// The commit's dot to the row bottom.
    Bottom,
    /// Row top to row bottom, past the commit.
    Through,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub from: usize,
    pub to: usize,
    pub part: Part,
    /// Colour index; the renderer picks from a palette.
    pub color: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Lane of the commit's dot.
    pub lane: usize,
    pub color: usize,
    pub lines: Vec<Line>,
    /// Lanes this row needs.
    pub width: usize,
}

#[derive(Clone)]
struct Lane {
    sha: String,
    color: usize,
}

pub fn layout(commits: &[Commit]) -> Vec<Row> {
    layout_parents(commits.iter().map(|c| (c.sha.as_str(), c.parents.as_slice())))
}

pub fn layout_parents<'a>(commits: impl Iterator<Item = (&'a str, &'a [String])>) -> Vec<Row> {
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut next_color = 0;
    let free = |lanes: &Vec<Option<Lane>>| lanes.iter().position(Option::is_none).unwrap_or(lanes.len());
    let mut rows = Vec::new();
    for (sha, parents) in commits {
        let mut lines = Vec::new();
        let found = lanes.iter().position(|l| l.as_ref().is_some_and(|l| l.sha == sha));
        let tip = found.is_none();
        let lane = found.unwrap_or_else(|| {
            let i = free(&lanes);
            let new = Some(Lane { sha: sha.to_owned(), color: next_color });
            next_color += 1;
            if i == lanes.len() { lanes.push(new) } else { lanes[i] = new }
            i
        });
        let color = lanes[lane].as_ref().map_or(0, |l| l.color);

        // Top half: lanes waiting for this commit end at its dot; the others pass through.
        // A copy, as lanes merging into this commit are freed along the way.
        for (i, l) in lanes.clone().into_iter().enumerate() {
            let Some(l) = l else { continue };
            if l.sha != sha {
                lines.push(Line { from: i, to: i, part: Part::Through, color: l.color });
            } else if !(tip && i == lane) {
                lines.push(Line { from: i, to: lane, part: Part::Top, color: l.color });
                if i != lane {
                    lanes[i] = None;
                }
            }
        }

        // Bottom half: the parents.
        let set = |lanes: &mut Vec<Option<Lane>>, i: usize, value: Option<Lane>| {
            if i == lanes.len() { lanes.push(value) } else { lanes[i] = value }
        };
        match parents.split_first() {
            Some((first, others)) => {
                let existing = lanes.iter().enumerate().position(|(i, l)| i != lane && l.as_ref().is_some_and(|l| &l.sha == first));
                match existing {
                    None => {
                        lanes[lane] = Some(Lane { sha: first.clone(), color });
                        lines.push(Line { from: lane, to: lane, part: Part::Bottom, color });
                    }
                    Some(e) => {
                        // Another lane already waits for the parent (a branch point seen from above): join it.
                        lines.push(Line { from: lane, to: e, part: Part::Bottom, color: lanes[e].as_ref().map_or(0, |l| l.color) });
                        lanes[lane] = None;
                    }
                }
                for parent in others {
                    let target = match lanes.iter().position(|l| l.as_ref().is_some_and(|l| &l.sha == parent)) {
                        Some(t) => t,
                        None => {
                            let t = free(&lanes);
                            set(&mut lanes, t, Some(Lane { sha: parent.clone(), color: next_color }));
                            next_color += 1;
                            t
                        }
                    };
                    lines.push(Line { from: lane, to: target, part: Part::Bottom, color: lanes[target].as_ref().map_or(0, |l| l.color) });
                }
            }
            None => lanes[lane] = None,
        }

        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        let width = lines.iter().flat_map(|l| [l.from, l.to]).chain([lane]).max().unwrap_or(0) + 1;
        rows.push(Row { lane, color, lines, width });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(history: &[(&str, &[&str])]) -> Vec<Row> {
        let owned: Vec<(String, Vec<String>)> = history.iter().map(|(s, p)| (s.to_string(), p.iter().map(|p| p.to_string()).collect())).collect();
        layout_parents(owned.iter().map(|(s, p)| (s.as_str(), p.as_slice())))
    }

    #[test]
    fn a_straight_line_stays_in_one_lane() {
        let r = rows(&[("c", &["b"]), ("b", &["a"]), ("a", &[])]);
        assert!(r.iter().all(|row| row.lane == 0 && row.width == 1));
        assert_eq!(r[0].lines, [Line { from: 0, to: 0, part: Part::Bottom, color: 0 }]);
        assert_eq!(r[2].lines, [Line { from: 0, to: 0, part: Part::Top, color: 0 }]);
    }

    #[test]
    fn a_merge_opens_a_lane_that_closes_at_the_fork() {
        // m merges b2 (side branch) into a2; both come from a1.
        let r = rows(&[("m", &["a2", "b2"]), ("b2", &["a1"]), ("a2", &["a1"]), ("a1", &[])]);
        assert_eq!(r[0].lane, 0);
        assert!(r[0].lines.contains(&Line { from: 0, to: 1, part: Part::Bottom, color: 1 }));
        // b2 is newer by date, so it comes first, in the merge's second lane.
        assert_eq!(r[1].lane, 1);
        assert_eq!(r[2].lane, 0);
        // a2 joins the lane already waiting for a1, where the two meet.
        assert_eq!(r[2].lines, [Line { from: 0, to: 0, part: Part::Top, color: 0 }, Line { from: 1, to: 1, part: Part::Through, color: 1 }, Line { from: 0, to: 1, part: Part::Bottom, color: 1 }]);
        assert_eq!(r[3].lane, 1);
        assert_eq!(r[3].lines, [Line { from: 1, to: 1, part: Part::Top, color: 1 }]);
    }
}

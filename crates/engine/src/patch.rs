//! Custom patches (lazygit's, at line level): changes picked line by line from one commit's diff, turned into a
//! patch that takes them back out of the commit's version of each file. Pure, so it is unit tested here; applying
//! it is in `rebase` (out of the commit, into another one, into the working tree).
//!
//! Picking works by change block (a run of removed and added lines between context lines), as VS Code's "Stage
//! Selected Ranges" does: picking some added lines of a block also picks the lines they replaced, and picking only
//! removed lines also picks the lines that replaced them.

use std::collections::BTreeSet;

use crate::diff::{DiffFile, Hunk, LineKind};

/// Picked lines of one file: (hunk index, line index in the hunk).
pub type Picks = BTreeSet<(usize, usize)>;

/// Every change of a file.
pub fn all_changes(file: &DiffFile) -> Picks {
    let mut picks = Picks::new();
    for (h, hunk) in file.hunks.iter().enumerate() {
        for (i, line) in hunk.lines.iter().enumerate() {
            if matches!(line.kind, LineKind::Added | LineKind::Removed) {
                picks.insert((h, i));
            }
        }
    }
    picks
}

/// Runs of changed lines in a hunk, as line indexes.
fn blocks(hunk: &Hunk) -> Vec<Vec<usize>> {
    let mut blocks: Vec<Vec<usize>> = Vec::new();
    let mut open = false;
    for (i, line) in hunk.lines.iter().enumerate() {
        match line.kind {
            LineKind::Added | LineKind::Removed => {
                if !open {
                    blocks.push(Vec::new());
                    open = true;
                }
                blocks.last_mut().unwrap().push(i);
            }
            LineKind::Context => open = false,
            LineKind::Note => {}
        }
    }
    blocks
}

/// The picks with each touched block's other side added.
pub fn effective(file: &DiffFile, picks: &Picks) -> Picks {
    let mut out = Picks::new();
    for (h, hunk) in file.hunks.iter().enumerate() {
        for block in blocks(hunk) {
            let kind = |i: usize| hunk.lines[i].kind;
            let picked: Vec<usize> = block.iter().copied().filter(|&i| picks.contains(&(h, i))).collect();
            if picked.is_empty() {
                continue;
            }
            let added = picked.iter().any(|&i| kind(i) == LineKind::Added);
            let removed = picked.iter().any(|&i| kind(i) == LineKind::Removed);
            for &i in &block {
                let other_side = (added && !removed && kind(i) == LineKind::Removed) || (removed && !added && kind(i) == LineKind::Added);
                if picks.contains(&(h, i)) || other_side {
                    out.insert((h, i));
                }
            }
        }
    }
    out
}

/// How many changed lines the picks come to, once blocks are filled in.
pub fn picked_lines(file: &DiffFile, picks: &Picks) -> usize {
    effective(file, picks).len()
}

/// A patch that, applied to the commit's version of the file, takes the picked changes back out; None when the picks
/// change nothing. Binary files and files the commit deletes can't be split.
pub fn removal_patch(file: &DiffFile, picks: &Picks) -> Result<Option<String>, String> {
    if file.binary {
        return Err(format!("{} is binary: it can't be split by lines.", file.path));
    }
    if file.status == 'D' {
        return Err(format!("The commit deletes {}: there are no lines of it left to take out.", file.path));
    }
    let picked = effective(file, picks);
    if picked.is_empty() {
        return Ok(None);
    }
    // Taking every added line out of a file the commit added removes the file.
    let removes_file = file.status == 'A' && file.hunks.iter().enumerate().all(|(h, hunk)| hunk.lines.iter().enumerate().all(|(i, l)| l.kind != LineKind::Added || picked.contains(&(h, i))));

    let mut out: Vec<String> = Vec::new();
    // How much the hunks so far have changed line numbers by.
    let mut delta: i64 = 0;
    for (h, hunk) in file.hunks.iter().enumerate() {
        let mut lines = Vec::new();
        let mut start: Option<u32> = None;
        let (mut old_count, mut new_count) = (0i64, 0i64);
        let mut changed = false;
        let mut last_kept = false;
        for (i, line) in hunk.lines.iter().enumerate() {
            let pick = picked.contains(&(h, i));
            match line.kind {
                LineKind::Note => {
                    if last_kept {
                        lines.push(format!("\\ {}", line.text));
                    }
                    continue;
                }
                // Lines of the commit that stay.
                LineKind::Context => lines.push(format!(" {}", line.text)),
                LineKind::Added if !pick => lines.push(format!(" {}", line.text)),
                // A picked added line comes out.
                LineKind::Added => {
                    lines.push(format!("-{}", line.text));
                    changed = true;
                }
                // A picked removed line goes back in.
                LineKind::Removed if pick => {
                    lines.push(format!("+{}", line.text));
                    new_count += 1;
                    changed = true;
                    last_kept = true;
                    continue;
                }
                LineKind::Removed => {
                    last_kept = false;
                    continue;
                }
            }
            last_kept = true;
            start.get_or_insert(line.new.unwrap_or(1));
            old_count += 1;
            if line.kind == LineKind::Context || !pick {
                new_count += 1;
            }
        }
        if !changed {
            continue;
        }
        // A hunk with no lines on the commit's side sits at the line after it; an empty file starts at 1.
        let start = start.unwrap_or(1) as i64;
        let new_start = start + delta;
        let at = |s: i64, count: i64| if count > 0 { s } else { s - 1 };
        out.push(format!("@@ -{},{old_count} +{},{new_count} @@", at(start, old_count), at(new_start, new_count)));
        out.extend(lines);
        delta += new_count - old_count;
    }
    if out.is_empty() {
        return Ok(None);
    }
    let path = &file.path;
    let mut header = vec![format!("diff --git a/{path} b/{path}")];
    if removes_file {
        header.push(format!("deleted file mode {}", file.new_mode.as_deref().unwrap_or("100644")));
        header.extend([format!("--- a/{path}"), "+++ /dev/null".to_owned()]);
    } else {
        header.extend([format!("--- a/{path}"), format!("+++ b/{path}")]);
    }
    header.extend(out);
    Ok(Some(header.join("\n") + "\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::parse_diff;

    const DIFF: &str = "diff --git a/f.txt b/f.txt
index 1111111..2222222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,5 +1,6 @@
 one
-two
+TWO
+two and a half
 three
 four
+five
";

    fn picks(list: &[(usize, usize)]) -> Picks {
        list.iter().copied().collect()
    }

    #[test]
    fn picking_an_added_line_takes_the_line_it_replaced_along() {
        let file = &parse_diff(DIFF)[0];
        // Line 2 of the hunk is "+TWO".
        assert_eq!(effective(file, &picks(&[(0, 2)])), picks(&[(0, 1), (0, 2)]));
        let patch = removal_patch(file, &picks(&[(0, 2)])).unwrap().unwrap();
        assert_eq!(
            patch,
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,6 +1,6 @@\n one\n+two\n-TWO\n two and a half\n three\n four\n five\n"
        );
    }

    #[test]
    fn a_pure_addition_comes_out_alone() {
        let file = &parse_diff(DIFF)[0];
        let patch = removal_patch(file, &picks(&[(0, 6)])).unwrap().unwrap();
        assert!(patch.ends_with("@@ -1,6 +1,5 @@\n one\n TWO\n two and a half\n three\n four\n-five\n"), "{patch}");
        assert_eq!(removal_patch(file, &picks(&[(0, 0)])).unwrap(), None, "context lines are not changes");
    }

    #[test]
    fn taking_everything_out_of_an_added_file_deletes_it() {
        let added = "diff --git a/n.txt b/n.txt\nnew file mode 100755\nindex 0000000..3333333\n--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1,2 @@\n+a\n+b\n";
        let file = &parse_diff(added)[0];
        let patch = removal_patch(file, &all_changes(file)).unwrap().unwrap();
        assert!(patch.contains("deleted file mode 100755\n--- a/n.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n"), "{patch}");
        assert!(removal_patch(file, &picks(&[(0, 0)])).unwrap().unwrap().contains("--- a/n.txt\n+++ b/n.txt"));
    }
}

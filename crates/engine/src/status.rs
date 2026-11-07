//! The working tree and index: the Files panel.

use crate::git::{Exec, Git, Result};

/// One path from `git status`, with git's two status letters: X for the index, Y for the working tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    /// Where a staged rename or copy came from.
    pub old_path: Option<String>,
    pub x: char,
    pub y: char,
}

impl StatusEntry {
    pub fn is_conflict(&self) -> bool {
        matches!((self.x, self.y), ('D', 'D') | ('A', 'U') | ('U', 'D') | ('U', 'A') | ('D', 'U') | ('A', 'A') | ('U', 'U'))
    }

    pub fn is_untracked(&self) -> bool {
        self.x == '?'
    }

    /// Has changes in the index.
    pub fn is_staged(&self) -> bool {
        !self.is_conflict() && !self.is_untracked() && self.x != ' '
    }

    /// Has working tree changes that are not staged, untracked files included.
    pub fn is_unstaged(&self) -> bool {
        !self.is_conflict() && (self.is_untracked() || self.y != ' ')
    }

    fn paths(&self) -> impl Iterator<Item = &str> {
        self.old_path.as_deref().into_iter().chain(std::iter::once(self.path.as_str()))
    }
}

/// Parses `git status --porcelain=v1 -z`. A rename or copy is "XY new\0old\0". Conflicts first, then by path.
pub fn parse_status(output: &str) -> Vec<StatusEntry> {
    let tokens: Vec<&str> = output.split('\0').collect();
    let mut entries = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        i += 1;
        if token.len() < 4 {
            continue;
        }
        let mut chars = token.chars();
        let x = chars.next().unwrap();
        let y = chars.next().unwrap();
        let mut entry = StatusEntry { x, y, path: token[3..].to_owned(), old_path: None };
        if x == 'R' || x == 'C' {
            entry.old_path = tokens.get(i).map(|s| s.to_string());
            i += 1;
        }
        entries.push(entry);
    }
    entries.sort_by(|a, b| b.is_conflict().cmp(&a.is_conflict()).then_with(|| a.path.cmp(&b.path)));
    entries
}

pub fn status(git: &Git) -> Result<Vec<StatusEntry>> {
    Ok(parse_status(&git.run(["-c", "core.quotePath=false", "status", "--porcelain=v1", "-z", "--untracked-files=all"])?))
}

pub fn has_head(git: &Git) -> Result<bool> {
    Ok(git.exec(["rev-parse", "-q", "--verify", "HEAD"], Exec { allowed: &[1, 128], ..Default::default() })?.code == 0)
}

fn unique_paths(entries: &[StatusEntry]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for p in entries.iter().flat_map(StatusEntry::paths) {
        if !paths.iter().any(|q| q == p) {
            paths.push(p.to_owned());
        }
    }
    paths
}

fn with_paths(args: &[&str], paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut all: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    all.push("--".to_owned());
    all.extend(paths);
    all
}

pub fn stage(git: &Git, entries: &[StatusEntry]) -> Result<()> {
    if !entries.is_empty() {
        git.run(with_paths(&["add", "-A"], unique_paths(entries)))?;
    }
    Ok(())
}

pub fn unstage(git: &Git, entries: &[StatusEntry]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    // Before the first commit there is no HEAD to restore the index from.
    if has_head(git)? {
        git.run(with_paths(&["restore", "--staged"], unique_paths(entries)))?;
    } else {
        git.run(with_paths(&["rm", "-q", "-r", "--cached"], unique_paths(entries)))?;
    }
    Ok(())
}

/// Throws away a file's changes, staged and unstaged: tracked files go back to HEAD and files new since HEAD are
/// deleted. With `unstaged_only`, only the working tree goes back to the index (untracked files are still deleted).
pub fn discard(git: &Git, entries: &[StatusEntry], unstaged_only: bool) -> Result<()> {
    let (untracked, tracked): (Vec<_>, Vec<_>) = entries.iter().cloned().partition(StatusEntry::is_untracked);
    if !untracked.is_empty() {
        git.run(with_paths(&["clean", "-f", "-q"], untracked.into_iter().map(|e| e.path)))?;
    }
    if tracked.is_empty() {
        return Ok(());
    }
    if unstaged_only {
        let changed: Vec<String> = tracked.into_iter().filter(|e| e.y != ' ').map(|e| e.path).collect();
        if !changed.is_empty() {
            git.run(with_paths(&["restore", "--worktree"], changed))?;
        }
        return Ok(());
    }
    // Added in the index (or the new side of a rename): not in HEAD, so removing it is how it goes back.
    let (added, in_head): (Vec<_>, Vec<_>) = tracked.into_iter().partition(|e| matches!(e.x, 'A' | 'R' | 'C'));
    let mut restore: Vec<String> = in_head.into_iter().map(|e| e.path).collect();
    restore.extend(added.iter().filter(|e| e.x == 'R').filter_map(|e| e.old_path.clone()));
    if !added.is_empty() {
        git.run(with_paths(&["rm", "-q", "-f", "-r"], added.into_iter().map(|e| e.path)))?;
    }
    if !restore.is_empty() {
        if has_head(git)? {
            git.run(with_paths(&["restore", "--source=HEAD", "--staged", "--worktree"], restore))?;
        } else {
            git.run(with_paths(&["rm", "-q", "-f", "-r"], restore))?;
        }
    }
    Ok(())
}

/// A file's changes: `staged` against HEAD, otherwise the working tree against the index.
pub fn working_diff(git: &Git, entry: &StatusEntry, staged: bool) -> Result<String> {
    if entry.is_untracked() {
        return Ok(untracked_diff(git, &entry.path));
    }
    let mut args = vec!["-c", "core.quotePath=false", "diff", "--no-color", "--no-ext-diff", "-M"];
    if staged {
        args.push("--cached");
    }
    let paths: Vec<String> = if staged { entry.paths().map(str::to_owned).collect() } else { vec![entry.path.clone()] };
    git.run(with_paths(&args, paths))
}

/// An untracked file as an all-added diff, without running git.
fn untracked_diff(git: &Git, file: &str) -> String {
    let header = format!("diff --git a/{file} b/{file}\nnew file mode 100644\n--- /dev/null\n+++ b/{file}\n");
    let Ok(content) = std::fs::read(git.root().join(file)) else { return header };
    if content.contains(&0) {
        return format!("diff --git a/{file} b/{file}\nnew file mode 100644\nBinary files /dev/null and b/{file} differ\n");
    }
    let text = String::from_utf8_lossy(&content).replace("\r\n", "\n");
    if text.is_empty() {
        return header;
    }
    let missing_newline = !text.ends_with('\n');
    let lines: Vec<&str> = text.strip_suffix('\n').unwrap_or(&text).split('\n').collect();
    let mut out = format!("{header}@@ -0,0 +1,{} @@\n", lines.len());
    for line in lines {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    if missing_newline {
        out.push_str("\\ No newline at end of file\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status() {
        let out = " M src/a.rs\0R  new.rs\0old.rs\0?? notes.txt\0UU conflict.rs\0A  added.rs\0";
        let entries = parse_status(out);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].path, "conflict.rs");
        assert!(entries[0].is_conflict());
        let renamed = entries.iter().find(|e| e.path == "new.rs").unwrap();
        assert_eq!(renamed.old_path.as_deref(), Some("old.rs"));
        assert!(renamed.is_staged() && !renamed.is_unstaged());
        let untracked = entries.iter().find(|e| e.path == "notes.txt").unwrap();
        assert!(untracked.is_untracked() && untracked.is_unstaged() && !untracked.is_staged());
        let modified = entries.iter().find(|e| e.path == "src/a.rs").unwrap();
        assert!(!modified.is_staged() && modified.is_unstaged());
    }
}

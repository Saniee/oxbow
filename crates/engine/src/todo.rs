//! git's interactive rebase todo list, read and rewritten in place. Every Oxbow rebase starts stopped before its
//! first step (see [`crate::editor`]), rewrites `rebase-merge/git-rebase-todo` with [`transform_todo`], then
//! continues, so git never waits on an editor.
//!
//! A new commit message is applied by an `exec` step right after the commit's step (see [`reword_exec`]). That exec
//! line travels with its step when steps are moved, and goes away when the step becomes drop, squash, fixup or edit.
//!
//! With --rebase-merges the list also has `label`, `reset` and `merge` lines that rebuild the branch structure.
//! Changes that would cross them (squashing into another branch's commit, moving a commit past a merge or a branch
//! point) are refused. A merge step can be reworded, edited (a `break` after it) or dropped, but not squashed or moved.
//!
//! With --update-refs (stacked branches) it has `update-ref` lines, each after the commit that branch points at.
//! Moving a commit across one moves the commit into the other branch.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TodoAction {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

impl TodoAction {
    pub const ALL: [TodoAction; 6] = [TodoAction::Pick, TodoAction::Reword, TodoAction::Edit, TodoAction::Squash, TodoAction::Fixup, TodoAction::Drop];

    pub fn name(self) -> &'static str {
        match self {
            TodoAction::Pick => "pick",
            TodoAction::Reword => "reword",
            TodoAction::Edit => "edit",
            TodoAction::Squash => "squash",
            TodoAction::Fixup => "fixup",
            TodoAction::Drop => "drop",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            TodoAction::Pick => "Keep the commit",
            TodoAction::Reword => "Keep the commit, with a new message",
            TodoAction::Edit => "Stop after applying it, to change it",
            TodoAction::Squash => "Meld into the step before, combining both messages",
            TodoAction::Fixup => "Meld into the step before, keeping that message",
            TodoAction::Drop => "Leave the commit out",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "p" | "pick" => TodoAction::Pick,
            "r" | "reword" => TodoAction::Reword,
            "e" | "edit" => TodoAction::Edit,
            "s" | "squash" => TodoAction::Squash,
            "f" | "fixup" => TodoAction::Fixup,
            "d" | "drop" => TodoAction::Drop,
            _ => return None,
        })
    }
}

/// Up is towards newer commits, as in the commit list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

#[derive(Clone, Debug, Default)]
pub struct TodoPlan {
    /// Full (or todo-length) SHA and its new action.
    pub actions: Vec<(String, TodoAction)>,
    /// Swap a commit with its neighbour.
    pub move_commit: Option<(String, Direction)>,
    /// Move the first commit directly after the second and mark it as a fixup.
    pub fixup_into: Option<(String, String)>,
    /// Stop before the first step (a `break` line), so the steps can be changed before anything is applied.
    pub pause_first: bool,
    /// Commits whose new message has been written to [`message_file`]: an exec step after the commit's step amends it.
    pub reword: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepKind {
    Commit(TodoAction),
    /// Recreates a merge commit; can't be changed here.
    Merge,
    /// Moves a stacked branch (its full ref name); can't be changed.
    UpdateRef(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodoEntry {
    pub kind: StepKind,
    /// Abbreviated SHA as written in the list; for a merge, the original merge commit's (may be empty).
    pub sha: String,
    pub subject: String,
}

impl TodoEntry {
    pub fn action(&self) -> Option<TodoAction> {
        match self.kind {
            StepKind::Commit(a) => Some(a),
            _ => None,
        }
    }
}

/// Where a reworded step's new message waits; git deletes the directory when the rebase ends.
pub fn message_file(git_dir: &Path, sha: &str) -> PathBuf {
    git_dir.join("rebase-merge").join(format!("oxbow-message-{}", sha.to_lowercase()))
}

const REWORD_TAGS: [&str; 2] = ["# oxbow reword ", "# slothgit reword "];

/// The exec step that gives the commit just applied the message saved for `sha`. Exec lines run in git's shell, so
/// the path is single-quoted.
pub fn reword_exec(git_dir: &Path, sha: &str) -> String {
    let path = message_file(git_dir, sha).to_string_lossy().replace('\\', "/").replace('\'', r"'\''");
    format!("exec git commit --amend --only --allow-empty --cleanup=strip -F '{path}' {}{}", REWORD_TAGS[0], sha.to_lowercase())
}

fn is_sha(s: &str) -> bool {
    (4..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The commit an Oxbow (or SlothGit) reword exec line belongs to.
fn reword_exec_sha(line: &str) -> Option<&str> {
    if !line.starts_with("exec git commit --amend ") {
        return None;
    }
    REWORD_TAGS.iter().find_map(|tag| line.rfind(tag).map(|i| line[i + tag.len()..].trim())).filter(|s| is_sha(s))
}

fn split_word(line: &str) -> Option<(&str, &str)> {
    let end = line.find(char::is_whitespace)?;
    Some((&line[..end], &line[end..]))
}

/// `pick <sha> <rest>`: the action, the lowercased SHA and what follows it.
fn commit_line(line: &str) -> Option<(TodoAction, String, &str)> {
    let (word, after) = split_word(line)?;
    let action = TodoAction::parse(word)?;
    let after = after.trim_start();
    let end = after.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(after.len());
    let (sha, rest) = after.split_at(end);
    if !is_sha(sha) || rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some((action, sha.to_lowercase(), rest))
}

/// `merge [-C <sha> | -c <sha>] <label> [# <subject>]`: the SHA, the label and what follows.
fn merge_line(line: &str) -> Option<(Option<String>, &str, &str)> {
    let (word, after) = split_word(line)?;
    if word != "merge" && word != "m" {
        return None;
    }
    let mut rest = after.trim_start();
    let mut sha = None;
    if let Some(r) = rest.strip_prefix("-C").or_else(|| rest.strip_prefix("-c"))
        && r.starts_with(char::is_whitespace)
    {
        let (s, r) = split_word(r.trim_start())?;
        if !is_sha(s) {
            return None;
        }
        sha = Some(s.to_lowercase());
        rest = r.trim_start();
    }
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    Some((sha, &rest[..end], &rest[end..]))
}

/// Lines that rebuild the branch structure; HEAD jumps at them, so commits can't be squashed or moved across.
fn is_structure(line: &str) -> bool {
    split_word(line).is_some_and(|(w, _)| matches!(w, "label" | "l" | "reset" | "t" | "merge" | "m"))
}

fn update_ref(line: &str) -> Option<&str> {
    let (word, after) = split_word(line)?;
    if word != "update-ref" && word != "u" {
        return None;
    }
    after.split_whitespace().next()
}

/// " <subject>" or " # <subject>", depending on git's version.
fn subject_of(rest: &str) -> String {
    let rest = rest.trim_start();
    rest.strip_prefix('#').unwrap_or(rest).trim().to_owned()
}

/// Commit, merge and update-ref steps of a todo list, next one first.
pub fn parse_todo(todo: &str) -> Vec<TodoEntry> {
    let mut entries: Vec<TodoEntry> = Vec::new();
    for line in todo.lines() {
        // A pick followed by Oxbow's reword exec is shown as the reword it is.
        if let Some(sha) = reword_exec_sha(line) {
            if let Some(last) = entries.last_mut()
                && last.kind == StepKind::Commit(TodoAction::Pick)
                && sha.to_lowercase().starts_with(&last.sha)
            {
                last.kind = StepKind::Commit(TodoAction::Reword);
            }
            continue;
        }
        if let Some((action, sha, rest)) = commit_line(line) {
            entries.push(TodoEntry { kind: StepKind::Commit(action), sha, subject: subject_of(rest) });
        } else if let Some((sha, label, rest)) = merge_line(line) {
            let subject = subject_of(rest);
            let subject = if subject.is_empty() { format!("Merge {label}") } else { subject };
            entries.push(TodoEntry { kind: StepKind::Merge, sha: sha.unwrap_or_default(), subject });
        } else if let Some(r) = update_ref(line) {
            let branch = r.strip_prefix("refs/heads/").unwrap_or(r);
            entries.push(TodoEntry { kind: StepKind::UpdateRef(r.to_owned()), sha: String::new(), subject: format!("Update branch {branch}") });
        }
    }
    entries
}

/// The commit a pick-like or merge step applies, as written in the list.
fn step_sha(line: &str) -> Option<String> {
    commit_line(line).map(|(_, sha, _)| sha).or_else(|| merge_line(line).and_then(|(sha, _, _)| sha))
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

/// Applies `plan` to `todo`. `git_dir` is where reword messages are kept.
pub fn transform_todo(todo: &str, plan: &TodoPlan, git_dir: &Path) -> Result<String, String> {
    let eol = if todo.contains("\r\n") { "\r\n" } else { "\n" };
    // Reword execs are taken out here and put back after their commit's step at the end, wherever it moved.
    let mut reword_execs: Vec<(String, String)> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for line in todo.lines() {
        match reword_exec_sha(line) {
            Some(sha) => reword_execs.push((sha.to_lowercase(), line.to_owned())),
            None => lines.push(line.to_owned()),
        }
    }
    // Commit steps by line, with the text after their SHA; and the SHAs of merge steps.
    let parsed: Vec<(usize, String, String)> =
        lines.iter().enumerate().filter_map(|(i, l)| commit_line(l).map(|(_, sha, rest)| (i, sha, rest.to_owned()))).collect();
    let merges: Vec<String> = lines.iter().filter_map(|l| merge_line(l).and_then(|(sha, _, _)| sha)).collect();
    let is_merge = |full: &str| merges.iter().any(|m| full.to_lowercase().starts_with(m.as_str()));
    // Position in `parsed` of the commit `full`.
    let find = |full: &str| -> Result<usize, String> {
        let lower = full.to_lowercase();
        if let Some(p) = parsed.iter().position(|(_, sha, _)| lower.starts_with(sha.as_str())) {
            return Ok(p);
        }
        if is_merge(full) {
            return Err(format!("{} is a merge commit, which can't be squashed or moved", short(full)));
        }
        Err(format!("commit {} is not in the rebase todo list", short(full)))
    };
    let render = |p: usize, action: TodoAction| format!("{} {}{}", action.name(), parsed[p].1, parsed[p].2);
    let structure_between = |lines: &[String], a: usize, b: usize| lines[a.min(b) + 1..a.max(b)].iter().any(|l| is_structure(l));

    // Merge steps are changed last, as editing one adds a line.
    let mut merge_actions: Vec<(String, TodoAction)> = Vec::new();
    for (sha, action) in &plan.actions {
        let melds = matches!(action, TodoAction::Squash | TodoAction::Fixup);
        if is_merge(sha) && !melds {
            merge_actions.push((sha.to_lowercase(), *action));
            continue;
        }
        let p = find(sha)?;
        if melds {
            // Melded into whatever was applied just before, which must be its parent on the same branch.
            if p == 0 || structure_between(&lines, parsed[p - 1].0, parsed[p].0) {
                return Err(format!("cannot {} {}: the commit before it is on another branch or is a merge", action.name(), short(sha)));
            }
        }
        lines[parsed[p].0] = render(p, *action);
    }

    if let Some((sha, direction)) = &plan.move_commit {
        let p = find(sha)?;
        let other = match direction {
            Direction::Up => p + 1,
            Direction::Down => p.wrapping_sub(1),
        };
        let word = if *direction == Direction::Up { "up" } else { "down" };
        let Some(other) = parsed.get(other).map(|o| o.0) else { return Err(format!("cannot move commit {} {word}", short(sha))) };
        let i = parsed[p].0;
        if structure_between(&lines, i, other) {
            return Err(format!("cannot move commit {} {word} past a merge or branch point", short(sha)));
        }
        if lines[i.min(other) + 1..i.max(other)].iter().any(|l| update_ref(l).is_some()) {
            // Into the neighbouring branch: next to `other`, on this side of it.
            let line = lines.remove(i);
            lines.insert(if i < other { other - 1 } else { other + 1 }, line);
        } else {
            lines.swap(i, other);
        }
    }

    if let Some((sha, target)) = &plan.fixup_into {
        let p = find(sha)?;
        let t = parsed[find(target)?].0;
        let i = parsed[p].0;
        let line = render(p, TodoAction::Fixup);
        lines.remove(i);
        // Directly after the target, before any label that marks it as a branch point.
        lines.insert(if i < t { t } else { t + 1 }, line);
    }

    for (sha, action) in merge_actions {
        let Some(i) = lines.iter().position(|l| merge_line(l).and_then(|(s, _, _)| s).is_some_and(|s| sha.starts_with(&s))) else { continue };
        match action {
            TodoAction::Drop => {
                lines.remove(i);
            }
            // -c (lowercase) asks for the message instead of reusing the original's.
            TodoAction::Reword => lines[i] = replace_merge_flag(&lines[i], "-C", "-c"),
            TodoAction::Edit => lines.insert(i + 1, "break".to_owned()),
            _ => {}
        }
    }

    for sha in &plan.reword {
        let full = sha.to_lowercase();
        let i = lines
            .iter()
            .position(|l| step_sha(l).is_some_and(|s| full.starts_with(&s)))
            .ok_or_else(|| format!("commit {} is not in the rebase todo list", short(sha)))?;
        if let Some((TodoAction::Reword | TodoAction::Edit, s, rest)) = commit_line(&lines[i]) {
            lines[i] = format!("pick {s}{rest}");
        } else if merge_line(&lines[i]).is_some() {
            lines[i] = replace_merge_flag(&lines[i], "-c", "-C");
        }
        reword_execs.retain(|(key, _)| !full.starts_with(key.as_str()));
        reword_execs.push((full.clone(), reword_exec(git_dir, &full)));
    }

    // Each reword exec goes back right after its step, while that step still applies the commit (pick or merge).
    for (sha, exec) in reword_execs {
        let Some(i) = lines.iter().position(|l| step_sha(l).is_some_and(|s| sha.starts_with(&s) || s.starts_with(&sha))) else { continue };
        let keeps = commit_line(&lines[i]).is_some_and(|(a, _, _)| a == TodoAction::Pick) || merge_line(&lines[i]).is_some();
        if keeps {
            lines.insert(i + 1, exec);
        }
    }

    if plan.pause_first {
        let first = lines.iter().position(|l| commit_line(l).is_some()).ok_or("there are no commits to rebase")?;
        lines.insert(first, "break".to_owned());
    }

    let mut out = lines.join(eol);
    if todo.ends_with('\n') {
        out.push_str(eol);
    }
    Ok(out)
}

/// `merge -C <sha> …` ⇄ `merge -c <sha> …`.
fn replace_merge_flag(line: &str, from: &str, to: &str) -> String {
    match split_word(line) {
        Some((word, after)) if after.trim_start().starts_with(from) => {
            let after = after.trim_start();
            format!("{word} {to}{}", &after[from.len()..])
        }
        _ => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINEAR: &str = "pick aaaa111 First\npick bbbb222 Second\npick cccc333 Third\n\n# Rebase 1234..cccc333 onto 1234\n";

    fn dir() -> PathBuf {
        PathBuf::from("/repo/.git")
    }

    fn plan_actions(actions: &[(&str, TodoAction)]) -> TodoPlan {
        TodoPlan { actions: actions.iter().map(|(s, a)| ((*s).to_owned(), *a)).collect(), ..Default::default() }
    }

    fn commands(todo: &str) -> Vec<String> {
        todo.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| l.split(" #").next().unwrap().to_owned()).collect()
    }

    #[test]
    fn changes_actions_by_full_sha() {
        let out = transform_todo(LINEAR, &plan_actions(&[("bbbb2220000", TodoAction::Squash), ("cccc333", TodoAction::Drop)]), &dir()).unwrap();
        assert_eq!(commands(&out), ["pick aaaa111 First", "squash bbbb222 Second", "drop cccc333 Third"]);
        assert!(out.ends_with("onto 1234\n"));
    }

    #[test]
    fn refuses_to_squash_the_first_step() {
        assert!(transform_todo(LINEAR, &plan_actions(&[("aaaa111", TodoAction::Fixup)]), &dir()).is_err());
    }

    #[test]
    fn moves_commits() {
        let up = TodoPlan { move_commit: Some(("aaaa111".into(), Direction::Up)), ..Default::default() };
        assert_eq!(commands(&transform_todo(LINEAR, &up, &dir()).unwrap()), ["pick bbbb222 Second", "pick aaaa111 First", "pick cccc333 Third"]);
        let down = TodoPlan { move_commit: Some(("aaaa111".into(), Direction::Down)), ..Default::default() };
        assert!(transform_todo(LINEAR, &down, &dir()).is_err());
    }

    #[test]
    fn fixup_into_an_older_commit() {
        let plan = TodoPlan { fixup_into: Some(("cccc333".into(), "aaaa111".into())), ..Default::default() };
        assert_eq!(commands(&transform_todo(LINEAR, &plan, &dir()).unwrap()), ["pick aaaa111 First", "fixup cccc333 Third", "pick bbbb222 Second"]);
    }

    #[test]
    fn reword_adds_an_exec_that_moves_with_its_step() {
        let plan = TodoPlan { reword: vec!["bbbb222ffff".into()], ..Default::default() };
        let out = transform_todo(LINEAR, &plan, &dir()).unwrap();
        let lines = commands(&out);
        assert_eq!(lines[1], "pick bbbb222 Second");
        assert!(lines[2].starts_with("exec git commit --amend") && lines[2].contains("'/repo/.git/rebase-merge/oxbow-message-bbbb222ffff'"));
        let entries = parse_todo(&out);
        assert_eq!(entries[1].kind, StepKind::Commit(TodoAction::Reword));
        assert_eq!(entries.len(), 3);

        let moved = transform_todo(&out, &TodoPlan { move_commit: Some(("bbbb222".into(), Direction::Up)), ..Default::default() }, &dir()).unwrap();
        let lines = commands(&moved);
        assert_eq!(lines[1], "pick cccc333 Third");
        assert_eq!(lines[2], "pick bbbb222 Second");
        assert!(lines[3].starts_with("exec "));

        // Dropping it takes the exec away.
        let dropped = transform_todo(&out, &plan_actions(&[("bbbb222", TodoAction::Drop)]), &dir()).unwrap();
        assert!(!dropped.contains("exec "));
    }

    #[test]
    fn pause_first_puts_a_break_before_the_first_step() {
        let plan = TodoPlan { pause_first: true, ..Default::default() };
        assert_eq!(commands(&transform_todo(LINEAR, &plan, &dir()).unwrap())[0], "break");
    }

    const MERGES: &str = "label onto\n\nreset onto\npick aaaa111 Side work\nlabel side\n\nreset onto\npick bbbb222 Main work\nmerge -C dddd444 side # Merge branch 'side'\npick cccc333 After merge\nupdate-ref refs/heads/stacked\n";

    #[test]
    fn rebase_merges_steps_are_guarded() {
        let entries = parse_todo(MERGES);
        assert_eq!(entries.iter().map(|e| e.kind.clone()).collect::<Vec<_>>(), [
            StepKind::Commit(TodoAction::Pick),
            StepKind::Commit(TodoAction::Pick),
            StepKind::Merge,
            StepKind::Commit(TodoAction::Pick),
            StepKind::UpdateRef("refs/heads/stacked".into()),
        ]);
        assert_eq!(entries[2].subject, "Merge branch 'side'");
        // bbbb222's previous commit is on the other branch.
        assert!(transform_todo(MERGES, &plan_actions(&[("bbbb222", TodoAction::Squash)]), &dir()).is_err());
        assert!(transform_todo(MERGES, &TodoPlan { move_commit: Some(("cccc333".into(), Direction::Down)), ..Default::default() }, &dir()).is_err());
        assert!(transform_todo(MERGES, &plan_actions(&[("dddd444", TodoAction::Squash)]), &dir()).is_err());
        let reworded = transform_todo(MERGES, &plan_actions(&[("dddd4440000", TodoAction::Reword)]), &dir()).unwrap();
        assert!(reworded.contains("merge -c dddd444 side"));
        let edited = transform_todo(MERGES, &plan_actions(&[("dddd444", TodoAction::Edit)]), &dir()).unwrap();
        assert!(edited.contains("# Merge branch 'side'\nbreak\n"));
    }
}

//! History rewriting through interactive rebase: squash, fixup, drop, reword, edit, move, amend staged changes into
//! an older commit, and changing the steps of a paused rebase.

use crate::editor;
use crate::git::{Exec, Git, GitError, Result};
use crate::queries::{self, Operation};
use crate::todo::{self, Direction, TodoAction, TodoEntry, TodoPlan};

/// Whether rebases also move branches stacked on the rewritten commits (`--update-refs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateRefs {
    /// git's rebase.updateRefs setting decides.
    #[default]
    GitConfig,
    Always,
    Never,
}

#[derive(Default)]
struct Options<'a> {
    message: Option<&'a str>,
    autosquash: bool,
    onto: Option<&'a str>,
}

/// Rewrites the history of the current branch.
pub struct Rewrite<'a> {
    git: &'a Git,
    update_refs: UpdateRefs,
}

fn with_newline(message: &str) -> String {
    format!("{}\n", message.trim_end())
}

impl<'a> Rewrite<'a> {
    pub fn new(git: &'a Git) -> Self {
        Self { git, update_refs: git.update_refs() }
    }

    pub fn update_refs(mut self, mode: UpdateRefs) -> Self {
        self.update_refs = mode;
        self
    }

    /// `sha~depth`, or None when that commit does not exist (the rebase starts at the root).
    fn ancestor(&self, sha: &str, depth: usize) -> Result<Option<String>> {
        let out = self.git.exec(["rev-parse", "--verify", "-q", &format!("{sha}~{depth}^{{commit}}")], Exec { allowed: &[1, 128], ..Default::default() })?;
        Ok((out.code == 0).then(|| out.stdout.trim().to_owned()))
    }

    fn head(&self) -> Result<String> {
        queries::head_sha(self.git)?.ok_or_else(|| GitError::user("There are no commits yet."))
    }

    fn full_sha(&self, rev: &str) -> Result<String> {
        Ok(self.git.run(["rev-parse", "--verify", &format!("{rev}^{{commit}}")])?.trim().to_owned())
    }

    /// For commits on a linear history, the oldest of them.
    fn oldest(&self, shas: &[String]) -> Result<String> {
        match shas {
            [] => Err(GitError::user("Select a commit first.")),
            [one] => Ok(one.clone()),
            _ => {
                let mut args = vec!["merge-base".to_owned(), "--octopus".to_owned()];
                args.extend(shas.iter().cloned());
                Ok(self.git.run(&args)?.trim().to_owned())
            }
        }
    }

    fn has_merges(&self, base: Option<&str>) -> Result<bool> {
        let range = base.map_or("HEAD".to_owned(), |b| format!("{b}..HEAD"));
        Ok(self.git.run(["rev-list", "--merges", "--count", &range])?.trim() != "0")
    }

    fn merges_among(&self, revs: &[String]) -> Result<bool> {
        if revs.is_empty() {
            return Ok(false);
        }
        let mut args = vec!["rev-list".to_owned(), "--no-walk".to_owned(), "--merges".to_owned()];
        args.extend(revs.iter().cloned());
        Ok(!self.git.run(&args)?.trim().is_empty())
    }

    fn ensure_no_operation(&self) -> Result<()> {
        if let Some(op) = queries::operation(self.git)? {
            return Err(GitError::user(format!("Finish or abort the {} in progress first.", op.name())));
        }
        Ok(())
    }

    /// Runs `git rebase --interactive` from `base` (None: the root) with `plan` applied to its todo list. When the
    /// commits to rewrite include merges, --rebase-merges recreates them.
    fn rebase(&self, base: Option<&str>, mut plan: TodoPlan, options: Options<'_>) -> Result<()> {
        self.ensure_no_operation()?;
        let mut melded: Vec<String> = plan.actions.iter().filter(|(_, a)| matches!(a, TodoAction::Squash | TodoAction::Fixup)).map(|(s, _)| s.clone()).collect();
        melded.extend(plan.move_commit.iter().map(|(s, _)| s.clone()));
        melded.extend(plan.fixup_into.iter().map(|(_, t)| t.clone()));
        if self.merges_among(&melded)? {
            return Err(GitError::user("A merge commit can't be squashed, fixed up, moved or amended with staged changes. It can be reworded, edited or dropped."));
        }
        let mut args: Vec<String> = ["rebase", "--interactive", "--autostash", "--reschedule-failed-exec"].map(String::from).to_vec();
        args.push(if options.autosquash { "--autosquash" } else { "--no-autosquash" }.into());
        if self.has_merges(base)? {
            args.push("--rebase-merges".into());
        }
        // --update-refs is git 2.38's. An older git never moves other branches, so Never needs no flag there, and
        // Always can't be done: say so rather than leave the stacked branches behind unasked.
        let has_update_refs = self.git.version() >= (2, 38);
        match self.update_refs {
            UpdateRefs::GitConfig => {}
            UpdateRefs::Always if !has_update_refs => {
                let (major, minor) = self.git.version();
                return Err(GitError::user(format!(
                    "Moving stacked branches needs git 2.38 or newer, and this is git {major}.{minor}. Update git, or set Move stacked branches to Never in Settings."
                )));
            }
            UpdateRefs::Always => args.push("--update-refs".into()),
            UpdateRefs::Never if !has_update_refs => {}
            UpdateRefs::Never => args.push("--no-update-refs".into()),
        }
        if let Some(onto) = options.onto {
            args.extend(["--onto".into(), onto.to_owned()]);
        }
        args.push(base.map_or("--root".to_owned(), str::to_owned));
        let env = editor::pause_env()?;
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        // Stops at the break the editor puts first, before anything is applied.
        self.start(&args, &env)?;

        let mut prepare = || -> std::result::Result<(), String> {
            let file = self.git.git_path("rebase-merge/git-rebase-todo").map_err(|e| e.to_string())?;
            if !file.exists() {
                return Err("the rebase did not stop before its first step".into());
            }
            let git_dir = self.git.git_dir().map_err(|e| e.to_string())?;
            if let Some(message) = options.message {
                for (sha, action) in &mut plan.actions {
                    if *action == TodoAction::Reword {
                        std::fs::write(todo::message_file(git_dir, sha), with_newline(message)).map_err(|e| e.to_string())?;
                        plan.reword.push(sha.clone());
                    }
                }
            }
            let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
            let text = todo::transform_todo(&text, &plan, git_dir)?;
            std::fs::write(&file, text).map_err(|e| e.to_string())
        };
        if let Err(e) = prepare() {
            let _ = self.git.run(["rebase", "--abort"]);
            return Err(GitError::user(format!("Could not rewrite the history: {e}.")));
        }
        self.git.run(["rebase", "--continue"])?;
        Ok(())
    }

    /// Starts the rebase. On Windows, a virus scanner or indexer that still has the todo file open can make git fail
    /// to replace it ("Permission denied"); that is transient, so the half-started rebase is undone and tried again.
    fn start(&self, args: &[String], env: &[(&str, &str)]) -> Result<()> {
        let mut attempt = 0;
        loop {
            match self.git.exec(args, Exec { env, ..Default::default() }) {
                Err(GitError::Failed { stderr, .. }) if attempt < 3 && stderr.contains("git-rebase-todo") && stderr.contains("Permission denied") => {
                    attempt += 1;
                    if queries::operation(self.git)?.is_some() {
                        let _ = self.git.run(["rebase", "--abort"]);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100 * attempt));
                }
                result => return result.map(drop),
            }
        }
    }

    /// Stops a rebase at `sha`, runs `f` (which typically amends HEAD), then continues. If `f` fails the rebase is
    /// aborted, so the history is left as it was.
    fn at_commit(&self, sha: &str, f: impl FnOnce() -> Result<()>) -> Result<()> {
        let sha = self.full_sha(sha)?;
        self.rebase(self.ancestor(&sha, 1)?.as_deref(), TodoPlan { actions: vec![(sha.clone(), TodoAction::Edit)], ..Default::default() }, Options::default())?;
        if queries::operation(self.git)? != Some(Operation::Rebase) || queries::head_sha(self.git)?.as_deref() != Some(sha.as_str()) {
            return Err(GitError::user(format!("The rebase did not stop at {} as expected.", &sha[..10])));
        }
        if let Err(e) = f() {
            let _ = self.git.run(["rebase", "--abort"]);
            return Err(e);
        }
        self.git.run(["rebase", "--continue"])?;
        Ok(())
    }

    /// Amends only the metadata of `sha`, never the staged changes.
    fn amend_metadata(&self, sha: &str, extra: &[&str]) -> Result<()> {
        let mut args = vec!["commit", "--amend", "--only", "--no-edit", "--allow-empty"];
        args.extend(extra);
        if self.full_sha(sha)? == self.head()? && queries::operation(self.git)?.is_none() {
            self.git.run(&args)?;
            return Ok(());
        }
        self.at_commit(sha, || self.git.run(&args).map(drop))
    }

    fn actions(shas: &[String], action: TodoAction) -> TodoPlan {
        TodoPlan { actions: shas.iter().map(|s| (s.clone(), action)).collect(), ..Default::default() }
    }

    pub fn drop_commits(&self, shas: &[String]) -> Result<()> {
        let base = self.ancestor(&self.oldest(shas)?, 1)?;
        self.rebase(base.as_deref(), Self::actions(shas, TodoAction::Drop), Options::default())
    }

    /// Squashes (or fixes up) each commit into the commit before it.
    pub fn squash(&self, shas: &[String], fixup: bool) -> Result<()> {
        let oldest = self.oldest(shas)?;
        if self.ancestor(&oldest, 1)?.is_none() {
            return Err(GitError::user("The first commit has no previous commit to squash into."));
        }
        let parents: Vec<String> = shas.iter().map(|s| format!("{s}^")).collect();
        if self.merges_among(&parents)? {
            return Err(GitError::user("The commit before it is a merge commit, which can't be squashed into."));
        }
        let base = self.ancestor(&oldest, 2)?;
        let action = if fixup { TodoAction::Fixup } else { TodoAction::Squash };
        self.rebase(base.as_deref(), Self::actions(shas, action), Options::default())
    }

    pub fn reword(&self, sha: &str, message: &str) -> Result<()> {
        if message.trim().is_empty() {
            return Err(GitError::user("The message can't be empty."));
        }
        let sha = self.full_sha(sha)?;
        if sha == self.head()? && queries::operation(self.git)?.is_none() {
            let input = with_newline(message);
            self.git.exec(["commit", "--amend", "--only", "--allow-empty", "--cleanup=strip", "-F", "-"], Exec { input: Some(&input), ..Default::default() })?;
            return Ok(());
        }
        let plan = TodoPlan { actions: vec![(sha.clone(), TodoAction::Reword)], ..Default::default() };
        self.rebase(self.ancestor(&sha, 1)?.as_deref(), plan, Options { message: Some(message), ..Default::default() })
    }

    /// Stops at `sha` so it can be changed; continue the rebase when done.
    pub fn edit(&self, sha: &str) -> Result<()> {
        let sha = self.full_sha(sha)?;
        let plan = TodoPlan { actions: vec![(sha.clone(), TodoAction::Edit)], ..Default::default() };
        self.rebase(self.ancestor(&sha, 1)?.as_deref(), plan, Options::default())
    }

    pub fn move_commit(&self, sha: &str, direction: Direction) -> Result<()> {
        let sha = self.full_sha(sha)?;
        let plan = TodoPlan { move_commit: Some((sha.clone(), direction)), ..Default::default() };
        match direction {
            Direction::Up => {
                if sha == self.head()? {
                    return Err(GitError::user("This is already the newest commit."));
                }
                self.rebase(self.ancestor(&sha, 1)?.as_deref(), plan, Options::default())
            }
            Direction::Down => {
                if self.ancestor(&sha, 1)?.is_none() {
                    return Err(GitError::user("This is already the oldest commit."));
                }
                self.rebase(self.ancestor(&sha, 2)?.as_deref(), plan, Options::default())
            }
        }
    }

    /// Folds the staged changes into `sha`.
    pub fn amend_staged(&self, sha: &str) -> Result<()> {
        if !has_staged_changes(self.git)? {
            return Err(GitError::user("There are no staged changes to amend."));
        }
        let sha = self.full_sha(sha)?;
        if sha == self.head()? {
            self.git.run(["commit", "--amend", "--no-edit"])?;
            return Ok(());
        }
        self.ensure_no_operation()?;
        if self.merges_among(std::slice::from_ref(&sha))? {
            return Err(GitError::user("A merge commit can't be amended with staged changes."));
        }
        let base = self.ancestor(&sha, 1)?;
        // A temporary commit, folded into the target with its message discarded: no hooks.
        self.git.run(["commit", "--no-verify", "-m", &format!("oxbow: amend {sha}")])?;
        let temp = self.head()?;
        let plan = TodoPlan { fixup_into: Some((temp, sha)), ..Default::default() };
        let result = self.rebase(base.as_deref(), plan, Options::default());
        // If the rebase never started, put the changes back in the index.
        if result.is_err() && queries::operation(self.git)?.is_none() {
            self.git.run(["reset", "--soft", "HEAD~1"])?;
        }
        result
    }

    /// A "fixup! <subject>" commit for `sha` from the staged changes.
    pub fn create_fixup(&self, sha: &str) -> Result<()> {
        if !has_staged_changes(self.git)? {
            return Err(GitError::user("Stage the changes for the fixup commit first."));
        }
        self.git.run(["commit", &format!("--fixup={sha}")])?;
        Ok(())
    }

    /// Squashes every "fixup!" and "squash!" commit after `sha` into its target.
    pub fn autosquash(&self, sha: &str) -> Result<()> {
        let base = self.ancestor(sha, 1)?;
        self.rebase(base.as_deref(), TodoPlan::default(), Options { autosquash: true, ..Default::default() })
    }

    pub fn reset_author(&self, sha: &str) -> Result<()> {
        self.amend_metadata(sha, &["--reset-author"])
    }

    /// `author` as "Name <email>".
    pub fn set_author(&self, sha: &str, author: &str) -> Result<()> {
        self.amend_metadata(sha, &[&format!("--author={author}")])
    }

    /// Rebases the current branch onto `target`, stopped before the first step so its steps can be changed first.
    pub fn rebase_interactive(&self, target: &str) -> Result<()> {
        let count = self.git.run(["rev-list", "--count", &format!("{target}..HEAD")])?;
        if count.trim() == "0" {
            return Err(GitError::user("There is nothing to rebase: the current branch has no commits that aren't on the target."));
        }
        self.rebase(Some(target), TodoPlan { pause_first: true, ..Default::default() }, Options::default())
    }

    // --- a paused rebase ----------------------------------------------------------------

    /// Changes the remaining steps of the paused rebase.
    pub fn edit_todo(&self, plan: &TodoPlan) -> Result<()> {
        let file = self.git.git_path("rebase-merge/git-rebase-todo")?;
        let text = std::fs::read_to_string(&file).map_err(|_| GitError::user("No interactive rebase is paused."))?;
        let text = todo::transform_todo(&text, plan, self.git.git_dir()?).map_err(|e| GitError::user(format!("Could not change the rebase steps: {e}.")))?;
        std::fs::write(&file, text).map_err(|e| GitError::user(format!("Could not write the rebase steps: {e}")))
    }

    /// Gives a step of the paused rebase a new message, applied when git gets to it.
    pub fn reword_todo(&self, sha: &str, message: &str) -> Result<()> {
        let full = self.full_sha(sha)?;
        let git_dir = self.git.git_dir()?;
        std::fs::write(todo::message_file(git_dir, &full), with_newline(message)).map_err(|e| GitError::user(e.to_string()))?;
        self.edit_todo(&TodoPlan { reword: vec![full], ..Default::default() })
    }
}

fn has_staged_changes(git: &Git) -> Result<bool> {
    // --quiet exits 1 when there are differences. Before the first commit, compare with the empty tree.
    let against = if queries::head_sha(git)?.is_some() { "HEAD" } else { queries::EMPTY_TREE };
    Ok(git.exec(["diff", "--cached", "--quiet", against, "--"], Exec { allowed: &[1], ..Default::default() })?.code == 1)
}

/// The remaining steps of a paused interactive rebase, next one first; empty when none is paused.
pub fn todo_steps(git: &Git) -> Result<Vec<TodoEntry>> {
    match std::fs::read_to_string(git.git_path("rebase-merge/git-rebase-todo")?) {
        Ok(text) => Ok(todo::parse_todo(&text)),
        Err(_) => Ok(Vec::new()),
    }
}

/// Where changes taken out of a commit go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchTarget {
    /// Nowhere: they are gone (Undo brings them back).
    Discard,
    /// Into the working tree and index, as uncommitted changes.
    WorkingTree,
    /// Into a new commit right after the one they came from, with this message.
    NewCommit(String),
}

impl Rewrite<'_> {
    fn scratch(&self, name: &str, content: &str) -> Result<std::path::PathBuf> {
        let file = self.git.git_path(name)?;
        std::fs::write(&file, content).map_err(|e| GitError::user(format!("Could not write {}: {e}", file.display())))?;
        Ok(file)
    }

    /// The changes `removal` (from `patch::removal_patch`, for any number of files) takes out of `sha`, as a
    /// forward patch with blob ids, so it can be applied elsewhere with a 3-way merge.
    fn forward_patch(&self, sha: &str, removal: &str) -> Result<String> {
        let index = self.git.git_path("oxbow-patch-index")?;
        let removal_file = self.scratch("oxbow-patch.patch", removal)?;
        let index_text = index.to_string_lossy().into_owned();
        // A scratch index, so the real one and the working tree are untouched.
        let env = [("GIT_INDEX_FILE", index_text.as_str())];
        let run = |args: &[&str]| self.git.exec(args, Exec { env: &env, ..Default::default() }).map(|o| o.stdout);
        let result = (|| {
            run(&["read-tree", sha])?;
            run(&["apply", "--cached", "--recount", &removal_file.to_string_lossy()])?;
            let without = run(&["write-tree"])?;
            self.git.run(["diff", "--binary", without.trim(), sha])
        })();
        let _ = std::fs::remove_file(&index);
        let _ = std::fs::remove_file(&removal_file);
        result
    }

    /// Takes the changes in `removal` out of commit `sha`, and sends them to `target`.
    pub fn remove_patch(&self, sha: &str, removal: &str, target: &PatchTarget) -> Result<()> {
        let sha = self.full_sha(sha)?;
        let patch_file = self.scratch("oxbow-patch.patch", removal)?;
        let restore_file = self.git.git_path("oxbow-uncommit.patch")?;
        let patch_path = patch_file.to_string_lossy().into_owned();
        let result = self.at_commit(&sha, || {
            self.git.run(["apply", "--index", "--recount", &patch_path])?;
            self.git.run(["commit", "--amend", "--no-edit", "--allow-empty", "--no-verify"])?;
            if *target == PatchTarget::Discard {
                return Ok(());
            }
            // Exactly what was taken out, with blob ids, for a 3-way merge later.
            let taken = self.git.run(["diff", "--binary", "HEAD", &sha])?;
            std::fs::write(&restore_file, taken).map_err(|e| GitError::user(e.to_string()))?;
            if let PatchTarget::NewCommit(message) = target {
                self.git.run(["apply", "--index", &restore_file.to_string_lossy()])?;
                self.git.exec(["commit", "--no-verify", "-F", "-"], Exec { input: Some(message), ..Default::default() })?;
            }
            Ok(())
        });
        let _ = std::fs::remove_file(&patch_file);
        result?;
        if *target == PatchTarget::WorkingTree {
            let applied = self.git.run(["apply", "--3way", &restore_file.to_string_lossy()]);
            if let Err(e) = applied {
                return Err(GitError::user(format!(
                    "The changes were taken out of the commit but could not be applied to your working tree ({}). They are saved in {}.",
                    e.to_string().lines().next().unwrap_or_default(),
                    restore_file.display()
                )));
            }
        }
        let _ = std::fs::remove_file(&restore_file);
        Ok(())
    }

    /// Moves the changes `removal` takes out of `sha` into commit `target`, older or newer. If they don't apply there,
    /// the history is put back as it was.
    pub fn move_patch(&self, sha: &str, removal: &str, target: &str) -> Result<()> {
        let (sha, target) = (self.full_sha(sha)?, self.full_sha(target)?);
        if sha == target {
            return Err(GitError::user("Pick a different commit to move the changes into."));
        }
        // Found again by its distance from HEAD after the first rewrite, which changes it if it is newer than `sha`.
        let distance: usize = self.git.run(["rev-list", "--first-parent", "--count", &format!("{target}..HEAD")])?.trim().parse().unwrap_or(0);
        if self.ancestor("HEAD", distance)?.as_deref() != Some(target.as_str()) && !(distance == 0 && self.head()? == target) {
            return Err(GitError::user("The commit to move the changes into must be on the current branch, not on a merged-in branch."));
        }
        if self.merges_among(std::slice::from_ref(&target))? {
            return Err(GitError::user("Changes can't be moved into a merge commit."));
        }
        let forward = self.forward_patch(&sha, removal)?;
        let original = self.head()?;
        self.remove_patch(&sha, removal, &PatchTarget::Discard)?;
        let forward_file = self.scratch("oxbow-move.patch", &forward)?;
        let new_target = if distance == 0 { self.head()? } else { self.ancestor("HEAD", distance)?.unwrap_or_default() };
        let moved = self.at_commit(&new_target, || {
            self.git.run(["apply", "--index", "--3way", &forward_file.to_string_lossy()])?;
            self.git.run(["commit", "--amend", "--no-edit", "--allow-empty", "--no-verify"])?;
            Ok(())
        });
        let _ = std::fs::remove_file(&forward_file);
        if let Err(e) = moved {
            // Back to before the first step; --keep leaves uncommitted changes alone.
            let back = self.git.run(["reset", "--keep", &original]);
            let reason = e.to_string().lines().next().unwrap_or_default().to_owned();
            return Err(GitError::user(match back {
                Ok(_) => format!("The changes don't apply to {}, so nothing was changed ({reason}).", &target[..7]),
                Err(_) => format!("The changes don't apply to {} ({reason}). They were taken out of {}; the original history is {}.", &target[..7], &sha[..7], &original[..10]),
            }));
        }
        Ok(())
    }

    /// Applies the changes `removal` takes out of `sha` to the working tree, or with `reverse` takes them out of it.
    pub fn apply_patch(&self, sha: &str, removal: &str, reverse: bool) -> Result<()> {
        let forward = self.forward_patch(sha, removal)?;
        let file = self.scratch("oxbow-apply.patch", &forward)?;
        let path = file.to_string_lossy().into_owned();
        let mut args = vec!["apply"];
        if reverse {
            args.push("--reverse");
        }
        args.push(&path);
        // Unstaged when it applies as it is; otherwise a 3-way merge, which stages what it merges and marks conflicts.
        let plain = self.git.exec(&args, Exec { allowed: &[1], ..Default::default() });
        let result = match plain {
            Ok(out) if out.code == 0 => Ok(()),
            Ok(_) => {
                args.insert(1, "--3way");
                self.git.run(&args).map(drop)
            }
            Err(e) => Err(e),
        };
        let _ = std::fs::remove_file(&file);
        result
    }
}

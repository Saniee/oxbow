//! Reading the repository: commits, branches, tags, stashes, the reflog, worktrees and in-progress operations.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::git::{Exec, Git, Result};

const US: char = '\x1f';
const RS: char = '\x1e';
/// Separates co-authors within their field.
const GS: char = '\x1d';

/// Hash of git's empty tree; the "parent" of a root commit.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

fn records(output: &str) -> Vec<Vec<&str>> {
    output
        .split(RS)
        .map(|r| r.trim_start_matches(['\r', '\n']))
        .filter(|r| !r.is_empty())
        .map(|r| r.split(US).collect())
        .collect()
}

fn field<'a>(f: &[&'a str], i: usize) -> &'a str {
    f.get(i).copied().unwrap_or("")
}

fn split_list(value: &str, separator: char) -> Vec<String> {
    value.split(separator).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub email: String,
}

/// "Name <email>" (or just a name), as in a Co-authored-by trailer.
pub fn parse_person(value: &str) -> Person {
    let value = value.trim();
    if let (Some(open), true) = (value.rfind('<'), value.ends_with('>')) {
        let email = value[open + 1..value.len() - 1].to_owned();
        let name = value[..open].trim();
        return Person { name: if name.is_empty() { email.clone() } else { name.to_owned() }, email };
    }
    Person { name: value.to_owned(), email: String::new() }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub author_email: String,
    pub co_authors: Vec<Person>,
    /// The author date as git words it ("3 days ago"), as of when it was read; see [`relative_date`].
    pub rel_date: String,
    /// Author date, ISO 8601.
    pub date: String,
    /// Author date, Unix seconds.
    pub author_time: i64,
    /// Committer date, Unix seconds.
    pub timestamp: i64,
    pub parents: Vec<String>,
    /// Ref decorations, e.g. "HEAD -> main", "origin/main", "tag: v1".
    pub refs: Vec<String>,
    pub subject: String,
}

impl Commit {
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

const CO_AUTHORS: &str = "%(trailers:key=Co-authored-by,valueonly,unfold,separator=%x1d)";

/// With `decorate`, each commit's refs (%D); git looks them up for every commit, which costs time with many refs.
fn log_format(decorate: bool) -> String {
    let fields = ["%H", "%h", "%an", "%ae", "%ar", "%aI", "%ct", "%at", "%P", if decorate { "%D" } else { "" }, CO_AUTHORS, "%s"];
    format!("--format={}{RS}", fields.join(&US.to_string()))
}

pub fn parse_log(output: &str) -> Vec<Commit> {
    records(output)
        .into_iter()
        .filter(|f| f.len() >= 12)
        .map(|f| Commit {
            sha: f[0].to_owned(),
            short_sha: f[1].to_owned(),
            author: f[2].to_owned(),
            author_email: f[3].to_owned(),
            rel_date: f[4].to_owned(),
            date: f[5].to_owned(),
            timestamp: f[6].parse().unwrap_or(0),
            author_time: f[7].parse().unwrap_or(0),
            parents: split_list(f[8], ' '),
            refs: f[9].split(", ").filter(|s| !s.is_empty()).map(str::to_owned).collect(),
            co_authors: split_list(f[10], GS).iter().map(|p| parse_person(p)).collect(),
            subject: f[11].to_owned(),
        })
        .collect()
}

fn run_log(git: &Git, args: &[String], decorate: bool) -> Result<Vec<Commit>> {
    // Paths after "--" are file names as they are, not patterns: a file named `[ab].txt` must not match a.txt.
    let mut all = vec!["--literal-pathspecs".to_owned(), "log".to_owned(), log_format(decorate)];
    all.extend_from_slice(args);
    // 128: unborn branch (no commits yet), or a revision that doesn't exist.
    let out = git.exec(&all, Exec { allowed: &[128], ..Default::default() })?;
    Ok(if out.code == 0 { parse_log(&out.stdout) } else { Vec::new() })
}

#[derive(Clone, Debug, Default)]
pub struct LogQuery {
    pub limit: usize,
    /// Defaults to HEAD.
    pub rev: Option<String>,
    /// Every branch, remote branch and tag too.
    pub all: bool,
    /// Case-insensitive text matched against the message and author, or a SHA prefix.
    pub search: Option<String>,
    pub paths: Vec<String>,
    /// Leave `refs` empty: the caller fills them in from the refs it already has (see `Snapshot`).
    pub undecorated: bool,
}

/// Commits newest first, every commit after all of its children (`--date-order`), whatever the commit dates say: the
/// graph depends on it. git's default order doesn't promise that: with a clock that was off, a parent can come
/// before a child on another branch, or the child can fall past the limit while its parent is listed. With a
/// commit-graph file (git gc writes one) git streams this order as quickly as the default one.
pub fn log(git: &Git, query: &LogQuery) -> Result<Vec<Commit>> {
    let rev = query.rev.clone().unwrap_or_else(|| "HEAD".to_owned());
    let decorate = !query.undecorated;
    let mut base = vec![format!("-n{}", query.limit.max(1)), "--date-order".to_owned()];
    if query.all {
        base.extend(["--branches", "--remotes", "--tags"].map(str::to_owned));
    }
    base.push(rev);
    let with_paths = |mut args: Vec<String>| {
        args.push("--".to_owned());
        args.extend(query.paths.iter().cloned());
        args
    };
    let search = query.search.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let Some(text) = search else { return run_log(git, &with_paths(base), decorate) };

    let flags = ["--regexp-ignore-case".to_owned(), "--fixed-strings".to_owned()];
    let mut by_message = base.clone();
    by_message.extend(flags.clone());
    by_message.push(format!("--grep={text}"));
    let mut by_author = base.clone();
    by_author.extend(flags);
    by_author.push(format!("--author={text}"));
    let mut found = Vec::new();
    if text.len() >= 4 && text.chars().all(|c| c.is_ascii_hexdigit()) {
        found.extend(run_log(git, &["-n1".to_owned(), format!("{text}^{{commit}}"), "--".to_owned()], decorate)?);
    }
    found.extend(run_log(git, &with_paths(by_message), decorate)?);
    found.extend(run_log(git, &with_paths(by_author), decorate)?);
    let mut seen = HashSet::new();
    found.retain(|c| seen.insert(c.sha.clone()));
    found.sort_by_key(|c| std::cmp::Reverse(c.timestamp));
    found.truncate(query.limit);
    Ok(found)
}

/// A past time as git words it in %ar and --date=relative ("5 minutes ago", "2 years, 3 months ago"), the same
/// rounding included, so a list read a while ago can be brought up to date without asking git again.
pub fn relative_date(time: i64, now: i64) -> String {
    let plural = |n: i64, one: &str| if n == 1 { format!("{n} {one}") } else { format!("{n} {one}s") };
    if now < time {
        return "in the future".to_owned();
    }
    let mut diff = now - time;
    if diff < 90 {
        return format!("{} ago", plural(diff, "second"));
    }
    diff = (diff + 30) / 60;
    if diff < 90 {
        return format!("{} ago", plural(diff, "minute"));
    }
    diff = (diff + 30) / 60;
    if diff < 36 {
        return format!("{} ago", plural(diff, "hour"));
    }
    diff = (diff + 12) / 24;
    if diff < 14 {
        return format!("{} ago", plural(diff, "day"));
    }
    if diff < 70 {
        return format!("{} ago", plural((diff + 3) / 7, "week"));
    }
    if diff < 365 {
        return format!("{} ago", plural((diff + 15) / 30, "month"));
    }
    if diff < 1825 {
        let total_months = (diff * 12 * 2 + 365) / (365 * 2);
        let (years, months) = (total_months / 12, total_months % 12);
        return if months > 0 { format!("{}, {} ago", plural(years, "year"), plural(months, "month")) } else { format!("{} ago", plural(years, "year")) };
    }
    format!("{} ago", plural((diff + 183) / 365, "year"))
}

/// The full message of a commit, without trailing whitespace.
pub fn commit_message(git: &Git, rev: &str) -> Result<String> {
    Ok(git.run(["log", "-1", "--format=%B", rev, "--"])?.trim_end().to_owned())
}

/// The refs that count as the main branch: for each name, the local branch's upstream, or the local branch when it
/// has none, or origin/<name> when there is no local branch.
pub fn main_refs(git: &Git, names: &[String]) -> Result<Vec<String>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec!["for-each-ref".to_owned(), "--format=%(refname)%09%(upstream)".to_owned()];
    for n in names {
        args.push(format!("refs/heads/{n}"));
        args.push(format!("refs/remotes/origin/{n}"));
    }
    let out = git.run(&args)?;
    let found: Vec<(String, String)> = out
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .collect();
    let lookup = |r: &str| found.iter().find(|(name, _)| name == r).map(|(_, up)| up.clone());
    Ok(names
        .iter()
        .filter_map(|n| {
            let local = format!("refs/heads/{n}");
            match lookup(&local) {
                Some(up) => Some(if up.is_empty() { local } else { up }),
                None => {
                    let remote = format!("refs/remotes/origin/{n}");
                    lookup(&remote).map(|_| remote)
                }
            }
        })
        .collect())
}

/// Of `shas`, the ones reachable from any of `refs`.
pub fn reachable_from(git: &Git, shas: &[String], refs: &[String]) -> Result<HashSet<String>> {
    if shas.is_empty() || refs.is_empty() {
        return Ok(HashSet::new());
    }
    let mut input: Vec<String> = shas.to_vec();
    input.extend(refs.iter().map(|r| format!("^{r}")));
    let input = input.join("\n") + "\n";
    let out = git.exec(["rev-list", "--stdin"], Exec { input: Some(&input), allowed: &[128], ..Default::default() })?;
    if out.code != 0 {
        return Ok(HashSet::new());
    }
    let outside: HashSet<&str> = out.stdout.lines().map(str::trim).collect();
    Ok(shas.iter().filter(|s| !outside.contains(s.as_str())).cloned().collect())
}

/// Commits on HEAD that are not on its upstream, or None without an upstream.
pub fn unpushed(git: &Git) -> Result<Option<HashSet<String>>> {
    let out = git.exec(["rev-list", "@{upstream}..HEAD"], Exec { allowed: &[128], ..Default::default() })?;
    Ok((out.code == 0).then(|| out.stdout.lines().map(|l| l.trim().to_owned()).collect()))
}

#[derive(Clone, Debug)]
pub struct ReflogEntry {
    pub sha: String,
    pub short_sha: String,
    /// e.g. HEAD@{3}
    pub selector: String,
    pub rel_date: String,
    pub subject: String,
}

pub fn reflog(git: &Git, limit: usize) -> Result<Vec<ReflogEntry>> {
    let format = format!("--format=%H{US}%h{US}%gd{US}%cr{US}%gs{RS}");
    let n = format!("-n{limit}");
    let out = git.exec(["reflog", "show", &n, &format, "HEAD", "--"], Exec { allowed: &[128], ..Default::default() })?;
    if out.code != 0 {
        return Ok(Vec::new());
    }
    Ok(records(&out.stdout)
        .into_iter()
        .map(|f| ReflogEntry {
            sha: field(&f, 0).to_owned(),
            short_sha: field(&f, 1).to_owned(),
            selector: field(&f, 2).to_owned(),
            rel_date: field(&f, 3).to_owned(),
            subject: field(&f, 4).to_owned(),
        })
        .collect())
}

#[derive(Clone, Debug)]
pub struct Stash {
    /// e.g. stash@{0}
    pub selector: String,
    pub sha: String,
    pub rel_date: String,
    pub message: String,
}

pub fn stashes(git: &Git) -> Result<Vec<Stash>> {
    let format = format!("--format=%gd{US}%H{US}%cr{US}%gs{RS}");
    // Exit 1 ("bad revision 'refs/stash'") while the last stash is being dropped: there are none.
    let out = git.exec(["stash", "list", &format], Exec { allowed: &[1], ..Default::default() })?;
    if out.code != 0 {
        return Ok(Vec::new());
    }
    Ok(records(&out.stdout)
        .into_iter()
        .map(|f| Stash {
            selector: field(&f, 0).to_owned(),
            sha: field(&f, 1).to_owned(),
            rel_date: field(&f, 2).to_owned(),
            message: field(&f, 3).to_owned(),
        })
        .collect())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Rebase,
    CherryPick,
    Revert,
    Merge,
}

impl Operation {
    pub fn name(self) -> &'static str {
        match self {
            Operation::Rebase => "rebase",
            Operation::CherryPick => "cherry-pick",
            Operation::Revert => "revert",
            Operation::Merge => "merge",
        }
    }
}

/// The rebase, merge, cherry-pick or revert that is stopped, if any.
pub fn operation(git: &Git) -> Result<Option<Operation>> {
    let markers = [
        ("rebase-merge", Operation::Rebase),
        ("rebase-apply", Operation::Rebase),
        ("CHERRY_PICK_HEAD", Operation::CherryPick),
        ("REVERT_HEAD", Operation::Revert),
        ("MERGE_HEAD", Operation::Merge),
    ];
    for (marker, op) in markers {
        if git.git_path(marker)?.exists() {
            return Ok(Some(op));
        }
    }
    // A multi-commit cherry-pick or revert whose stopped commit was committed by hand is still queued here.
    if let Ok(todo) = std::fs::read_to_string(git.git_path("sequencer/todo")?) {
        let first = todo.split_whitespace().next().unwrap_or("");
        return Ok(Some(if first == "revert" || first == "r" { Operation::Revert } else { Operation::CherryPick }));
    }
    Ok(None)
}

/// The checked-out branch, or None when HEAD is detached (including during a rebase).
pub fn current_branch(git: &Git) -> Result<Option<String>> {
    let out = git.exec(["symbolic-ref", "--short", "-q", "HEAD"], Exec { allowed: &[1, 128], ..Default::default() })?;
    let name = out.stdout.trim();
    Ok((out.code == 0 && !name.is_empty()).then(|| name.to_owned()))
}

/// HEAD's SHA read straight from the git directory (HEAD, then the loose ref or packed-refs), without starting a
/// git process. None when that isn't possible (no commits yet, a reftable repository, anything unusual): ask git then.
pub fn head_from_files(git: &Git) -> Option<String> {
    let git_dir = git.git_dir().ok()?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    let is_sha = |s: &str| (s.len() == 40 || s.len() == 64) && s.chars().all(|c| c.is_ascii_hexdigit());
    let Some(reference) = head.strip_prefix("ref: ") else { return is_sha(head).then(|| head.to_owned()) };
    let common = common_dir(git)?;
    if let Ok(loose) = std::fs::read_to_string(common.join(reference)) {
        let sha = loose.trim();
        return is_sha(sha).then(|| sha.to_owned());
    }
    let packed = std::fs::read_to_string(common.join("packed-refs")).ok()?;
    packed.lines().find_map(|l| l.split_once(' ').filter(|(sha, name)| *name == reference && is_sha(sha)).map(|(sha, _)| sha.to_owned()))
}

/// The directory with the refs and objects every worktree of the repository shares: the git directory itself,
/// except in a linked worktree.
fn common_dir(git: &Git) -> Option<PathBuf> {
    let git_dir = git.git_dir().ok()?;
    Some(match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(rel) => git_dir.join(rel.trim()),
        Err(_) => git_dir.to_path_buf(),
    })
}

/// Whether the history git shows can change while every ref stays where it is: a shallow clone (fetching deeper
/// adds history), grafts, or replace refs. A commit list must not be reused then.
pub fn history_can_change_in_place(git: &Git) -> bool {
    let Some(common) = common_dir(git) else { return true };
    if common.join("shallow").exists() || common.join("info").join("grafts").exists() {
        return true;
    }
    let replace_dir = common.join("refs").join("replace");
    if std::fs::read_dir(&replace_dir).is_ok_and(|mut d| d.next().is_some()) {
        return true;
    }
    // Packed replace refs. The file is read only up to its first line naming one.
    std::fs::read_to_string(common.join("packed-refs")).is_ok_and(|p| p.contains(" refs/replace/"))
}

/// HEAD's SHA, or None before the first commit.
pub fn head_sha(git: &Git) -> Result<Option<String>> {
    let out = git.exec(["rev-parse", "-q", "--verify", "HEAD"], Exec { allowed: &[1, 128], ..Default::default() })?;
    Ok((out.code == 0).then(|| out.stdout.trim().to_owned()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upstream {
    /// e.g. "origin/feature/x"
    pub name: String,
    pub remote: String,
    /// The branch's ref on the remote, e.g. "refs/heads/feature/x".
    pub remote_ref: String,
}

#[derive(Clone, Debug)]
pub struct LocalBranch {
    pub name: String,
    pub sha: String,
    pub current: bool,
    pub upstream: Option<Upstream>,
    pub ahead: u32,
    pub behind: u32,
    /// The upstream was deleted on the remote.
    pub gone: bool,
    /// Checked out in another worktree.
    pub worktree: Option<PathBuf>,
    pub rel_date: String,
    pub subject: String,
}

/// `%(upstream:track,nobracket)`: "ahead 2, behind 1", "gone" or "".
pub fn parse_track(track: &str) -> (u32, u32, bool) {
    let count = |word: &str| {
        track
            .split(", ")
            .find_map(|part| part.strip_prefix(word))
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0)
    };
    (count("ahead "), count("behind "), track.contains("gone"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BranchSort {
    /// Most recently checked out first.
    #[default]
    Recency,
    Date,
    Name,
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

/// Local branches, the current one first.
pub fn local_branches(git: &Git, sort: BranchSort) -> Result<Vec<LocalBranch>> {
    let mut branches = read_local_branches(git, sort)?;
    if sort == BranchSort::Recency {
        let out = git.exec(["reflog", "show", "-n1000", "--format=%gs", "HEAD", "--"], Exec { allowed: &[128], ..Default::default() })?;
        sort_by_checkouts(&mut branches, out.stdout.lines());
    }
    Ok(branches)
}

/// Puts the most recently checked-out branches first, from the HEAD reflog's subjects (newest first); the current
/// branch stays on top.
pub fn sort_by_checkouts<'a>(branches: &mut [LocalBranch], subjects: impl Iterator<Item = &'a str>) {
    let recent = recent_checkouts(subjects);
    let rank = |name: &str| recent.iter().position(|r| r == name).unwrap_or(usize::MAX);
    branches.sort_by_cached_key(|b| (!b.current, rank(&b.name)));
}

/// Local branches by date or name, the current one first, without the recency order (see [`sort_by_checkouts`]).
pub fn read_local_branches(git: &Git, sort: BranchSort) -> Result<Vec<LocalBranch>> {
    let fields = [
        "%(refname:short)",
        "%(objectname)",
        "%(HEAD)",
        "%(upstream:short)",
        "%(upstream:remotename)",
        "%(upstream:remoteref)",
        "%(upstream:track,nobracket)",
        "%(committerdate:relative)",
        "%(contents:subject)",
        "%(worktreepath)",
    ];
    let order = if sort == BranchSort::Name { "--sort=refname" } else { "--sort=-committerdate" };
    let format = format!("--format={}{RS}", fields.join(&US.to_string()));
    let out = git.run(["for-each-ref", order, &format, "refs/heads"])?;
    let mut branches: Vec<LocalBranch> = records(&out)
        .into_iter()
        .map(|f| {
            let (ahead, behind, gone) = parse_track(field(&f, 6));
            let current = field(&f, 2) == "*";
            let wt = field(&f, 9);
            LocalBranch {
                name: field(&f, 0).to_owned(),
                sha: field(&f, 1).to_owned(),
                current,
                upstream: (!field(&f, 3).is_empty() && !field(&f, 4).is_empty()).then(|| Upstream {
                    name: field(&f, 3).to_owned(),
                    remote: field(&f, 4).to_owned(),
                    remote_ref: field(&f, 5).to_owned(),
                }),
                ahead,
                behind,
                gone,
                worktree: (!wt.is_empty() && !current && !same_path(Path::new(wt), git.root())).then(|| PathBuf::from(wt)),
                rel_date: field(&f, 7).to_owned(),
                subject: field(&f, 8).to_owned(),
            }
        })
        .collect();
    branches.sort_by_key(|b| !b.current);
    Ok(branches)
}

/// Branch names in the order they were last checked out, most recent first.
fn recent_checkouts<'a>(subjects: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in subjects {
        let Some(rest) = line.strip_prefix("checkout: moving from ") else { continue };
        let Some((from, to)) = rest.split_once(" to ") else { continue };
        for name in [to, from] {
            if !names.iter().any(|n| n == name) {
                names.push(name.to_owned());
            }
        }
    }
    names
}

#[derive(Clone, Debug)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

pub fn remotes(git: &Git) -> Result<Vec<Remote>> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in git.run(["remote", "-v"])?.lines() {
        let Some(rest) = line.strip_suffix(" (fetch)") else { continue };
        let Some((name, url)) = rest.split_once('\t') else { continue };
        if !remotes.iter().any(|r| r.name == name) {
            remotes.push(Remote { name: name.to_owned(), url: url.to_owned() });
        }
    }
    Ok(remotes)
}

#[derive(Clone, Debug)]
pub struct RemoteBranch {
    /// e.g. "origin/main"
    pub name: String,
    pub remote: String,
    /// The branch name on the remote, e.g. "main".
    pub branch: String,
    pub sha: String,
    pub rel_date: String,
    pub subject: String,
}

pub fn remote_branches(git: &Git, remotes: &[Remote]) -> Result<Vec<RemoteBranch>> {
    Ok(parse_remote_branches(&read_remote_branches(git)?, remotes))
}

/// `for-each-ref refs/remotes`, for [`parse_remote_branches`]; it doesn't need the remotes, so it can run beside them.
pub fn read_remote_branches(git: &Git) -> Result<String> {
    let fields = ["%(refname)", "%(refname:short)", "%(objectname)", "%(symref)", "%(committerdate:relative)", "%(contents:subject)"];
    let format = format!("--format={}{RS}", fields.join(&US.to_string()));
    git.run(["for-each-ref", "--sort=refname", &format, "refs/remotes"])
}

/// The symbolic refs among the remote refs (origin/HEAD), short name and the commit they point at: not branches,
/// but git lists them among a commit's refs.
pub fn parse_remote_heads(out: &str) -> Vec<(String, String)> {
    // Named in full: the short name of refs/remotes/origin/HEAD is "origin".
    records(out)
        .into_iter()
        .filter(|f| !field(f, 3).is_empty())
        .filter_map(|f| Some((field(&f, 0).strip_prefix("refs/remotes/")?.to_owned(), field(&f, 2).to_owned())))
        .collect()
}

pub fn parse_remote_branches(out: &str, remotes: &[Remote]) -> Vec<RemoteBranch> {
    // Longest remote name first, in case one is a prefix of another.
    let mut names: Vec<&str> = remotes.iter().map(|r| r.name.as_str()).collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    records(out)
        .into_iter()
        .filter(|f| field(f, 3).is_empty()) // origin/HEAD
        .filter_map(|f| {
            let full = field(&f, 0);
            let remote = names.iter().find(|n| full.starts_with(&format!("refs/remotes/{n}/")))?;
            Some(RemoteBranch {
                name: field(&f, 1).to_owned(),
                remote: (*remote).to_owned(),
                branch: full[format!("refs/remotes/{remote}/").len()..].to_owned(),
                sha: field(&f, 2).to_owned(),
                rel_date: field(&f, 4).to_owned(),
                subject: field(&f, 5).to_owned(),
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct Tag {
    pub name: String,
    /// The commit it points at (peeled for annotated tags).
    pub sha: String,
    pub annotated: bool,
    pub rel_date: String,
    pub subject: String,
}

pub fn tags(git: &Git) -> Result<Vec<Tag>> {
    let fields = ["%(refname:short)", "%(objectname)", "%(*objectname)", "%(creatordate:relative)", "%(contents:subject)", "%(*objecttype)", "%(refname)"];
    let format = format!("--format={}{RS}", fields.join(&US.to_string()));
    let out = git.run(["for-each-ref", "--sort=-creatordate", &format, "refs/tags"])?;
    let records = records(&out);
    let mut tags: Vec<Tag> = records
        .iter()
        .map(|f| Tag {
            name: field(f, 0).to_owned(),
            sha: if field(f, 2).is_empty() { field(f, 1) } else { field(f, 2) }.to_owned(),
            annotated: !field(f, 2).is_empty(),
            rel_date: field(f, 3).to_owned(),
            subject: field(f, 4).to_owned(),
        })
        .collect();
    // %(*objectname) peels once; a tag of a tag needs peeling further (rare, so asked for only when there is one).
    let nested: Vec<usize> = (0..records.len()).filter(|&i| field(&records[i], 5) == "tag").collect();
    if !nested.is_empty() {
        let mut args = vec!["rev-parse".to_owned()];
        args.extend(nested.iter().map(|&i| format!("{}^{{}}", field(&records[i], 6))));
        let peeled = git.run(&args)?;
        for (&i, sha) in nested.iter().zip(peeled.lines()) {
            tags[i].sha = sha.trim().to_owned();
        }
    }
    Ok(tags)
}

#[derive(Clone, Debug)]
pub struct Worktree {
    pub path: PathBuf,
    pub sha: Option<String>,
    /// Short branch name, unless detached.
    pub branch: Option<String>,
    pub main: bool,
    pub current: bool,
    pub locked: bool,
    pub prunable: bool,
}

pub fn worktrees(git: &Git) -> Result<Vec<Worktree>> {
    let out = git.run(["worktree", "list", "--porcelain"])?;
    let out = out.replace("\r\n", "\n");
    Ok(out
        .split("\n\n")
        .filter(|b| !b.trim().is_empty())
        .enumerate()
        .filter_map(|(index, block)| {
            let lines: Vec<&str> = block.lines().collect();
            if lines.contains(&"bare") {
                return None;
            }
            let value = |key: &str| lines.iter().find_map(|l| l.strip_prefix(&format!("{key} ")[..]));
            let path = PathBuf::from(value("worktree").unwrap_or(""));
            let flag = |key: &str| lines.iter().any(|l| *l == key || l.starts_with(&format!("{key} ")));
            Some(Worktree {
                current: same_path(&path, git.root()),
                path,
                sha: value("HEAD").map(str::to_owned),
                branch: value("branch").map(|b| b.trim_start_matches("refs/heads/").to_owned()),
                main: index == 0,
                locked: flag("locked"),
                prunable: flag("prunable"),
            })
        })
        .collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// A, M, D, R, C, T or U (untracked, for stashes).
    pub status: char,
    pub path: String,
    pub old_path: Option<String>,
}

/// `git diff --name-status -z` output.
pub fn parse_name_status(output: &str) -> Vec<FileChange> {
    let tokens: Vec<&str> = output.split('\0').collect();
    let mut changes = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let status = tokens[i];
        let Some(letter) = status.chars().next() else { break };
        if letter == 'R' || letter == 'C' {
            changes.push(FileChange { status: letter, old_path: tokens.get(i + 1).map(|s| s.to_string()), path: tokens.get(i + 2).unwrap_or(&"").to_string() });
            i += 3;
        } else {
            changes.push(FileChange { status: letter, old_path: None, path: tokens.get(i + 1).unwrap_or(&"").to_string() });
            i += 2;
        }
    }
    changes
}

/// The patch of a commit against its first parent (or the empty tree for a root commit).
pub fn commit_patch(git: &Git, sha: &str, parent: Option<&str>) -> Result<String> {
    let base = parent.unwrap_or(EMPTY_TREE);
    git.run(["-c", "core.quotePath=false", "diff", "--no-color", "--no-ext-diff", "-M", base, sha, "--"])
}

/// A stash's changes to tracked files, then its untracked files.
pub fn stash_patch(git: &Git, stash: &Stash) -> Result<String> {
    let mut patch = git.run(["-c", "core.quotePath=false", "diff", "--no-color", "--no-ext-diff", "-M", &format!("{}^1", stash.sha), &stash.sha, "--"])?;
    let untracked = format!("{}^3", stash.sha);
    let has_untracked = git.exec(["rev-parse", "-q", "--verify", &untracked], Exec { allowed: &[1, 128], ..Default::default() })?.code == 0;
    if has_untracked {
        patch.push_str(&git.run(["-c", "core.quotePath=false", "diff", "--no-color", "--no-ext-diff", EMPTY_TREE, &untracked, "--"])?);
    }
    Ok(patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_records() {
        let rec = |sha: &str, parents: &str, refs: &str, co: &str, subject: &str| {
            [sha, &sha[..7], "Ada", "ada@example.com", "2 days ago", "2024-01-01T00:00:00+00:00", "1700000000", "1699999999", parents, refs, co, subject]
                .join("\x1f")
                + "\x1e"
        };
        let out = rec("aaaaaaaaaa", "bbbb cccc", "HEAD -> main, origin/main, tag: v1", "Bo <bo@x.org>\x1dCy", "Merge it")
            + "\n"
            + &rec("bbbbbbbbbb", "", "", "", "Root");
        let commits = parse_log(&out);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].parents, ["bbbb", "cccc"]);
        assert!(commits[0].is_merge());
        assert_eq!(commits[0].refs, ["HEAD -> main", "origin/main", "tag: v1"]);
        assert_eq!(commits[0].co_authors, [Person { name: "Bo".into(), email: "bo@x.org".into() }, Person { name: "Cy".into(), email: "".into() }]);
        assert!(commits[1].parents.is_empty());
        assert_eq!(commits[1].subject, "Root");
        assert_eq!(commits[0].author_time, 1699999999);
    }

    #[test]
    fn parses_track() {
        assert_eq!(parse_track("ahead 2, behind 1"), (2, 1, false));
        assert_eq!(parse_track("behind 3"), (0, 3, false));
        assert_eq!(parse_track("gone"), (0, 0, true));
        assert_eq!(parse_track(""), (0, 0, false));
    }

    #[test]
    fn parses_name_status() {
        let changes = parse_name_status("M\0a.txt\0R100\0old.txt\0new.txt\0A\0b.txt\0");
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[1], FileChange { status: 'R', path: "new.txt".into(), old_path: Some("old.txt".into()) });
        assert_eq!(changes[2].status, 'A');
    }

    #[test]
    fn parses_person() {
        assert_eq!(parse_person(" Ada Lovelace <ada@x.org> "), Person { name: "Ada Lovelace".into(), email: "ada@x.org".into() });
        assert_eq!(parse_person("<ada@x.org>").name, "ada@x.org");
    }
}

// --- Bisect ----------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BisectState {
    /// What was checked out when the bisect started (a branch name or SHA).
    pub start: String,
    pub bad: Option<String>,
    pub good: Vec<String>,
    pub skipped: Vec<String>,
    /// The words for bad and good: "bad"/"good" unless the bisect was started with other terms.
    pub terms: (String, String),
    /// Commits still to look at, once there is a bad and a good commit.
    pub remaining: Option<usize>,
    /// About how many more steps it takes.
    pub steps: Option<usize>,
    /// The first bad commit, once only it is left.
    pub culprit: Option<String>,
    /// Its subject: it may be newer than HEAD, so not in the commit list.
    pub culprit_subject: String,
}

/// The bisect in progress, if any.
pub fn bisect_state(git: &Git) -> Result<Option<BisectState>> {
    let Ok(start) = std::fs::read_to_string(git.git_path("BISECT_START")?) else { return Ok(None) };
    let terms = std::fs::read_to_string(git.git_path("BISECT_TERMS")?).unwrap_or_default();
    let mut words = terms.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim().to_owned());
    let bad_term = words.next().unwrap_or_else(|| "bad".into());
    let good_term = words.next().unwrap_or_else(|| "good".into());
    let mut state = BisectState { start: start.trim().to_owned(), terms: (bad_term.clone(), good_term.clone()), ..Default::default() };
    for line in git.run(["for-each-ref", "--format=%(refname) %(objectname)", "refs/bisect"])?.lines() {
        let Some((name, sha)) = line.split_once(' ') else { continue };
        let name = name.trim_start_matches("refs/bisect/");
        if name == bad_term {
            state.bad = Some(sha.to_owned());
        } else if name.starts_with(&format!("{good_term}-")) {
            state.good.push(sha.to_owned());
        } else if name.starts_with("skip-") {
            state.skipped.push(sha.to_owned());
        }
    }
    if let (Some(bad), false) = (&state.bad, state.good.is_empty()) {
        let mut args = vec!["rev-list".to_owned(), "--bisect-vars".to_owned(), bad.clone()];
        args.extend(state.good.iter().map(|g| format!("^{g}")));
        let out = git.exec(&args, Exec { allowed: &[1], ..Default::default() })?.stdout;
        let var = |name: &str| out.lines().find_map(|l| l.strip_prefix(&format!("{name}=")).map(|v| v.trim_matches('\'').to_owned()));
        // bisect_all counts the bad commit itself; only it left means it is the first bad one.
        let all: Option<usize> = var("bisect_all").and_then(|v| v.parse().ok());
        state.remaining = all.map(|a| a.saturating_sub(1));
        state.steps = var("bisect_steps").and_then(|v| v.parse().ok());
        if all == Some(1) {
            state.culprit = Some(bad.clone());
            state.culprit_subject = git.run(["log", "-1", "--format=%s", bad.as_str()])?.trim().to_owned();
        }
    }
    Ok(Some(state))
}

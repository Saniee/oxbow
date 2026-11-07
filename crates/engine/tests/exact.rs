//! What Oxbow works out itself to save asking git (relative dates, each commit's refs, a reused commit list) must
//! come out exactly as git's own answer would. These compare the two on real repositories.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use oxbow_engine::Exec;
use oxbow_engine::snapshot::LogCache;
use oxbow_engine::{CommitMark, Git, Settings, Snapshot, queries};

struct TempRepo {
    dir: PathBuf,
    git: Git,
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn repo() -> TempRepo {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("oxbow-exact-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = Git::new(&dir);
    git.run(["init", "-q", "-b", "main"]).unwrap();
    for (k, v) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("tag.gpgsign", "false"), ("core.autocrlf", "false")] {
        git.run(["config", k, v]).unwrap();
    }
    TempRepo { dir, git }
}

/// One fast-import commit: (mark, author and committer time, parent marks).
fn import(git: &Git, branch: &str, commits: &[(u32, i64, &[u32])]) {
    let mut stream = String::new();
    for (mark, time, parents) in commits {
        stream.push_str(&format!("commit refs/heads/{branch}\nmark :{mark}\nauthor T <t@x> {time} +0000\ncommitter T <t@x> {time} +0000\ndata {}\nc{mark}\n", format!("c{mark}").len()));
        let mut parents = parents.iter();
        if let Some(first) = parents.next() {
            stream.push_str(&format!("from :{first}\n"));
        }
        for p in parents {
            stream.push_str(&format!("merge :{p}\n"));
        }
        stream.push('\n');
    }
    git.exec(["fast-import", "--quiet"], Exec { input: Some(&stream), ..Default::default() }).unwrap();
}

#[test]
fn relative_dates_are_worded_as_git_words_them() {
    let r = repo();
    let now: i64 = 1_700_000_000;
    // Each side of every rounding step, then a sweep out to twenty years, and the future.
    let mut ago: Vec<i64> = vec![0, 1, 2, 59, 60, 89, 90, 91, 149, 150];
    for step in [89 * 60 + 30, 35 * 3600 + 1800, 13 * 86400 + 43200, 69 * 86400 + 43200, 364 * 86400 + 43200, 1824 * 86400 + 43200] {
        ago.extend((-3..=3).map(|d| step + d * 30));
    }
    let mut t = 100.0f64;
    while t < 20.0 * 365.0 * 86400.0 {
        ago.push(t as i64);
        t *= 1.09;
    }
    ago.push(-5);
    ago.sort();
    ago.dedup();
    let commits: Vec<(u32, i64, Vec<u32>)> = ago.iter().enumerate().map(|(i, a)| (i as u32 + 1, now - a, if i == 0 { vec![] } else { vec![i as u32] })).collect();
    let commits: Vec<(u32, i64, &[u32])> = commits.iter().map(|(m, t, p)| (*m, *t, p.as_slice())).collect();
    import(&r.git, "main", &commits);

    let now_text = now.to_string();
    let out = r.git.exec(["log", "--format=%at%x09%ar", "main"], Exec { env: &[("GIT_TEST_DATE_NOW", &now_text)], ..Default::default() }).unwrap();
    let mut checked = 0;
    for line in out.stdout.lines() {
        let (time, words) = line.split_once('\t').unwrap();
        assert_eq!(queries::relative_date(time.parse().unwrap(), now), words, "{} seconds ago", now - time.parse::<i64>().unwrap());
        checked += 1;
    }
    assert_eq!(checked, ago.len());
}

/// Each commit's refs, as sets, from git's %D.
fn refs_by_git(git: &Git) -> Vec<(String, BTreeSet<String>)> {
    git.run(["log", "--date-order", "--format=%H%x09%D", "HEAD"])
        .unwrap()
        .lines()
        .map(|l| {
            let (sha, refs) = l.split_once('\t').unwrap();
            (sha.to_owned(), refs.split(", ").filter(|s| !s.is_empty()).map(str::to_owned).collect())
        })
        .collect()
}

fn refs_by_oxbow(snap: &Snapshot) -> Vec<(String, BTreeSet<String>)> {
    snap.commits.iter().map(|c| (c.sha.clone(), c.refs.iter().cloned().collect())).collect()
}

#[test]
fn each_commits_refs_are_the_ones_git_lists() {
    let r = repo();
    import(&r.git, "main", &[(1, 1000, &[]), (2, 2000, &[1]), (3, 3000, &[2])]);
    r.git.run(["checkout", "-q", "main"]).unwrap();
    r.git.run(["branch", "same-as-main"]).unwrap();
    r.git.run(["branch", "older", "main~1"]).unwrap();
    r.git.run(["tag", "light"]).unwrap();
    r.git.run(["tag", "-a", "annotated", "main~1", "-m", "An annotated tag"]).unwrap();
    // A tag of a tag: git peels it all the way to the commit.
    r.git.run(["tag", "-a", "outer", "annotated", "-m", "A tag of a tag"]).unwrap();
    r.git.run(["remote", "add", "origin", "https://example.invalid/repo.git"]).unwrap();
    r.git.run(["update-ref", "refs/remotes/origin/main", "main~1"]).unwrap();
    r.git.run(["update-ref", "refs/remotes/origin/feature", "main~2"]).unwrap();
    r.git.run(["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]).unwrap();

    let mut cache = LogCache::default();
    let snap = Snapshot::load_cached(&r.git, &Settings::default(), None, &mut cache).unwrap();
    assert_eq!(refs_by_oxbow(&snap), refs_by_git(&r.git));
    // Reused from the cache, after refs moved without HEAD moving.
    r.git.run(["branch", "-f", "older", "main~2"]).unwrap();
    r.git.run(["tag", "-d", "light"]).unwrap();
    let snap = Snapshot::load_cached(&r.git, &Settings::default(), None, &mut cache).unwrap();
    assert_eq!(refs_by_oxbow(&snap), refs_by_git(&r.git));

    // Detached.
    r.git.run(["checkout", "-q", "--detach", "main~1"]).unwrap();
    let snap = Snapshot::load_cached(&r.git, &Settings::default(), None, &mut cache).unwrap();
    assert_eq!(refs_by_oxbow(&snap), refs_by_git(&r.git));
}

#[test]
fn children_come_before_parents_even_when_the_clock_was_off() {
    let r = repo();
    // 2 was committed with a clock behind its parent's; git's default order then lists 1 before its child 2.
    import(&r.git, "main", &[(1, 3000, &[]), (2, 1000, &[1]), (3, 4000, &[1]), (4, 5000, &[2, 3])]);
    let default = r.git.run(["log", "--format=%s", "main"]).unwrap();
    assert_eq!(default.lines().collect::<Vec<_>>(), ["c4", "c3", "c1", "c2"], "the case this guards against");

    r.git.run(["checkout", "-q", "main"]).unwrap();
    let snap = Snapshot::load(&r.git, &Settings::default(), None).unwrap();
    let at = |sha: &str| snap.commits.iter().position(|c| c.sha == sha);
    for (i, c) in snap.commits.iter().enumerate() {
        for p in &c.parents {
            assert!(at(p).is_none_or(|j| j > i), "{} is listed before its child {}", p, c.subject);
        }
    }
    assert_eq!(snap.commits.len(), 4);
}

#[test]
fn a_main_branch_moving_shows_without_head_moving() {
    let r = repo();
    import(&r.git, "main", &[(1, 1000, &[]), (2, 2000, &[1])]);
    r.git.run(["checkout", "-q", "-b", "feature", "main"]).unwrap();
    r.git.run(["branch", "-f", "main", "main~1"]).unwrap();
    let mut cache = LogCache::default();
    let snap = Snapshot::load_cached(&r.git, &Settings::default(), None, &mut cache).unwrap();
    let feature = snap.commits[0].sha.clone();
    assert_ne!(snap.mark(&feature), CommitMark::Merged);

    // main catches up (a pull elsewhere, a fetch moving its upstream): HEAD stays, the list is reused.
    r.git.run(["branch", "-f", "main", "feature"]).unwrap();
    let snap = Snapshot::load_cached(&r.git, &Settings::default(), None, &mut cache).unwrap();
    assert_eq!(snap.mark(&feature), CommitMark::Merged);
}

#[test]
fn head_read_from_the_files_is_the_one_git_reports() {
    let r = repo();
    import(&r.git, "main", &[(1, 1000, &[]), (2, 2000, &[1])]);
    r.git.run(["checkout", "-q", "main"]).unwrap();
    let git_says = || queries::head_sha(&r.git).unwrap();
    assert_eq!(queries::head_from_files(&r.git), git_says());
    // Packed, with a longer name that ends the same way packed alongside.
    r.git.run(["branch", "x/main"]).unwrap();
    r.git.run(["update-ref", "refs/heads/x/main", "main~1"]).unwrap();
    r.git.run(["pack-refs", "--all"]).unwrap();
    assert_eq!(queries::head_from_files(&r.git), git_says());
    r.git.run(["checkout", "-q", "--detach", "main~1"]).unwrap();
    assert_eq!(queries::head_from_files(&r.git), git_says());
}

#[test]
fn files_are_staged_by_their_exact_name_however_many_there_are() {
    use oxbow_engine::status;
    let r = repo();
    import(&r.git, "main", &[(1, 1000, &[])]);
    r.git.run(["checkout", "-q", "main"]).unwrap();
    // A name that is also a pattern: as a pattern, [ab].txt matches a.txt.
    std::fs::write(r.dir.join("[ab].txt"), "x\n").unwrap();
    std::fs::write(r.dir.join("a.txt"), "x\n").unwrap();
    let entries = status::status(&r.git).unwrap();
    let bracket: Vec<_> = entries.iter().filter(|e| e.path == "[ab].txt").cloned().collect();
    status::stage(&r.git, &bracket).unwrap();
    let staged: Vec<String> = status::status(&r.git).unwrap().into_iter().filter(|e| e.is_staged()).map(|e| e.path).collect();
    assert_eq!(staged, ["[ab].txt"]);

    // More paths than a Windows command line holds.
    let dir = r.dir.join("many-files-with-rather-long-names");
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..2500 {
        std::fs::write(dir.join(format!("a-file-with-a-long-name-number-{i:05}.txt")), "x\n").unwrap();
    }
    let all = status::status(&r.git).unwrap();
    assert_eq!(all.len(), 2502);
    status::stage(&r.git, &all).unwrap();
    assert!(status::status(&r.git).unwrap().iter().all(|e| e.is_staged()));
    status::unstage(&r.git, &status::status(&r.git).unwrap()).unwrap();
    assert!(status::status(&r.git).unwrap().iter().all(|e| e.is_untracked()));
    status::discard(&r.git, &status::status(&r.git).unwrap(), false).unwrap();
    assert!(status::status(&r.git).unwrap().is_empty());
}

#[test]
fn status_saves_the_times_of_files_that_only_look_changed() {
    use oxbow_engine::status;
    let r = repo();
    std::fs::write(r.dir.join("a.txt"), "x\n").unwrap();
    r.git.run(["add", "a.txt"]).unwrap();
    r.git.run(["commit", "-q", "-m", "a"]).unwrap();
    // Same content, new time: as a formatter rewriting it unchanged leaves it.
    let later = std::time::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000_000);
    std::fs::File::options().write(true).open(r.dir.join("a.txt")).unwrap().set_modified(later).unwrap();
    let saved_mtime = || {
        let out = r.git.run(["ls-files", "--debug", "a.txt"]).unwrap();
        let line = out.lines().find(|l| l.trim_start().starts_with("mtime:")).unwrap().to_owned();
        line.trim_start().trim_start_matches("mtime: ").split(':').next().unwrap().parse::<u64>().unwrap()
    };
    assert_ne!(saved_mtime(), 2_000_000_000);
    assert!(status::status(&r.git).unwrap().is_empty());
    assert_eq!(saved_mtime(), 2_000_000_000);
}

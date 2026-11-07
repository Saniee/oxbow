//! What each action ran, newest first. Read-only git commands are left out, so the log shows what changed things.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogStatus {
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub id: u64,
    pub title: String,
    pub time: SystemTime,
    /// The git commands it ran, as shell-like text.
    pub commands: Vec<String>,
    pub status: LogStatus,
}

const MAX_ENTRIES: usize = 100;

#[derive(Default)]
struct Inner {
    items: VecDeque<LogEntry>,
    next_id: u64,
    current: Option<u64>,
    version: u64,
}

#[derive(Default)]
pub struct CommandLog {
    inner: Mutex<Inner>,
}

impl CommandLog {
    /// Starts an action: mutating commands run until `finish` are listed under it.
    pub fn start(&self, title: &str) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.push(title.trim_end_matches('…'), LogStatus::Running);
        inner.current = Some(id);
        id
    }

    pub fn finish(&self, id: u64, ok: bool) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(entry) = inner.items.iter_mut().find(|e| e.id == id) {
            entry.status = if ok { LogStatus::Done } else { LogStatus::Failed };
        }
        if inner.current == Some(id) {
            inner.current = None;
        }
        inner.version += 1;
    }

    /// Called by `Git` after every command.
    pub fn record(&self, args: &[String], code: i32) {
        if is_read_only(args) {
            return;
        }
        let mut line = format_command(args);
        if code != 0 {
            line.push_str(&format!("  (exit {code})"));
        }
        let mut inner = self.inner.lock().unwrap();
        let id = match inner.current {
            Some(id) if inner.items.iter().any(|e| e.id == id) => id,
            _ => {
                let title = format!("git {}", subcommand(args).first().map(String::as_str).unwrap_or(""));
                inner.push(title.trim(), LogStatus::Done)
            }
        };
        if let Some(entry) = inner.items.iter_mut().find(|e| e.id == id) {
            entry.commands.push(line);
        }
        inner.version += 1;
    }

    pub fn entries(&self) -> Vec<LogEntry> {
        self.inner.lock().unwrap().items.iter().cloned().collect()
    }

    /// Changes whenever the log does.
    pub fn version(&self) -> u64 {
        self.inner.lock().unwrap().version
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.items.clear();
        inner.version += 1;
    }
}

impl Inner {
    fn push(&mut self, title: &str, status: LogStatus) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.items.push_front(LogEntry { id, title: title.to_owned(), time: SystemTime::now(), commands: Vec::new(), status });
        self.items.truncate(MAX_ENTRIES);
        self.version += 1;
        id
    }
}

/// Subcommands that never change the repository.
const READ_ONLY: &[&str] = &[
    "blame", "cat-file", "check-ignore", "describe", "diff", "diff-tree", "for-each-ref", "grep", "log", "ls-files",
    "ls-remote", "ls-tree", "merge-base", "name-rev", "rev-list", "rev-parse", "shortlog", "show", "show-ref", "status",
    "symbolic-ref", "var", "version", "worktree list", "remote get-url", "stash list", "stash show", "reflog",
    "bisect log", "bisect visualize",
];

/// The subcommand and its arguments, skipping `-c key=value` and other global options.
fn subcommand(args: &[String]) -> &[String] {
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') {
        i += if args[i] == "-c" || args[i] == "-C" { 2 } else { 1 };
    }
    &args[i.min(args.len())..]
}

pub fn is_read_only(args: &[String]) -> bool {
    let sub = subcommand(args);
    let Some(command) = sub.first() else { return true };
    let next = sub.get(1).map(String::as_str).unwrap_or("");
    if READ_ONLY.contains(&command.as_str()) || READ_ONLY.contains(&format!("{command} {next}").as_str()) {
        return true;
    }
    let rest = &sub[1..];
    let has = |flags: &[&str]| rest.iter().any(|a| flags.contains(&a.as_str()));
    match command.as_str() {
        "config" => has(&["--get", "--get-all", "--list", "-l"]),
        "branch" => has(&["--list", "--show-current", "--merged", "--no-merged", "--contains"]),
        "tag" => rest.is_empty() || has(&["-l", "--list"]),
        "remote" => rest.is_empty() || next == "-v" || next == "show",
        _ => false,
    }
}

/// As a shell would show it: arguments with spaces or quotes are quoted.
pub fn format_command(args: &[String]) -> String {
    let plain = |a: &str| !a.is_empty() && a.chars().all(|c| c.is_alphanumeric() || "_@%+=:,./^~{}-".contains(c));
    std::iter::once("git".to_owned())
        .chain(args.iter().map(|a| if plain(a) { a.clone() } else { format!("'{}'", a.replace('\'', "'\\''")) }))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn read_only_commands() {
        assert!(is_read_only(&args("log -n5")));
        assert!(is_read_only(&args("-c core.quotePath=false diff --cached")));
        assert!(is_read_only(&args("stash list")));
        assert!(is_read_only(&args("branch --show-current")));
        assert!(is_read_only(&args("tag")));
        assert!(!is_read_only(&args("tag v1")));
        assert!(!is_read_only(&args("stash push")));
        assert!(!is_read_only(&args("commit -F -")));
        assert!(!is_read_only(&args("branch feature")));
    }

    #[test]
    fn formats_commands() {
        assert_eq!(format_command(&args("commit -F -")), "git commit -F -");
        assert_eq!(format_command(&["commit".into(), "-m".into(), "it's done".into()]), "git commit -m 'it'\\''s done'");
    }

    #[test]
    fn groups_commands_under_actions() {
        let log = CommandLog::default();
        log.record(&args("log"), 0);
        assert!(log.entries().is_empty());
        let id = log.start("Committing…");
        log.record(&args("commit -F -"), 0);
        log.finish(id, true);
        log.record(&args("fetch"), 1);
        let entries = log.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "git fetch");
        assert_eq!(entries[0].commands, ["git fetch  (exit 1)"]);
        assert_eq!(entries[1].title, "Committing");
        assert_eq!(entries[1].status, LogStatus::Done);
    }
}

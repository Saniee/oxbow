use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::command_log::CommandLog;

#[derive(Debug, Clone)]
pub enum GitError {
    /// `git` could not be started at all (not installed, not on PATH).
    Spawn(String),
    /// `git` ran and exited with a failure.
    Failed { args: Vec<String>, code: i32, stderr: String },
    /// Something Oxbow itself refused to do, with a message for the user.
    User(String),
}

impl GitError {
    pub fn user(message: impl Into<String>) -> Self {
        GitError::User(message.into())
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitError::Spawn(e) => write!(f, "Could not run git: {e}"),
            GitError::Failed { args, stderr, code } => {
                let detail = stderr.trim();
                if detail.is_empty() { write!(f, "git {} failed (exit {code})", args.join(" ")) } else { f.write_str(detail) }
            }
            GitError::User(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for GitError {}

pub type Result<T> = std::result::Result<T, GitError>;

/// Options for one git command.
#[derive(Default, Clone, Copy)]
pub struct Exec<'a> {
    /// Extra environment variables.
    pub env: &'a [(&'a str, &'a str)],
    /// Exit codes that count as success (0 always does).
    pub allowed: &'a [i32],
    /// Written to stdin; otherwise stdin is empty.
    pub input: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// The git CLI bound to one working tree. Cheap to clone; clones share the cached git directory and the command log.
#[derive(Clone)]
pub struct Git {
    git_path: PathBuf,
    root: PathBuf,
    git_dir: Arc<OnceLock<PathBuf>>,
    log: Option<Arc<CommandLog>>,
}

impl fmt::Debug for Git {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Git").field("root", &self.root).finish()
    }
}

impl Git {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { git_path: PathBuf::from("git"), root: root.into(), git_dir: Arc::default(), log: None }
    }

    /// The working tree root of the repository containing `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let probe = Git::new(path.as_ref());
        let root = probe.run(["rev-parse", "--show-toplevel"])?;
        Ok(Git::new(PathBuf::from(root.trim())))
    }

    /// Records every command that changes the repository in `log`.
    pub fn with_log(mut self, log: Arc<CommandLog>) -> Self {
        self.log = Some(log);
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn command_log(&self) -> Option<&Arc<CommandLog>> {
        self.log.as_ref()
    }

    /// Runs git and returns its stdout; a failing exit code is an error.
    pub fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Ok(self.exec(args, Exec::default())?.stdout)
    }

    /// Runs git. A command that can't take index.lock is retried a few times: another tool (an editor's git
    /// integration, say) may hold it for a moment, and git gives up before changing anything.
    pub fn exec<I, S>(&self, args: I, options: Exec<'_>) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let args: Vec<String> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
        let mut attempt = 1;
        loop {
            match self.exec_once(&args, options) {
                Err(GitError::Failed { ref stderr, .. }) if attempt < 10 && stderr.contains("index.lock': File exists") => {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(200));
                }
                other => return other,
            }
        }
    }

    fn exec_once(&self, args: &[String], options: Exec<'_>) -> Result<Output> {
        let mut cmd = Command::new(&self.git_path);
        cmd.args(args)
            .current_dir(&self.root)
            // There is no terminal to prompt in: fail instead of waiting. Credential helpers still show their own UI.
            .env("GIT_TERMINAL_PROMPT", "0")
            // Never wait on an editor; Oxbow passes messages itself.
            .env("GIT_EDITOR", ":")
            .env("GIT_SEQUENCE_EDITOR", ":")
            // Nor on a pager, which some commands (stash list, log) start even without a terminal.
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .stdin(if options.input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in options.env {
            cmd.env(key, value);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| GitError::Spawn(e.to_string()))?;
        if let Some(input) = options.input
            && let Some(mut stdin) = child.stdin.take() {
                // A command that exits early closes the pipe; its exit code says what went wrong.
                let _ = stdin.write_all(input.as_bytes());
            }
        let out = child.wait_with_output().map_err(|e| GitError::Spawn(e.to_string()))?;
        let code = out.status.code().unwrap_or(-1);
        if let Some(log) = &self.log {
            log.record(args, code);
        }
        let output = Output {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            code,
        };
        if code == 0 || options.allowed.contains(&code) {
            Ok(output)
        } else {
            let stderr = if output.stderr.trim().is_empty() { output.stdout } else { output.stderr };
            Err(GitError::Failed { args: args.to_vec(), code, stderr })
        }
    }

    /// This working tree's git directory (HEAD, index, rebase-merge, …).
    pub fn git_dir(&self) -> Result<&Path> {
        if let Some(dir) = self.git_dir.get() {
            return Ok(dir);
        }
        let dir = PathBuf::from(self.run(["rev-parse", "--absolute-git-dir"])?.trim());
        Ok(self.git_dir.get_or_init(|| dir))
    }

    /// A per-worktree file in the git directory, such as MERGE_HEAD or rebase-merge.
    pub fn git_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.git_dir()?.join(name))
    }
}

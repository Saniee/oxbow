//! Oxbow as git's sequence editor. An interactive rebase opens its todo list in an editor; Oxbow points git at its own
//! executable (as lazygit does), which puts a `break` before the first step and exits. The rebase then stops before
//! applying anything, and Oxbow rewrites the list itself (see [`crate::todo`]).
//!
//! Every binary that rebases must call [`run_if_requested`] first thing in `main` (the test binaries do it from a
//! constructor).

use std::path::Path;

use crate::git::{GitError, Result};

/// Set on the rebase's environment; its value says what the editor should do.
pub const ENV: &str = "OXBOW_EDITOR";
const PAUSE: &str = "pause";

/// When git started this process as Oxbow's sequence editor, does the job and returns the exit code to exit with.
pub fn run_if_requested() -> Option<i32> {
    let mode = std::env::var(ENV).ok()?;
    let file = std::env::args_os().nth(1)?;
    let file = Path::new(&file);
    // Only ever a rebase todo list.
    if mode != PAUSE || file.file_name().is_none_or(|n| n != "git-rebase-todo") {
        return Some(2);
    }
    let pause = || -> std::io::Result<()> {
        let todo = std::fs::read_to_string(file)?;
        std::fs::write(file, format!("break\n{todo}"))
    };
    Some(if pause().is_ok() { 0 } else { 1 })
}

/// The environment that makes a rebase stop before its first step.
pub(crate) fn pause_env() -> Result<[(&'static str, String); 2]> {
    let exe = std::env::current_exe().map_err(|e| GitError::user(format!("Could not find Oxbow's executable: {e}")))?;
    // git runs the editor through its shell: forward slashes, single-quoted.
    let path = exe.to_string_lossy().replace('\\', "/").replace('\'', r"'\''");
    Ok([("GIT_SEQUENCE_EDITOR", format!("'{path}'")), (ENV, PAUSE.to_owned())])
}

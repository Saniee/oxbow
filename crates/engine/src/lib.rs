//! Oxbow's Git backend. Everything that touches a repository goes through the user's own `git` binary, so hooks,
//! config and credential helpers behave exactly as on the command line. No UI code here, so the front end can be
//! swapped without touching it.

pub mod command_log;
pub mod diff;
pub mod editor;
mod git;
pub mod graph;
pub mod ops;
pub mod patch;
pub mod queries;
pub mod rebase;
pub mod snapshot;
pub mod status;
pub mod todo;
pub mod undo;
pub mod watch;

pub use command_log::{CommandLog, LogEntry, LogStatus};
pub use git::{Exec, Git, GitError, Output, Result};
pub use snapshot::{CommitMark, Settings, Snapshot};

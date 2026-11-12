//! Who is working, from git - and the small process helpers every git call
//! uses.
//!
//! wipe supports git and nothing else: identities and the people on a board
//! come from git (`user.name`/`user.email`, the authors in its history), and
//! only inside a git repository. A board anywhere else - a plain folder, or a
//! workspace of another version control such as Plastic SCM - is plain files,
//! and falls back to the OS account. No other tool is ever asked who you are.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

/// Hard cap on any identity probe, so a hung `git` never blocks a `wipe`
/// command or a daemon request.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Cap on history walked when discovering authors, so large repos stay fast.
const AUTHOR_SCAN_LIMIT: &str = "2000";

/// Whether `start` is inside a git repository (a `.git` directory, or the
/// `.git` file of a worktree or submodule, at or above it).
pub fn in_git(start: &Path) -> bool {
    let abs = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    abs.ancestors().any(|d| d.join(".git").exists())
}

/// The current user's git identity (`Name <email>`), or `None` outside a git
/// repository or when git has none set.
pub fn identity(start: &Path) -> Option<String> {
    in_git(start).then(|| git_identity(start)).flatten()
}

/// Distinct authors in the git history as `(name, email)`; empty outside git.
pub fn authors(start: &Path) -> Vec<(String, String)> {
    if in_git(start) {
        git_authors(start)
    } else {
        Vec::new()
    }
}

/// The OS account name (`$USERNAME` on Windows, `$USER` elsewhere), a last-ditch
/// identity when no VCS can supply one.
pub fn system_user() -> Option<String> {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// --- per-VCS implementations ----------------------------------------------

/// Strip Windows' `\\?\` verbatim prefix, which some CLIs (git `-C`) reject.
/// Handles UNC verbatim paths (`\\?\UNC\server\share` → `\\server\share`) so repos
/// on network shares still resolve.
pub fn plain(root: &Path) -> PathBuf {
    let s = root.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => root.to_path_buf(),
    }
}

/// Run a prepared command with a hard timeout, returning trimmed stdout on success.
/// On timeout (or spawn/exec failure) returns `None`; the abandoned child is left to
/// exit on its own rather than blocking us - identity is best-effort.
fn run_timed(mut cmd: Command) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(cmd.output());
    });
    match rx.recv_timeout(PROBE_TIMEOUT) {
        Ok(Ok(out)) => finish(out),
        _ => None,
    }
}

/// A `Command` that never opens a console window on Windows. The tray/daemon
/// has no console, so each console child (git, cm, ...) would otherwise get a
/// visible window of its own. Output is still captured through pipes.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Run `program args...` in `dir` (via `-C`), returning trimmed stdout on success.
fn output_in(dir: &Path, program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = command(program);
    cmd.arg("-C").arg(plain(dir)).args(args);
    run_timed(cmd)
}

fn finish(out: std::process::Output) -> Option<String> {
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn combine(name: Option<String>, email: Option<String>) -> Option<String> {
    match (name, email) {
        (Some(n), Some(e)) => Some(format!("{n} <{e}>")),
        (Some(n), None) => Some(n),
        (None, Some(e)) => Some(e),
        (None, None) => None,
    }
}

fn git_identity(root: &Path) -> Option<String> {
    let name = output_in(root, "git", &["config", "user.name"]);
    let email = output_in(root, "git", &["config", "user.email"]);
    combine(name, email)
}

fn git_authors(root: &Path) -> Vec<(String, String)> {
    let Some(out) = output_in(
        root,
        "git",
        &[
            "--no-pager",
            "log",
            "-n",
            AUTHOR_SCAN_LIMIT,
            "--format=%an\t%ae",
        ],
    ) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut authors = Vec::new();
    for line in out.lines() {
        if let Some((name, email)) = line.split_once('\t') {
            if seen.insert(email.to_string()) {
                authors.push((name.to_string(), email.to_string()));
            }
        }
    }
    authors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_comes_from_git_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(!in_git(root));
        assert_eq!(identity(root), None);
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap();
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "Ada Lovelace"]);
        git(&["config", "user.email", "ada@example.com"]);
        assert!(in_git(root));
        assert_eq!(
            identity(root).as_deref(),
            Some("Ada Lovelace <ada@example.com>")
        );
    }

    /// Another version control's workspace is not git: no identity from it.
    #[test]
    fn other_version_control_is_not_asked() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".plastic")).unwrap();
        std::fs::create_dir(dir.path().join(".hg")).unwrap();
        assert!(!in_git(dir.path()));
        assert_eq!(identity(dir.path()), None);
        assert!(authors(dir.path()).is_empty());
    }
}

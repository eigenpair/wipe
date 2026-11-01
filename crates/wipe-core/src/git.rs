//! Lightweight git integration by shelling out to the `git` CLI.
//!
//! wipe is git-native, so `git` is always present. Rather than pull in a heavy
//! libgit2/gitoxide dependency (and its native build), we invoke `git` for the
//! few things the UI needs: the commit history of the board, and the contents of
//! a tracked file at a past commit (used for the board-rewind feature).

use std::path::Path;

use serde::Serialize;

use crate::error::{Error, Result};

/// A single commit's metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct CommitInfo {
    /// Full commit hash.
    pub hash: String,
    /// Abbreviated hash.
    pub short: String,
    /// Author display name.
    pub author_name: String,
    /// Author email.
    pub author_email: String,
    /// Author date, ISO-8601 / RFC-3339.
    pub date: String,
    /// Commit subject line.
    pub subject: String,
}

// Field/record separators unlikely to appear in commit metadata.
const FS: char = '\u{1f}';
const RS: char = '\u{1e}';

/// Whether `path` is inside a git work tree.
pub fn is_repo(root: &Path) -> bool {
    run(root, &["rev-parse", "--is-inside-work-tree"])
        .map(|o| o.trim() == "true")
        .unwrap_or(false)
}

/// Return the commit history, most recent first. If `pathspec` is given, only
/// commits touching that path are returned. `limit` caps the number of commits.
pub fn log(root: &Path, pathspec: Option<&str>, limit: Option<usize>) -> Result<Vec<CommitInfo>> {
    let format = format!("--format=%H{FS}%h{FS}%an{FS}%ae{FS}%aI{FS}%s{RS}");
    let mut args: Vec<String> = vec![
        "--no-pager".into(),
        "log".into(),
        format,
        "--no-color".into(),
    ];
    if let Some(l) = limit {
        args.push("-n".into());
        args.push(l.to_string());
    }
    if let Some(p) = pathspec {
        args.push("--".into());
        args.push(p.to_string());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run(root, &refs)?;
    Ok(parse_log(&out))
}

/// Commits in `range` (e.g. `a..b`), most recent first, capped at `limit`.
pub fn log_range(root: &Path, range: &str, limit: usize) -> Result<Vec<CommitInfo>> {
    let format = format!("--format=%H{FS}%h{FS}%an{FS}%ae{FS}%aI{FS}%s{RS}");
    let n = limit.to_string();
    let out = run(
        root,
        &["--no-pager", "log", &format, "--no-color", "-n", &n, range],
    )?;
    Ok(parse_log(&out))
}

/// Read the contents of a tracked file as of a specific commit/ref. Returns
/// `None` if the file did not exist at that revision.
pub fn file_at_commit(root: &Path, rev: &str, relpath: &str) -> Result<Option<String>> {
    // `rev:./path` resolves relative to git's cwd (our `-C root`), whereas a bare
    // `rev:path` is relative to the REPO root - which breaks for a board nested in
    // a subdirectory of the repo (e.g. a monorepo's `apps/foo/.wipe`). Forward
    // slashes work on all platforms for git pathspecs.
    let spec = format!("{rev}:./{}", relpath.replace('\\', "/"));
    match run(root, &["--no-pager", "show", &spec]) {
        Ok(s) => Ok(Some(s)),
        // A non-zero exit here means "path not present at rev", not a hard error.
        Err(Error::Message(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Commits whose message mentions `ticket_id` (e.g. `T-81`, as in "fix login
/// (T-81)"), most recent first, capped at `limit`. wipe's own bookkeeping commits
/// (`chore(wipe): ...`) are skipped - they touch the board, not the code. The id
/// must stand alone: `T-8` does not match a message that only mentions `T-81`.
pub fn commits_mentioning(root: &Path, ticket_id: &str, limit: usize) -> Result<Vec<CommitInfo>> {
    // Subject and body both come back so the exact-token check below sees the whole
    // message; the body is the 7th field.
    let format = format!("--format=%H{FS}%h{FS}%an{FS}%ae{FS}%aI{FS}%s{FS}%b{RS}");
    let grep = format!("--grep={ticket_id}");
    // Over-fetch: git's --grep is a substring match, so `T-8` also finds `T-81`;
    // the exact word-boundary filter runs below.
    let fetch = (limit.max(1) * 4).to_string();
    let out = run(
        root,
        &[
            "--no-pager",
            "log",
            &format,
            "--no-color",
            "--fixed-strings",
            "-i",
            &grep,
            "-n",
            &fetch,
        ],
    )?;
    let mut hits = Vec::new();
    for rec in out.split(RS) {
        let f: Vec<&str> = rec.trim_start_matches('\n').split(FS).collect();
        if f.len() < 7 {
            continue;
        }
        let subject = f[5].to_string();
        if subject.starts_with("chore(wipe)") {
            continue;
        }
        if !mentions_id(&format!("{subject}\n{}", f[6]), ticket_id) {
            continue;
        }
        hits.push(CommitInfo {
            hash: f[0].to_string(),
            short: f[1].to_string(),
            author_name: f[2].to_string(),
            author_email: f[3].to_string(),
            date: f[4].to_string(),
            subject,
        });
        if hits.len() >= limit {
            break;
        }
    }
    Ok(hits)
}

/// Whether `text` mentions `id` as a standalone token (case-insensitive): the
/// characters around it must not continue the id (`T-8` vs `T-81` / `XT-8`).
pub fn mentions_id(text: &str, id: &str) -> bool {
    let hay = text.to_ascii_uppercase();
    let needle = id.to_ascii_uppercase();
    let bytes = hay.as_bytes();
    let mut from = 0;
    while let Some(pos) = hay[from..].find(&needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_digit();
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// How many paths under `pathspec` differ from `HEAD` (modified, staged, or
/// untracked) - e.g. uncommitted board files.
pub fn changed_count(root: &Path, pathspec: &str) -> Result<usize> {
    Ok(run(
        root,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            pathspec,
        ],
    )?
    .lines()
    .filter(|l| !l.trim().is_empty())
    .count())
}

/// The most recent commit that touched `relpath`, if any (for attribution).
pub fn last_change(root: &Path, relpath: &str) -> Result<Option<CommitInfo>> {
    Ok(log(root, Some(relpath), Some(1))?.into_iter().next())
}

/// Turn an identity string into a git `(name, email)` pair. Accepts the usual
/// `Name <email>` form; a bare agent id (e.g. `claude-dev`) gets a stable,
/// synthetic `<id>@wipe.local` address so `git` accepts the attribution.
pub(crate) fn author_ident(author: &str) -> (String, String) {
    if let Some(open) = author.rfind('<') {
        if let Some(rel_close) = author[open..].find('>') {
            let name = author[..open].trim();
            let email = author[open + 1..open + rel_close].trim();
            if !email.is_empty() {
                let name = if name.is_empty() { email } else { name };
                return (name.to_string(), email.to_string());
            }
        }
    }
    let slug: String = author
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = if slug.trim_matches('-').is_empty() {
        "agent".to_string()
    } else {
        slug
    };
    (author.to_string(), format!("{slug}@wipe.local"))
}

/// Compute the git blob hash of `bytes` (identical to `git hash-object`).
pub fn blob_hash(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", bytes.len()).as_bytes());
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// All tracked files as `(blob_hash, repo-relative path)` pairs.
pub fn tracked_blobs(root: &Path) -> Result<Vec<(String, String)>> {
    let out = run(root, &["ls-files", "-s"])?;
    let mut blobs = Vec::new();
    for line in out.lines() {
        // Format: "<mode> <hash> <stage>\t<path>"
        if let Some((meta, path)) = line.split_once('\t') {
            let mut cols = meta.split_whitespace();
            let _mode = cols.next();
            if let Some(hash) = cols.next() {
                blobs.push((hash.to_string(), path.to_string()));
            }
        }
    }
    Ok(blobs)
}

/// Distinct commit authors as `(name, email)`, most-recent first.
pub fn authors(root: &Path) -> Result<Vec<(String, String)>> {
    let out = run(
        root,
        &["--no-pager", "log", &format!("--format=%an{FS}%ae")],
    )?;
    let mut seen = std::collections::HashSet::new();
    let mut authors = Vec::new();
    for line in out.lines() {
        if let Some((name, email)) = line.split_once(FS) {
            if seen.insert(email.to_string()) {
                authors.push((name.to_string(), email.to_string()));
            }
        }
    }
    Ok(authors)
}

/// The configured git identity for this repo (`Name <email>`), if set. Used to
/// attribute UI-driven changes in the activity timeline to the human at the keyboard.
pub fn config_identity(root: &Path) -> Option<String> {
    let get = |key: &str| {
        run(root, &["config", key])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    match (get("user.name"), get("user.email")) {
        (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
        (Some(name), None) => Some(name),
        (None, Some(email)) => Some(email),
        (None, None) => None,
    }
}

pub(crate) fn parse_log(out: &str) -> Vec<CommitInfo> {
    out.split(RS)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let mut f = record.split(FS);
            Some(CommitInfo {
                hash: f.next()?.to_string(),
                short: f.next()?.to_string(),
                author_name: f.next()?.to_string(),
                author_email: f.next()?.to_string(),
                date: f.next()?.to_string(),
                subject: f.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// Strip Windows' `\\?\` verbatim prefix, which `git -C` does not accept (UNC-aware).
fn plain(root: &Path) -> std::path::PathBuf {
    crate::vcs::plain(root)
}

/// Run a git command in `root`, returning stdout on success or an
/// [`Error::Message`] carrying stderr on failure.
fn run(root: &Path, args: &[&str]) -> Result<String> {
    let out = crate::vcs::command("git")
        .arg("-C")
        .arg(plain(root))
        .args(args)
        .output()
        .map_err(|e| Error::msg(format!("failed to run git: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(Error::msg(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(root: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_repo(dir.path()));
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "Tester"]);
        dir
    }

    #[test]
    fn log_and_show_roundtrip() {
        let dir = repo();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "v1\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-q", "-m", "first commit"]);

        assert!(is_repo(root));
        let history = log(root, None, None).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(
            (
                history[0].subject.as_str(),
                history[0].author_email.as_str()
            ),
            ("first commit", "t@example.com")
        );
        let head = &history[0].hash;
        assert_eq!(
            file_at_commit(root, head, "a.txt").unwrap().as_deref(),
            Some("v1\n")
        );
        assert_eq!(file_at_commit(root, head, "missing.txt").unwrap(), None);
    }

    #[test]
    fn commits_mentioning_match_standalone_ids_in_subject_and_body() {
        for (text, hit) in [
            ("fix login (T-8)", true),
            ("t-8: lowercase works", true),
            ("see T-81 and T-8.", true),
            ("fix login (T-81)", false),
            ("XT-8 is another id", false),
        ] {
            assert_eq!(mentions_id(text, "T-8"), hit, "{text}");
        }
        let dir = repo();
        let root = dir.path();
        for (file, msg) in [
            ("a", "fix login (T-8)"),
            ("b", "unrelated work on T-81"),
            ("c", "refactor\n\nfollow-up for t-8"),
            ("d", "chore(wipe): update T-8"),
        ] {
            std::fs::write(root.join(file), msg).unwrap();
            git(root, &["add", "."]);
            git(root, &["commit", "-q", "-m", msg]);
        }
        let subjects: Vec<String> = commits_mentioning(root, "T-8", 10)
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect();
        // Newest first; T-81 and wipe's own bookkeeping commits are excluded.
        assert_eq!(subjects, ["refactor", "fix login (T-8)"]);
        assert_eq!(commits_mentioning(root, "T-8", 1).unwrap().len(), 1);
    }
}

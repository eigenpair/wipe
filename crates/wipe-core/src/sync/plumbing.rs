//! Git plumbing for the wipe branch.
//!
//! The wipe branch is never checked out: its commits are read with `ls-tree` /
//! `cat-file` and written with `hash-object`, a throwaway index, `write-tree` and
//! `commit-tree`, so the user's checkout, index and HEAD are never touched.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::error::{Error, Result};

/// A blob in a tree listing: its object id, keyed by path.
pub type Index = BTreeMap<String, String>;

/// Runs git in one directory.
#[derive(Debug, Clone)]
pub struct Git {
    dir: PathBuf,
}

/// A finished git run.
pub struct Output {
    pub ok: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Git {
    pub fn new(dir: &Path) -> Self {
        Git {
            dir: crate::vcs::plain(dir),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Run git with optional stdin and extra environment; never fails on a
    /// non-zero exit (see [`Output::ok`]), only when git can't be started.
    pub fn exec(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
        stdin: Option<&[u8]>,
    ) -> Result<Output> {
        let mut cmd = crate::vcs::command("git");
        cmd.arg("-C").arg(&self.dir).args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::msg(format!("failed to run git: {e}")))?;
        // Feed stdin from a thread so a large input can't deadlock against a
        // full stdout pipe.
        let feeder = match (stdin, child.stdin.take()) {
            (Some(bytes), Some(mut pipe)) => {
                let bytes = bytes.to_vec();
                Some(std::thread::spawn(move || {
                    let _ = pipe.write_all(&bytes);
                }))
            }
            _ => None,
        };
        let out = child
            .wait_with_output()
            .map_err(|e| Error::msg(format!("failed to run git: {e}")))?;
        if let Some(f) = feeder {
            let _ = f.join();
        }
        Ok(Output {
            ok: out.status.success(),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }

    /// Run git and return trimmed stdout, or an error carrying stderr.
    pub fn run(&self, args: &[&str]) -> Result<String> {
        self.run_with(args, &[], None)
    }

    pub fn run_with(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
        stdin: Option<&[u8]>,
    ) -> Result<String> {
        let o = self.exec(args, env, stdin)?;
        if o.ok {
            Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
        } else {
            Err(Error::msg(format!(
                "git {}: {}",
                args.first().unwrap_or(&""),
                o.stderr
            )))
        }
    }

    /// Like [`Git::run`], but `None` on any failure.
    pub fn try_run(&self, args: &[&str]) -> Option<String> {
        self.run(args).ok()
    }

    /// The commit a ref or revision names, if it exists.
    pub fn rev(&self, name: &str) -> Option<String> {
        let spec = format!("{name}^{{commit}}");
        self.try_run(&["rev-parse", "--verify", "--quiet", &spec])
            .filter(|s| !s.is_empty())
    }

    /// Whether `a` is an ancestor of (or equal to) `b`.
    pub fn is_ancestor(&self, a: &str, b: &str) -> bool {
        self.exec(&["merge-base", "--is-ancestor", a, b], &[], None)
            .map(|o| o.ok)
            .unwrap_or(false)
    }

    pub fn merge_base(&self, a: &str, b: &str) -> Option<String> {
        self.try_run(&["merge-base", a, b])
            .filter(|s| !s.is_empty())
    }

    /// Number of commits in `a..b`.
    pub fn count(&self, a: &str, b: &str) -> usize {
        self.try_run(&["rev-list", "--count", &format!("{a}..{b}")])
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    /// Commits only in `a`, and only in `b` (`a...b`).
    pub fn left_right(&self, a: &str, b: &str) -> (usize, usize) {
        let out = self
            .try_run(&["rev-list", "--left-right", "--count", &format!("{a}...{b}")])
            .unwrap_or_default();
        let mut n = out.split_whitespace().map(|s| s.parse().unwrap_or(0));
        (n.next().unwrap_or(0), n.next().unwrap_or(0))
    }

    /// Number of commits reachable from `a`.
    pub fn count_all(&self, a: &str) -> usize {
        self.try_run(&["rev-list", "--count", a])
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    /// Every blob under `dir` (repo-relative, no trailing slash) at `rev`, keyed by
    /// path relative to `dir`.
    pub fn ls_tree(&self, rev: &str, dir: &str) -> Result<Index> {
        let o = self.exec(
            &["ls-tree", "-r", "-z", "--full-tree", rev, "--", dir],
            &[],
            None,
        )?;
        if !o.ok {
            return Err(Error::msg(format!("git ls-tree: {}", o.stderr)));
        }
        let prefix = format!("{dir}/");
        let mut out = Index::new();
        for rec in o.stdout.split(|b| *b == 0) {
            let rec = String::from_utf8_lossy(rec);
            // "<mode> <type> <sha>\t<path>"
            let Some((meta, path)) = rec.split_once('\t') else {
                continue;
            };
            let mut cols = meta.split_whitespace();
            let (_mode, kind, sha) = (cols.next(), cols.next(), cols.next());
            if kind != Some("blob") {
                continue;
            }
            if let (Some(sha), Some(rel)) = (sha, path.strip_prefix(&prefix)) {
                out.insert(rel.to_string(), sha.to_string());
            }
        }
        Ok(out)
    }

    /// Whether `rev` has any blob under `dir`.
    pub fn has_path(&self, rev: &str, path: &str) -> bool {
        self.try_run(&["ls-tree", "--name-only", rev, "--", path])
            .is_some_and(|s| !s.is_empty())
    }

    /// The contents of blobs, in order, via one `cat-file --batch`.
    pub fn cat_blobs(&self, shas: &[&str]) -> Result<Vec<Vec<u8>>> {
        if shas.is_empty() {
            return Ok(Vec::new());
        }
        let input = shas.join("\n") + "\n";
        let o = self.exec(&["cat-file", "--batch"], &[], Some(input.as_bytes()))?;
        if !o.ok {
            return Err(Error::msg(format!("git cat-file: {}", o.stderr)));
        }
        let mut out = Vec::with_capacity(shas.len());
        let mut pos = 0;
        let buf = &o.stdout;
        for sha in shas {
            let nl = buf[pos..]
                .iter()
                .position(|b| *b == b'\n')
                .ok_or_else(|| Error::msg("git cat-file: truncated output"))?;
            let header = String::from_utf8_lossy(&buf[pos..pos + nl]).to_string();
            pos += nl + 1;
            let size: usize = header
                .rsplit(' ')
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| Error::msg(format!("git cat-file: missing object {sha}")))?;
            out.push(buf[pos..pos + size].to_vec());
            pos += size + 1;
        }
        Ok(out)
    }

    /// Write bytes as a blob; returns its id.
    pub fn hash_bytes(&self, bytes: &[u8]) -> Result<String> {
        self.run_with(
            &["hash-object", "-w", "--no-filters", "--stdin"],
            &[],
            Some(bytes),
        )
    }

    /// Write files on disk as blobs (byte-exact, no filters); returns their ids.
    pub fn hash_files(&self, paths: &[PathBuf]) -> Result<Vec<String>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let input: String = paths
            .iter()
            .map(|p| format!("{}\n", crate::vcs::plain(p).display()))
            .collect();
        let out = self.run_with(
            &["hash-object", "-w", "--no-filters", "--stdin-paths"],
            &[],
            Some(input.as_bytes()),
        )?;
        let ids: Vec<String> = out.lines().map(str::to_string).collect();
        if ids.len() != paths.len() {
            return Err(Error::msg("git hash-object: unexpected output"));
        }
        Ok(ids)
    }

    /// Build a tree: start from `parent`'s tree (or empty), drop everything under
    /// `dir` when `replace` is set, then add `entries` (path relative to `dir`,
    /// blob id; `None` removes the path).
    pub fn build_tree(
        &self,
        parent: Option<&str>,
        dir: &str,
        replace: bool,
        entries: &[(String, Option<String>)],
    ) -> Result<String> {
        let tmp = tempfile::Builder::new()
            .prefix("wipe-index-")
            .tempdir()
            .map_err(Error::Io)?;
        let index = tmp.path().join("index");
        let index = index.to_string_lossy().to_string();
        let env = [("GIT_INDEX_FILE", index.as_str())];
        match parent {
            Some(p) => self.run_with(&["read-tree", p], &env, None)?,
            None => self.run_with(&["read-tree", "--empty"], &env, None)?,
        };
        if replace {
            self.run_with(
                &["rm", "-r", "--cached", "-q", "--ignore-unmatch", "--", dir],
                &env,
                None,
            )?;
        }
        let mut info = String::new();
        for (rel, sha) in entries {
            let path = format!("{dir}/{rel}");
            match sha {
                Some(sha) => info.push_str(&format!("100644 {sha}\t{path}\n")),
                // Mode 0 removes the entry.
                None => info.push_str(&format!(
                    "0 0000000000000000000000000000000000000000\t{path}\n"
                )),
            }
        }
        if !info.is_empty() {
            self.run_with(
                &["update-index", "--index-info"],
                &env,
                Some(info.as_bytes()),
            )?;
        }
        self.run_with(&["write-tree"], &env, None)
    }

    /// Create a commit object attributed (author and committer) to `who`.
    pub fn commit_tree(
        &self,
        tree: &str,
        parents: &[&str],
        message: &str,
        who: (&str, &str),
    ) -> Result<String> {
        let mut args = vec!["commit-tree", tree];
        for p in parents {
            args.push("-p");
            args.push(p);
        }
        let env = [
            ("GIT_AUTHOR_NAME", who.0),
            ("GIT_AUTHOR_EMAIL", who.1),
            ("GIT_COMMITTER_NAME", who.0),
            ("GIT_COMMITTER_EMAIL", who.1),
        ];
        self.run_with(&args, &env, Some(message.as_bytes()))
    }

    /// Point `refname` at `new`, only if it currently points at `old` (`None`:
    /// only if it doesn't exist yet).
    pub fn update_ref(&self, refname: &str, new: &str, old: Option<&str>) -> Result<()> {
        let zero = "0000000000000000000000000000000000000000";
        self.run(&[
            "update-ref",
            "-m",
            "wipe",
            refname,
            new,
            old.unwrap_or(zero),
        ])
        .map(|_| ())
    }

    /// The remote the wipe branch syncs with: `git config wipe.remote`, else
    /// `origin`, else the only remote.
    pub fn remote(&self) -> Option<String> {
        if let Some(r) = self
            .try_run(&["config", "wipe.remote"])
            .filter(|s| !s.is_empty())
        {
            return Some(r);
        }
        let remotes: Vec<String> = self
            .try_run(&["remote"])?
            .lines()
            .map(str::to_string)
            .collect();
        if remotes.iter().any(|r| r == "origin") {
            return Some("origin".into());
        }
        remotes.into_iter().next()
    }

    /// Fetch `branch` from `remote` into its remote-tracking ref. `Ok(false)` when
    /// the remote doesn't have the branch (yet).
    pub fn fetch(&self, remote: &str, branch: &str, timeout: Duration) -> Result<bool> {
        let spec = format!("+refs/heads/{branch}:refs/remotes/{remote}/{branch}");
        let o = self.exec_timed(
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                "--no-write-fetch-head",
                remote,
                &spec,
            ],
            timeout,
        )?;
        if o.ok {
            return Ok(true);
        }
        if o.stderr.contains("couldn't find remote ref") {
            // Gone from the remote: drop the stale tracking ref too.
            let _ = self.run(&[
                "update-ref",
                "-d",
                &format!("refs/remotes/{remote}/{branch}"),
            ]);
            return Ok(false);
        }
        Err(Error::msg(format!(
            "fetching {remote}/{branch} failed: {}",
            o.stderr
        )))
    }

    /// Push `commit` to `remote`'s `branch` (fast-forward only).
    pub fn push(
        &self,
        remote: &str,
        commit: &str,
        branch: &str,
        timeout: Duration,
    ) -> Result<Push> {
        let spec = format!("{commit}:refs/heads/{branch}");
        let o = self.exec_timed(&["push", "--quiet", "--porcelain", remote, &spec], timeout)?;
        if o.ok {
            // Keep the tracking ref in step without another fetch.
            let _ = self.run(&[
                "update-ref",
                &format!("refs/remotes/{remote}/{branch}"),
                commit,
            ]);
            return Ok(Push::Done);
        }
        let all = format!("{}\n{}", String::from_utf8_lossy(&o.stdout), o.stderr);
        if all.contains("[rejected]")
            || all.contains("non-fast-forward")
            || all.contains("fetch first")
        {
            return Ok(Push::Rejected);
        }
        Err(Error::msg(format!(
            "pushing to {remote}/{branch} failed: {}",
            o.stderr
        )))
    }

    /// [`Git::exec`] for network commands: no credential prompts on a terminal
    /// that can't answer, and a hard timeout.
    fn exec_timed(&self, args: &[&str], timeout: Duration) -> Result<Output> {
        let git = self.clone();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let _ = tx.send(git.exec(&refs, &[("GIT_TERMINAL_PROMPT", "0")], None));
        });
        rx.recv_timeout(timeout)
            .map_err(|_| Error::msg(format!("git timed out after {}s", timeout.as_secs())))?
    }
}

/// The result of a push.
#[derive(Debug, PartialEq, Eq)]
pub enum Push {
    Done,
    /// The remote moved on: fetch, merge, and try again.
    Rejected,
}

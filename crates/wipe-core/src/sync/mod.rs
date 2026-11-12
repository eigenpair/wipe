//! The wipe branch: the board's single source of truth.
//!
//! In a git repo, the board lives on a dedicated orphan branch (`wipe`) that
//! holds nothing but board directories (`.wipe/`, or `apps/x/.wipe/` for a board
//! nested in a monorepo). Code branches don't track `.wipe/` at all - it is
//! gitignored there - so the `.wipe/` directory in a checkout is a *working
//! copy* of the board, shared by every worktree of the repo.
//!
//! Three things describe a board's state:
//!
//! * the **base** - the wipe-branch commit the working copy was last synced to
//!   (kept in `.wipe/.cache/sync.json`);
//! * the **working copy** - `.wipe/` on disk; how it differs from the base is
//!   the *local changes*;
//! * the **upstream** - the branch on the remote (`origin/wipe`; the local
//!   `wipe` ref when there's no remote); how it moved past the base is the
//!   *incoming changes*.
//!
//! [`commit`] records local changes as one commit on the branch and publishes
//! it (merging incoming changes first); [`sync`] applies incoming changes to the
//! working copy, keeping local ones on top. Both merge per field (see
//! [`merge`]) and stop at a real conflict until it is resolved. Nothing here
//! commits on its own: committing is always an explicit step.
//!
//! The branch is never checked out - everything goes through git plumbing
//! ([`plumbing`]) - so HEAD, the index and the code checkout stay untouched.

pub mod diff;
pub mod merge;
pub mod plumbing;
pub mod take;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::git::CommitInfo;
use crate::Store;
pub use diff::Change;
pub use merge::{Conflict, Resolutions, Side};
use plumbing::{Git, Index, Push};
pub use take::Entity;

/// Board files by path relative to `.wipe/` (forward slashes).
pub type Files = BTreeMap<String, Vec<u8>>;

/// The branch that holds the board.
pub const BRANCH: &str = "wipe";
const LOCAL_REF: &str = "refs/heads/wipe";
/// How long a network call (fetch, push) may take.
const NET_TIMEOUT: Duration = Duration::from_secs(30);
/// How stale the last fetch may get before a command refreshes it in the
/// background.
const FETCH_EVERY: i64 = 120;
const STATE_FILE: &str = "sync.json";
const HASH_CACHE: &str = "sync-hashes.json";
// Versioned: a cache written by an older wipe (missing fields) is never read.
const STATUS_CACHE: &str = "sync-status-v2.json";

/// How a board relates to the wipe branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// On the wipe branch: the normal state.
    Branch,
    /// In a git repo, but the board isn't on the wipe branch yet (a board from
    /// before 1.0, tracked with the code): `wipe migrate` moves it.
    Unmigrated,
    /// The board is on the wipe branch, but the checked-out code branch still
    /// tracks `.wipe/` (it predates the move): `wipe migrate` untracks it here.
    Tracked,
    /// Not in a git repository: the board is plain files.
    None,
}

/// Where a board is, relative to the wipe branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub mode: Mode,
    pub branch: String,
    /// The remote the branch is published to.
    pub remote: Option<String>,
    /// The board directory inside the repo (`.wipe`, `apps/x/.wipe`).
    pub board_dir: String,
    /// The commit the working copy is based on.
    pub base: Option<String>,
    /// The branch tip to sync with.
    pub upstream: Option<String>,
    /// Changes in the working copy that aren't committed.
    pub local: Vec<Change>,
    /// Changes on the branch that aren't in the working copy.
    pub incoming: Vec<Change>,
    /// The commits those incoming changes came in.
    pub commits: Vec<CommitInfo>,
    /// Local branch commits the remote doesn't have yet.
    pub unpublished: usize,
    /// The upstream was rewritten (force-pushed) since the working copy's base.
    pub rewritten: bool,
    /// When the remote was last fetched.
    pub fetched: Option<DateTime<Utc>>,
}

impl Status {
    fn bare(mode: Mode) -> Self {
        Status {
            mode,
            branch: BRANCH.to_string(),
            remote: None,
            board_dir: ".wipe".into(),
            base: None,
            upstream: None,
            local: Vec::new(),
            incoming: Vec::new(),
            commits: Vec::new(),
            unpublished: 0,
            rewritten: false,
            fetched: None,
        }
    }
}

/// What a board outside git is, in one line: plain files, with none of the
/// wipe branch's features. wipe supports git only - a workspace of any other
/// version control is the same as a plain folder.
pub const PLAIN_FILES: &str = "not a git repository - the board is plain files in .wipe/, without commit, sync or history (those need the wipe branch, in a git repo)";

/// The error for a branch command on a board outside git.
fn no_branch(_store: &Store) -> String {
    PLAIN_FILES.to_string()
}

/// Per-board sync bookkeeping, in the gitignored cache.
#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fetched: Option<DateTime<Utc>>,
    /// Incoming changes applied one by one (see [`take`]): the base's files as
    /// they would be had the base moved for just those changes - path to blob id,
    /// `None` for removed. Lets the next merge know they were taken, and keeps
    /// them out of both the local and the incoming lists. Cleared whenever the
    /// base moves.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    overlay: BTreeMap<String, Option<String>>,
}

impl State {
    /// The working copy is now based on `commit`.
    fn rebase(&mut self, commit: String) {
        self.base = Some(commit);
        self.overlay.clear();
    }
}

fn load_state(store: &Store) -> State {
    std::fs::read(store.cache_dir().join(STATE_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_state(store: &Store, s: &State) -> Result<()> {
    crate::store::write_json_atomic(&store.cache_dir().join(STATE_FILE), s)
}

/// A board in a git repo, with what's needed to talk to its branch.
struct Repo {
    git: Git,
    /// `.wipe` or `apps/x/.wipe`.
    dir: String,
    remote: Option<String>,
    /// This worktree's git dir and the repo's common one (they differ in a
    /// linked worktree).
    git_dir: PathBuf,
    common_dir: PathBuf,
}

impl Repo {
    fn open(store: &Store) -> Option<Repo> {
        let git = Git::new(store.root());
        let out = git.try_run(&[
            "rev-parse",
            "--is-inside-work-tree",
            "--show-prefix",
            "--absolute-git-dir",
            "--git-common-dir",
        ])?;
        let mut lines = out.lines();
        // Not in a work tree: inside .git, or a bare repo.
        if lines.next() != Some("true") {
            return None;
        }
        let prefix = lines.next()?.to_string();
        let git_dir = PathBuf::from(lines.next()?);
        let common = PathBuf::from(lines.next()?);
        let common_dir = if common.is_absolute() {
            common
        } else {
            git.dir().join(common)
        };
        let remote = git.remote();
        Some(Repo {
            dir: format!("{prefix}{}", crate::WIPE_DIR),
            remote,
            git,
            git_dir,
            common_dir,
        })
    }

    /// The local and remote-tracking tips of the branch, in one call.
    fn tips(&self) -> (Option<String>, Option<String>) {
        let mut refs = vec![LOCAL_REF.to_string()];
        refs.extend(self.remote_ref());
        let mut args = vec!["for-each-ref", "--format=%(refname) %(objectname)"];
        args.extend(refs.iter().map(String::as_str));
        let out = self.git.try_run(&args).unwrap_or_default();
        let find = |name: &str| {
            out.lines()
                .find_map(|l| l.strip_prefix(name)?.strip_prefix(' ').map(str::to_string))
        };
        (find(LOCAL_REF), self.remote_ref().and_then(|r| find(&r)))
    }

    fn remote_ref(&self) -> Option<String> {
        self.remote
            .as_ref()
            .map(|r| format!("refs/remotes/{r}/{BRANCH}"))
    }

    fn local(&self) -> Option<String> {
        self.git.rev(LOCAL_REF)
    }

    fn remote_tip(&self) -> Option<String> {
        self.remote_ref().and_then(|r| self.git.rev(&r))
    }

    /// The tip to sync with: the remote's branch, else the local one.
    fn upstream(&self) -> Option<String> {
        self.remote_tip().or_else(|| self.local())
    }

    /// Whether `rev` holds this board.
    fn has_board(&self, rev: &str) -> bool {
        self.git.has_path(rev, &format!("{}/board.json", self.dir))
    }

    /// Whether the checked-out code branch tracks the board (in the index).
    fn tracked(&self) -> bool {
        self.git
            .try_run(&["ls-files", "--", &format!("{}/board.json", self.dir)])
            .is_some_and(|s| !s.is_empty())
    }

    fn mode(&self) -> Mode {
        match self.upstream().filter(|u| self.has_board(u)) {
            None => Mode::Unmigrated,
            Some(_) if self.tracked() => Mode::Tracked,
            Some(_) => Mode::Branch,
        }
    }

    fn fetch(&self) -> Result<()> {
        if let Some(r) = &self.remote {
            self.git.fetch(r, BRANCH, NET_TIMEOUT)?;
        }
        Ok(())
    }

    /// The base's listing, with the changes taken one by one on top when `base`
    /// is the recorded base.
    fn base_tree(&self, state: &State, base: Option<&str>) -> Result<Index> {
        let mut index = self.tree(base)?;
        if base.is_some() && base == state.base.as_deref() {
            for (path, sha) in &state.overlay {
                match sha {
                    Some(s) => index.insert(path.clone(), s.clone()),
                    None => index.remove(path),
                };
            }
        }
        Ok(index)
    }

    fn tree(&self, rev: Option<&str>) -> Result<Index> {
        match rev {
            Some(r) => self.git.ls_tree(r, &self.dir),
            None => Ok(Index::new()),
        }
    }

    /// The contents of `paths` (those present) from a tree listing.
    fn read(&self, index: &Index, paths: &BTreeSet<String>) -> Result<Files> {
        let wanted: Vec<(&String, &String)> = paths
            .iter()
            .filter_map(|p| index.get(p).map(|sha| (p, sha)))
            .collect();
        let shas: Vec<&str> = wanted.iter().map(|(_, s)| s.as_str()).collect();
        let blobs = self.git.cat_blobs(&shas)?;
        Ok(wanted
            .into_iter()
            .zip(blobs)
            .map(|((p, _), b)| (p.clone(), b))
            .collect())
    }
}

// --- the working copy -------------------------------------------------------

/// Every board file in the working copy (the cache and temp files aside), by
/// path relative to `.wipe/`.
fn work_paths(store: &Store) -> Vec<String> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if (rel.is_empty() && name == ".cache") || name.starts_with(".tmp") {
                continue;
            }
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path(), &path, out),
                Ok(t) if t.is_file() => out.push(path),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&store.wipe_dir(), "", &mut out);
    out.sort();
    out
}

fn disk_path(store: &Store, rel: &str) -> PathBuf {
    rel.split('/').fold(store.wipe_dir(), |p, seg| p.join(seg))
}

/// The working copy as a tree listing: each file's blob id, from a cache keyed
/// by size and modification time so unchanged files (large media included)
/// aren't re-read on every command.
fn work_index(store: &Store) -> Index {
    type Cache = BTreeMap<String, (u64, u64, String)>;
    let cache_path = store.cache_dir().join(HASH_CACHE);
    let old: Cache = std::fs::read(&cache_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut new = Cache::new();
    let mut index = Index::new();
    for rel in work_paths(store) {
        let path = disk_path(store, &rel);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let sha = match old.get(&rel) {
            Some((len, m, sha)) if *len == meta.len() && *m == mtime && mtime != 0 => sha.clone(),
            _ => match std::fs::read(&path) {
                Ok(bytes) => crate::git::blob_hash(&bytes),
                Err(_) => continue,
            },
        };
        new.insert(rel.clone(), (meta.len(), mtime, sha.clone()));
        index.insert(rel, sha);
    }
    if new != old {
        let _ = crate::store::write_json_atomic(&cache_path, &new);
    }
    index
}

fn read_work(store: &Store, paths: &BTreeSet<String>) -> Files {
    paths
        .iter()
        .filter_map(|p| {
            std::fs::read(disk_path(store, p))
                .ok()
                .map(|b| (p.clone(), b))
        })
        .collect()
}

/// Write merged files over the working copy (only those that differ from it).
fn write_work(
    store: &Store,
    merged: &BTreeMap<String, Option<Vec<u8>>>,
    ours: &Files,
) -> Result<()> {
    // board.json last, so a reader never sees cards for tickets not yet written.
    let mut paths: Vec<&String> = merged.keys().filter(|p| *p != "board.json").collect();
    paths.extend(merged.keys().filter(|p| *p == "board.json"));
    for p in paths {
        let want = &merged[p];
        if want.as_ref() == ours.get(p) {
            continue;
        }
        let path = disk_path(store, p);
        match want {
            Some(bytes) => crate::store::write_bytes_atomic(&path, bytes)?,
            None => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            },
        }
    }
    Ok(())
}

/// Paths whose blob differs between two listings.
fn changed(a: &Index, b: &Index) -> BTreeSet<String> {
    a.keys()
        .chain(b.keys())
        .filter(|p| a.get(*p) != b.get(*p))
        .cloned()
        .collect()
}

fn with_board(mut paths: BTreeSet<String>) -> BTreeSet<String> {
    paths.insert("board.json".into());
    paths
}

// --- status -----------------------------------------------------------------

/// A file's size and modification time, for fingerprints (`-` when absent).
fn stamp(p: &Path) -> String {
    match std::fs::metadata(p) {
        Ok(m) => {
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("{}:{t}", m.len())
        }
        Err(_) => "-".into(),
    }
}

/// The last computed status and what it was computed from.
#[derive(Serialize, Deserialize)]
struct StatusCache {
    key: String,
    /// The repo's git dirs, so the next fingerprint needs no git call.
    git_dir: Option<PathBuf>,
    common_dir: Option<PathBuf>,
    remote: Option<String>,
    status: Status,
}

fn status_key(
    store: &Store,
    git: Option<(&Path, &Path, Option<&str>)>,
    state: &State,
    work: &Index,
) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    for (p, sha) in work {
        h.update(p.as_bytes());
        h.update(sha.as_bytes());
    }
    let refs = match git {
        Some((git_dir, common, remote)) => {
            let mut files = vec![
                common.join("refs").join("heads").join(BRANCH),
                common.join("packed-refs"),
                common.join("config"),
                git_dir.join("index"),
                git_dir.join("HEAD"),
            ];
            if let Some(r) = remote {
                files.push(common.join("refs").join("remotes").join(r).join(BRANCH));
            }
            files.iter().map(|p| stamp(p)).collect::<Vec<_>>().join(",")
        }
        // Not in git: until a repo appears around the board.
        None => stamp(store.root()),
    };
    for (p, sha) in &state.overlay {
        h.update(p.as_bytes());
        h.update(sha.as_deref().unwrap_or("-").as_bytes());
    }
    format!(
        "{}|{refs}|{}|{:x}",
        env!("CARGO_PKG_VERSION"),
        state.base.as_deref().unwrap_or(""),
        h.finalize()
    )
}

/// The board's state against the wipe branch, as of the last fetch (no network).
///
/// Cached: when neither the working copy nor git's refs, index or config
/// changed since the last call, it is answered without running git at all - it
/// runs after every command.
pub fn status(store: &Store) -> Result<Status> {
    let cache_path = store.cache_dir().join(STATUS_CACHE);
    let state = load_state(store);
    let work = work_index(store);
    let cached: Option<StatusCache> = std::fs::read(&cache_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    if let Some(c) = &cached {
        let git = match (&c.git_dir, &c.common_dir) {
            (Some(g), Some(cd)) => Some((g.as_path(), cd.as_path(), c.remote.as_deref())),
            _ => None,
        };
        if status_key(store, git, &state, &work) == c.key {
            let mut st = c.status.clone();
            st.fetched = state.fetched;
            return Ok(st);
        }
    }
    let repo = Repo::open(store);
    let key = status_key(
        store,
        repo.as_ref().map(|r| {
            (
                r.git_dir.as_path(),
                r.common_dir.as_path(),
                r.remote.as_deref(),
            )
        }),
        &state,
        &work,
    );
    let st = match &repo {
        Some(repo) => compute_status(store, repo, &state, &work)?,
        None => Status::bare(Mode::None),
    };
    let _ = crate::store::write_json_atomic(
        &cache_path,
        &StatusCache {
            key,
            git_dir: repo.as_ref().map(|r| r.git_dir.clone()),
            common_dir: repo.as_ref().map(|r| r.common_dir.clone()),
            remote: repo.as_ref().and_then(|r| r.remote.clone()),
            status: st.clone(),
        },
    );
    Ok(st)
}

fn compute_status(store: &Store, repo: &Repo, state: &State, work: &Index) -> Result<Status> {
    let mut st = Status::bare(Mode::Unmigrated);
    st.remote = repo.remote.clone();
    st.board_dir = repo.dir.clone();
    st.fetched = state.fetched;
    let (local, remote_tip) = repo.tips();
    let Some(upstream) = remote_tip.clone().or_else(|| local.clone()) else {
        return Ok(st);
    };
    let base = state.base.clone().or_else(|| local.clone());
    let base_index = repo.base_tree(state, base.as_deref())?;
    let up_index = if base.as_deref() == Some(upstream.as_str()) {
        base_index.clone()
    } else {
        repo.tree(Some(&upstream))?
    };
    if !up_index.contains_key("board.json") {
        return Ok(st);
    }
    if repo.tracked() {
        st.mode = Mode::Tracked;
        return Ok(st);
    }
    st.mode = Mode::Branch;
    st.upstream = Some(upstream.clone());
    st.base = base.clone();

    // Local: base -> working copy.
    let paths = with_board(changed(&base_index, work));
    if paths.len() > 1 || base_index.get("board.json") != work.get("board.json") {
        let before = repo.read(&base_index, &paths)?;
        let after = read_work(store, &paths);
        st.local = diff::diff(&before, &after);
    }

    // Incoming: base -> upstream, when the upstream has commits the base lacks.
    if let Some(b) = base.as_ref().filter(|b| **b != upstream) {
        let (only_base, only_up) = repo.git.left_right(b, &upstream);
        if only_up > 0 {
            st.rewritten = only_base > 0;
            let paths = with_board(changed(&base_index, &up_index));
            let before = repo.read(&base_index, &paths)?;
            let after = repo.read(&up_index, &paths)?;
            st.incoming = diff::diff(&before, &after);
            let from = if only_base > 0 {
                repo.git.merge_base(b, &upstream)
            } else {
                Some(b.clone())
            };
            let range = match from {
                Some(f) => format!("{f}..{upstream}"),
                None => upstream.clone(),
            };
            st.commits = crate::git::log_range(repo.git.dir(), &range, 50).unwrap_or_default();
        }
    }

    // Unpublished: local branch commits the remote lacks.
    if let (Some(_), Some(l)) = (&repo.remote, &local) {
        st.unpublished = match &remote_tip {
            Some(r) if r == l => 0,
            Some(r) => repo.git.left_right(r, l).1,
            None => repo.git.count_all(l),
        };
    }
    // Diverging because of our own unpublished commits isn't a rewrite.
    st.rewritten &= st.unpublished == 0;
    Ok(st)
}

/// Fetch the remote branch now (when there is a remote), then report.
pub fn refresh(store: &Store) -> Result<Status> {
    if let Some(repo) = Repo::open(store) {
        repo.fetch()?;
        let mut state = load_state(store);
        state.fetched = Some(Utc::now());
        let _ = save_state(store, &state);
    }
    status(store)
}

/// Start a background fetch when the last one is older than a couple of
/// minutes, so the next command sees fresh incoming changes without this one
/// waiting on the network. Best-effort and silent.
pub fn fetch_in_background(store: &Store) {
    let mut state = load_state(store);
    let fresh = state
        .fetched
        .is_some_and(|t| (Utc::now() - t).num_seconds() < FETCH_EVERY);
    if fresh {
        return;
    }
    let Some(repo) = Repo::open(store) else {
        return;
    };
    let Some(remote) = repo.remote.clone() else {
        return;
    };
    if repo.mode() != Mode::Branch {
        return;
    }
    state.fetched = Some(Utc::now());
    if save_state(store, &state).is_err() {
        return;
    }
    let spec = format!("+refs/heads/{BRANCH}:refs/remotes/{remote}/{BRANCH}");
    let _ = crate::vcs::command("git")
        .arg("-C")
        .arg(repo.git.dir())
        .args([
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            &remote,
            &spec,
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

// --- merging ----------------------------------------------------------------

struct Prepared {
    merged: merge::Merged,
    /// The three sides the merge read.
    base: Files,
    theirs: Files,
    ours: Files,
    /// What the merge brings in, for reporting.
    incoming: Vec<Change>,
}

/// Merge `upstream` into the working copy, with `base` the commit the working
/// copy started from.
fn prepare(
    store: &Store,
    repo: &Repo,
    state: &State,
    base: Option<&str>,
    upstream: &str,
    res: &Resolutions,
) -> Result<Prepared> {
    // Your working copy's changes are what it holds beyond the board it was
    // last synced to - so that board is the merge base, even when the
    // upstream history was rewritten since (force-pushed, recreated): its
    // differences from that board then read as their changes, and yours
    // never clash with content that didn't change. Only when that commit is
    // gone is the common ancestor (or nothing) used.
    let merge_base = match base {
        Some(b) if repo.git.rev(b).is_some() => Some(b.to_string()),
        Some(b) => repo.git.merge_base(b, upstream),
        None => None,
    };
    let base_index = repo.base_tree(state, merge_base.as_deref())?;
    let up_index = repo.tree(Some(upstream))?;
    let work = work_index(store);
    let mut paths = changed(&base_index, &up_index);
    paths.extend(changed(&base_index, &work));
    let paths = with_board(paths);
    let b = repo.read(&base_index, &paths)?;
    let t = repo.read(&up_index, &paths)?;
    let o = read_work(store, &paths);
    check_version(&t)?;
    let incoming = {
        let shown_base = repo.base_tree(state, base)?;
        let shown = with_board(changed(&shown_base, &up_index));
        diff::diff(
            &repo.read(&shown_base, &shown)?,
            &repo.read(&up_index, &shown)?,
        )
    };
    Ok(Prepared {
        merged: merge::merge(&b, &o, &t, res),
        base: b,
        theirs: t,
        ours: o,
        incoming,
    })
}

/// Refuse a board written by a newer wipe (its schema may not round-trip).
fn check_version(files: &Files) -> Result<()> {
    #[derive(Deserialize)]
    struct V {
        version: u32,
    }
    for (p, bytes) in files {
        if !p.ends_with(".json") {
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<V>(bytes) {
            if v.version > crate::model::FORMAT_VERSION {
                return Err(Error::msg(format!(
                    "the wipe branch was written by a newer wipe (board format {}, this wipe reads {}) - run `wipe update` first",
                    v.version,
                    crate::model::FORMAT_VERSION
                )));
            }
        }
    }
    Ok(())
}

/// The board as it would be after applying incoming changes - nothing written.
#[derive(Debug)]
pub struct Preview {
    pub board: crate::model::Board,
    /// Each list's tickets, in order.
    pub lists: Vec<crate::ops::ListView>,
    /// What comes in.
    pub incoming: Vec<Change>,
    /// Conflicts with the resolutions given (their side shown until resolved).
    pub conflicts: Vec<Conflict>,
    pub renumbered: Vec<(String, String)>,
}

/// Preview applying the incoming changes (as last fetched) to the working copy.
pub fn preview(store: &Store, res: &Resolutions) -> Result<Preview> {
    let repo = Repo::open(store).ok_or_else(|| Error::msg(no_branch(store)))?;
    require_branch(&repo)?;
    let state = load_state(store);
    let base = state.base.clone().or_else(|| repo.local());
    let upstream = repo
        .upstream()
        .ok_or_else(|| Error::msg("the wipe branch is missing"))?;
    let behind = base.as_deref() != Some(upstream.as_str())
        && !base
            .as_ref()
            .is_some_and(|b| repo.git.is_ancestor(&upstream, b));
    let (overlay, incoming, conflicts, renumbered) = if behind {
        let p = prepare(store, &repo, &state, base.as_deref(), &upstream, res)?;
        (
            p.merged.files,
            p.incoming,
            p.merged.conflicts,
            p.merged.renumbered,
        )
    } else {
        (BTreeMap::new(), Vec::new(), Vec::new(), Vec::new())
    };
    let read = |rel: &str| -> Option<Vec<u8>> {
        match overlay.get(rel) {
            Some(v) => v.clone(),
            None => std::fs::read(disk_path(store, rel)).ok(),
        }
    };
    let board: crate::model::Board = serde_json::from_slice(
        &read("board.json").ok_or_else(|| Error::msg("the merged board has no board.json"))?,
    )?;
    let lists = board
        .lists
        .iter()
        .map(|l| {
            let tickets = l
                .cards
                .iter()
                .filter_map(|id| serde_json::from_slice(&read(&format!("tickets/{id}.json"))?).ok())
                .collect();
            (l.id.clone(), tickets)
        })
        .collect();
    Ok(Preview {
        board,
        lists,
        incoming,
        conflicts,
        renumbered,
    })
}

/// The result of applying incoming changes.
#[derive(Debug, Serialize)]
pub struct Synced {
    /// What came in.
    pub applied: Vec<Change>,
    /// Our ids renumbered past ids the branch also used.
    pub renumbered: Vec<(String, String)>,
    /// Local commits published to the remote.
    pub published: usize,
    /// Conflicts that need a resolution; when non-empty nothing was changed.
    pub conflicts: Vec<Conflict>,
}

fn require_branch(repo: &Repo) -> Result<()> {
    match repo.mode() {
        Mode::Branch => Ok(()),
        Mode::Unmigrated => Err(Error::msg(
            "this board isn't on the wipe branch yet - run `wipe migrate` once to move it there",
        )),
        Mode::Tracked => Err(Error::msg(
            "the checked-out branch still tracks .wipe/ (it predates the wipe branch) - run `wipe migrate` to untrack it here",
        )),
        Mode::None => Err(Error::msg("not a git repository - the board has no branch to sync with")),
    }
}

/// Apply incoming changes to the working copy, keeping local changes on top,
/// and publish local branch commits the remote lacks. Fetches first when
/// `fetch` is set. Writes nothing while a conflict lacks a resolution. The
/// caller holds the board lock.
pub fn sync(store: &Store, fetch: bool, res: &Resolutions) -> Result<Synced> {
    let repo = Repo::open(store).ok_or_else(|| Error::msg(no_branch(store)))?;
    if fetch {
        repo.fetch()?;
        let mut state = load_state(store);
        state.fetched = Some(Utc::now());
        save_state(store, &state)?;
    }
    require_branch(&repo)?;
    let mut state = load_state(store);
    let base = state.base.clone().or_else(|| repo.local());
    let upstream = repo
        .upstream()
        .ok_or_else(|| Error::msg("the wipe branch is missing"))?;
    let mut out = Synced {
        applied: Vec::new(),
        renumbered: Vec::new(),
        published: 0,
        conflicts: Vec::new(),
    };
    if base.as_deref() != Some(upstream.as_str())
        && !base
            .as_ref()
            .is_some_and(|b| repo.git.is_ancestor(&upstream, b))
    {
        let p = prepare(store, &repo, &state, base.as_deref(), &upstream, res)?;
        if !p.merged.conflicts.is_empty() {
            out.conflicts = p.merged.conflicts;
            return Ok(out);
        }
        write_work(store, &p.merged.files, &p.ours)?;
        state.rebase(upstream.clone());
        save_state(store, &state)?;
        out.applied = p.incoming;
        out.renumbered = p.merged.renumbered;
    }

    // The local ref follows: fast-forward it, or - when it holds commits the
    // remote never got and the remote moved on - let it go (those changes are
    // in the working copy now, as local changes).
    let local = repo.local();
    if let (Some(_), Some(r)) = (&repo.remote, repo.remote_tip()) {
        match &local {
            Some(l) if l == &r => {}
            Some(l) if repo.git.is_ancestor(&r, l) => {
                // Ahead: publish.
                let n = repo.git.count(&r, l);
                if repo.git.push(
                    repo.remote.as_deref().unwrap_or("origin"),
                    l,
                    BRANCH,
                    NET_TIMEOUT,
                )? == Push::Done
                {
                    out.published = n;
                }
            }
            _ => repo.git.update_ref(LOCAL_REF, &r, local.as_deref())?,
        }
    } else if let (Some(remote), Some(l)) = (&repo.remote, &local) {
        // Never published: publish now.
        let n = repo.git.count_all(l);
        if repo.git.push(remote, l, BRANCH, NET_TIMEOUT)? == Push::Done {
            out.published = n;
        }
    }
    Ok(out)
}

/// What [`commit`] did.
#[derive(Debug, Serialize)]
pub struct Committed {
    /// The new commit (`None`: nothing to commit).
    pub commit: Option<String>,
    pub short: Option<String>,
    /// What it recorded.
    pub changes: Vec<Change>,
    /// Incoming changes merged into the working copy first.
    pub merged: Vec<Change>,
    pub renumbered: Vec<(String, String)>,
    /// Where it was published (`None`: no remote).
    pub remote: Option<String>,
    pub message: String,
    /// Conflicts in merging incoming changes; when non-empty nothing was done.
    pub conflicts: Vec<Conflict>,
}

/// Record the working copy's changes as one commit on the wipe branch and
/// publish it. Incoming changes are merged into the working copy first (it is
/// what gets committed); a push that loses a race fetches, merges and retries.
/// `scope` limits the commit to one ticket. The caller holds the board lock.
pub fn commit(
    store: &Store,
    message: Option<&str>,
    scope: Option<&str>,
    author: &str,
    res: &Resolutions,
) -> Result<Committed> {
    let repo = Repo::open(store).ok_or_else(|| Error::msg(no_branch(store)))?;
    require_branch(&repo)?;
    let scope_path = match scope {
        Some(raw) => Some(format!(
            "tickets/{}.json",
            store.resolve_ticket_id(raw).or_else(|e| {
                // A ticket deleted locally can still be committed by its exact id.
                if crate::id::ticket_ref_digits(raw).is_some() && raw.starts_with("T-") {
                    Ok(raw.to_string())
                } else {
                    Err(e)
                }
            })?
        )),
        None => None,
    };
    let (name, email) = crate::git::author_ident(author);
    let mut out = Committed {
        commit: None,
        short: None,
        changes: Vec::new(),
        merged: Vec::new(),
        renumbered: Vec::new(),
        remote: repo.remote.clone(),
        message: String::new(),
        conflicts: Vec::new(),
    };

    for _attempt in 0..4 {
        repo.fetch()?;
        let mut state = load_state(store);
        state.fetched = Some(Utc::now());
        let upstream = repo
            .upstream()
            .ok_or_else(|| Error::msg("the wipe branch is missing"))?;
        let base = state.base.clone().or_else(|| repo.local());

        // Bring the working copy up to the upstream first.
        let behind = base.as_deref() != Some(upstream.as_str())
            && !base
                .as_ref()
                .is_some_and(|b| repo.git.is_ancestor(&upstream, b));
        if behind {
            let p = prepare(store, &repo, &state, base.as_deref(), &upstream, res)?;
            if !p.merged.conflicts.is_empty() {
                out.conflicts = p.merged.conflicts;
                return Ok(out);
            }
            write_work(store, &p.merged.files, &p.ours)?;
            out.merged.extend(p.incoming);
            out.renumbered.extend(p.merged.renumbered);
        }
        // When the local ref is ahead of the remote (unpublished commits), the
        // new commit goes on top of it.
        let parent = match repo.local() {
            Some(l) if repo.git.is_ancestor(&upstream, &l) && behind => upstream.clone(),
            Some(l) if repo.git.is_ancestor(&upstream, &l) => l,
            _ => upstream.clone(),
        };
        state.rebase(parent.clone());
        save_state(store, &state)?;

        let parent_index = repo.tree(Some(&parent))?;
        let work = work_index(store);
        let mut paths = changed(&parent_index, &work);
        if let Some(sp) = &scope_path {
            paths.retain(|p| p == sp);
        }
        if paths.is_empty() {
            if repo.local().as_deref() != Some(parent.as_str()) {
                repo.git
                    .update_ref(LOCAL_REF, &parent, repo.local().as_deref())?;
            }
            return Ok(out);
        }
        let all = with_board(paths.clone());
        let before = repo.read(&parent_index, &all)?;
        let mut after = read_work(store, &all);
        if scope_path.is_some() {
            // Only the scoped file changes; describe it against the old board.
            if let Some(b) = before.get("board.json") {
                after.insert("board.json".into(), b.clone());
            }
        }
        out.changes = diff::diff(&before, &after);

        // Blobs for what changed, then the tree and the commit.
        let present: Vec<&String> = paths.iter().filter(|p| work.contains_key(*p)).collect();
        let shas = repo.git.hash_files(
            &present
                .iter()
                .map(|p| disk_path(store, p))
                .collect::<Vec<_>>(),
        )?;
        let mut ids: BTreeMap<&String, String> = present.into_iter().zip(shas).collect();
        let entries: Vec<(String, Option<String>)> =
            paths.iter().map(|p| (p.clone(), ids.remove(p))).collect();
        let tree = repo
            .git
            .build_tree(Some(&parent), &repo.dir, false, &entries)?;
        let msg = match message.map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => m.to_string(),
            None => default_message(&out.changes),
        };
        let commit = repo
            .git
            .commit_tree(&tree, &[&parent], &msg, (&name, &email))?;

        if let Some(remote) = &repo.remote {
            match repo.git.push(remote, &commit, BRANCH, NET_TIMEOUT)? {
                Push::Done => {}
                // Someone else got there first: merge their commit and retry.
                Push::Rejected => continue,
            }
        }
        repo.git
            .update_ref(LOCAL_REF, &commit, repo.local().as_deref())?;
        let mut state = load_state(store);
        state.rebase(commit.clone());
        save_state(store, &state)?;
        out.short = Some(commit.chars().take(7).collect());
        out.commit = Some(commit);
        out.message = msg;
        return Ok(out);
    }
    Err(Error::msg(
        "the wipe branch kept moving while committing - try again in a moment",
    ))
}

/// A commit message that says what changed: a summary line, then one line per
/// change.
pub fn default_message(changes: &[Change]) -> String {
    let mut msg = format!("board: {}", diff::summary(changes, 60));
    if changes.len() > 1 {
        msg.push_str("\n\n");
        for c in changes {
            msg.push_str(&format!("- {}\n", c.line()));
        }
    }
    msg
}

// --- taking single changes, and the branch's history ------------------------

/// Where [`take`] takes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The incoming changes, merged with yours (as [`preview`] shows them).
    Incoming,
    /// The board as it was after a commit on the branch.
    Commit(String),
}

/// What [`take`] did.
#[derive(Debug, Serialize)]
pub struct Taken {
    pub taken: Vec<Entity>,
    /// Your new tickets / threads renumbered to make room for incoming ones.
    pub renumbered: Vec<(String, String)>,
    /// Every incoming change is now in: the working copy moved up to the branch.
    pub synced: bool,
    /// Conflicts on what was asked for; when non-empty nothing was changed.
    pub conflicts: Vec<Conflict>,
    /// Taken from a past commit, these (ticket / post ids) would lose
    /// uncommitted changes of yours; when non-empty nothing was changed - take
    /// again with `discard_local` to replace them anyway.
    pub discards: Vec<String>,
}

/// The file that is an entity alone (a ticket's or a post's), if it has one.
fn own_file(e: &Entity) -> Option<String> {
    match e.scope.as_str() {
        "ticket" => Some(format!("tickets/{}.json", e.id)),
        "thread" => Some(format!("forum/{}.json", e.id)),
        _ => None,
    }
}

/// Whether conflict `c` concerns `e`.
fn concerns(c: &Conflict, e: &Entity) -> bool {
    match e.scope.as_str() {
        "list" => c.entity == "board" && c.field.contains(&format!("lists[{}]", e.id)),
        "label" => c.entity == "definitions" && c.field.contains(&format!("labels[{}]", e.id)),
        "board" => c.entity == "board" || c.entity == "definitions" || e.fields.contains(&c.entity),
        _ => c.entity == e.id,
    }
}

/// A path's content in a merge result, else (untouched by the merge) the
/// working copy's.
fn merged_or_work(
    merged: &BTreeMap<String, Option<Vec<u8>>>,
    work: &Files,
    path: &str,
) -> Option<Vec<u8>> {
    match merged.get(path) {
        Some(v) => v.clone(),
        None => work.get(path).cloned(),
    }
}

/// Bring single changes into the working copy: each entity as it is in the
/// incoming merge (so it lands with your own changes to it kept), or as it was
/// after a past commit (a revert, which then shows as an uncommitted change).
/// Taking incoming changes this way records them in the base overlay, so they
/// are neither local changes nor still incoming; once nothing is left incoming
/// the working copy moves up to the branch. The caller holds the board lock.
pub fn take(
    store: &Store,
    source: &Source,
    entities: &[Entity],
    res: &Resolutions,
    discard_local: bool,
) -> Result<Taken> {
    let repo = Repo::open(store).ok_or_else(|| Error::msg(no_branch(store)))?;
    require_branch(&repo)?;
    let mut out = Taken {
        taken: entities.to_vec(),
        renumbered: Vec::new(),
        synced: false,
        conflicts: Vec::new(),
        discards: Vec::new(),
    };
    let paths: BTreeSet<String> = entities.iter().flat_map(take::paths).collect();
    let paths = with_board(paths);
    let mut state = load_state(store);
    let base = state.base.clone().or_else(|| repo.local());
    let upstream = repo
        .upstream()
        .ok_or_else(|| Error::msg("the wipe branch is missing"))?;

    let source_files: Files = match source {
        Source::Commit(rev) => {
            let rev = repo
                .git
                .rev(rev)
                .ok_or_else(|| Error::msg(format!("no commit `{rev}`")))?;
            let files = repo.read(&repo.tree(Some(&rev))?, &paths)?;
            // Bringing back an old version replaces the current one - but
            // uncommitted work on it is only ever replaced on request.
            if !discard_local {
                let base_index = repo.base_tree(&state, base.as_deref())?;
                let own: BTreeSet<String> = entities.iter().filter_map(own_file).collect();
                let (b, w) = (repo.read(&base_index, &own)?, read_work(store, &own));
                out.discards = entities
                    .iter()
                    .filter(|e| {
                        own_file(e)
                            .is_some_and(|p| w.get(&p) != b.get(&p) && w.get(&p) != files.get(&p))
                    })
                    .map(|e| e.id.clone())
                    .collect();
                if !out.discards.is_empty() {
                    return Ok(out);
                }
            }
            files
        }
        Source::Incoming => {
            let mut p = prepare(store, &repo, &state, base.as_deref(), &upstream, res)?;
            // Your new tickets under ids the branch also used make room first:
            // renumbered in the working copy, as a full sync would.
            if !p.merged.renumbered.is_empty() {
                let mut ours = p.ours.clone();
                out.renumbered = merge::renumber_files(&p.base, &mut ours, &p.theirs);
                let mut changes: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();
                for k in p.ours.keys().chain(ours.keys()) {
                    changes.insert(k.clone(), ours.get(k).cloned());
                }
                write_work(store, &changes, &p.ours)?;
                p = prepare(store, &repo, &state, base.as_deref(), &upstream, res)?;
            }
            out.conflicts = p
                .merged
                .conflicts
                .into_iter()
                .filter(|c| entities.iter().any(|e| concerns(c, e)))
                .collect();
            if !out.conflicts.is_empty() {
                return Ok(out);
            }
            let work = read_work(store, &paths);
            paths
                .iter()
                .filter_map(|path| {
                    merged_or_work(&p.merged.files, &work, path).map(|b| (path.clone(), b))
                })
                .collect()
        }
    };

    // The working copy.
    let before = read_work(store, &paths);
    let mut after = before.clone();
    for e in entities {
        take::apply(&mut after, &source_files, e)?;
    }
    let mut changes: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();
    for k in before.keys().chain(after.keys()) {
        changes.insert(k.clone(), after.get(k).cloned());
    }
    write_work(store, &changes, &before)?;

    // The base overlay: the same entities, as the branch has them.
    let ancestor = base
        .as_ref()
        .is_some_and(|b| repo.git.is_ancestor(b, &upstream));
    if *source == Source::Incoming && ancestor {
        let base_index = repo.base_tree(&state, base.as_deref())?;
        let up_index = repo.tree(Some(&upstream))?;
        let mut b = repo.read(&base_index, &paths)?;
        let up = repo.read(&up_index, &paths)?;
        let was = b.clone();
        for e in entities {
            take::apply(&mut b, &up, e)?;
        }
        for p in &paths {
            if b.get(p) == was.get(p) {
                continue;
            }
            let sha = match b.get(p) {
                Some(bytes) => Some(repo.git.hash_bytes(bytes)?),
                None => None,
            };
            state.overlay.insert(p.clone(), sha);
        }
        save_state(store, &state)?;
        // Nothing left incoming: move the base up to the branch.
        let base_index = repo.base_tree(&state, base.as_deref())?;
        let rest = with_board(changed(&base_index, &up_index));
        let left = diff::diff(
            &repo.read(&base_index, &rest)?,
            &repo.read(&up_index, &rest)?,
        );
        if left.is_empty() {
            let r = sync(store, false, res)?;
            out.synced = r.conflicts.is_empty();
        }
    }
    Ok(out)
}

/// One commit of the branch, placed relative to the working copy.
#[derive(Debug, Serialize)]
pub struct LogEntry {
    #[serde(flatten)]
    pub commit: CommitInfo,
    /// `incoming` (on the branch, not in your board yet), `unpublished` (yours,
    /// not on the remote yet), `base` (what your board is based on), or `past`.
    pub place: &'static str,
}

/// The branch's commits, newest first: incoming ones, then the base, then the
/// past.
pub fn log(store: &Store, limit: usize) -> Result<Vec<LogEntry>> {
    let repo = Repo::open(store).ok_or_else(|| Error::msg(no_branch(store)))?;
    require_branch(&repo)?;
    let state = load_state(store);
    let (local, remote) = repo.tips();
    let upstream = remote
        .clone()
        .or_else(|| local.clone())
        .ok_or_else(|| Error::msg("the wipe branch is missing"))?;
    let base = state.base.clone().or_else(|| local.clone());
    // Show the local tip when it is ahead (unpublished commits).
    let tip = match &local {
        Some(l) if repo.git.is_ancestor(&upstream, l) => l.clone(),
        _ => upstream.clone(),
    };
    let set = |range: String| -> BTreeSet<String> {
        repo.git
            .try_run(&["rev-list", &range])
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    };
    let incoming = match &base {
        Some(b) => set(format!("{b}..{upstream}")),
        None => BTreeSet::new(),
    };
    let unpublished = match (&remote, &local) {
        (Some(r), Some(l)) => set(format!("{r}..{l}")),
        _ => BTreeSet::new(),
    };
    Ok(crate::git::log_range(repo.git.dir(), &tip, limit)?
        .into_iter()
        .map(|c| {
            let place = if incoming.contains(&c.hash) {
                "incoming"
            } else if unpublished.contains(&c.hash) {
                "unpublished"
            } else if base.as_deref() == Some(c.hash.as_str()) {
                "base"
            } else {
                "past"
            };
            LogEntry { commit: c, place }
        })
        .collect())
}

/// The board right after a commit, and what that commit changed.
#[derive(Debug)]
pub struct AtCommit {
    pub board: crate::model::Board,
    pub lists: Vec<crate::ops::ListView>,
    /// The commit's own changes (against its parent).
    pub changes: Vec<Change>,
}

/// The board as of `rev` on the branch.
pub fn at(store: &Store, rev: &str) -> Result<AtCommit> {
    let repo = Repo::open(store).ok_or_else(|| Error::msg(no_branch(store)))?;
    let rev = repo
        .git
        .rev(rev)
        .ok_or_else(|| Error::msg(format!("no commit `{rev}`")))?;
    let index = repo.tree(Some(&rev))?;
    if !index.contains_key("board.json") {
        return Err(Error::msg(format!("commit {rev} has no board")));
    }
    let parent = repo.git.rev(&format!("{rev}^"));
    let parent_index = repo.tree(parent.as_deref())?;
    let changed_paths = with_board(changed(&parent_index, &index));
    let changes = diff::diff(
        &repo.read(&parent_index, &changed_paths)?,
        &repo.read(&index, &changed_paths)?,
    );
    // Files that differ from the working copy come from the commit; the rest
    // are read from disk.
    let work = work_index(store);
    let differ = with_board(changed(&work, &index));
    let from_rev = repo.read(&index, &differ)?;
    let read = |rel: &str| -> Option<Vec<u8>> {
        if differ.contains(rel) {
            from_rev.get(rel).cloned()
        } else {
            std::fs::read(disk_path(store, rel)).ok()
        }
    };
    let board: crate::model::Board =
        serde_json::from_slice(&read("board.json").unwrap_or_default())?;
    let lists = board
        .lists
        .iter()
        .map(|l| {
            let tickets = l
                .cards
                .iter()
                .filter_map(|id| serde_json::from_slice(&read(&format!("tickets/{id}.json"))?).ok())
                .collect();
            (l.id.clone(), tickets)
        })
        .collect();
    Ok(AtCommit {
        board,
        lists,
        changes,
    })
}

// --- moving a board onto the branch ----------------------------------------

/// What [`migrate`] did.
#[derive(Debug, Serialize)]
pub struct Migrated {
    /// The branch commit holding the board.
    pub commit: String,
    /// The board was already on the branch (pushed from another clone); the
    /// checked-out branch's board edits were merged in as local changes.
    pub adopted: bool,
    /// Published to this remote.
    pub published: Option<String>,
    /// Why publishing failed, if it did (the branch stays local until the next
    /// commit or sync).
    pub publish_error: Option<String>,
    /// The code-branch commit untracking `.wipe/` (`None`: left staged, or
    /// nothing to untrack).
    pub code_commit: Option<String>,
    /// Untracking is staged but not committed (other changes were staged).
    pub staged: bool,
    pub conflicts: Vec<Conflict>,
}

/// Options for [`migrate`].
#[derive(Debug, Clone, Default)]
pub struct MigrateOpts {
    /// Publish the new branch to the remote.
    pub publish: bool,
    /// Commit the untracking of `.wipe/` on the checked-out code branch.
    pub code_commit: bool,
    pub resolutions: Resolutions,
}

/// Move a board onto the wipe branch: seed the branch with it (from the last
/// commit of the code branch, so uncommitted edits stay local changes), stop
/// tracking `.wipe/` on the code branch and ignore it there. When the branch
/// already holds the board (another clone migrated it), the code branch's board
/// edits since it diverged are merged into the working copy instead. The caller
/// holds the board lock.
pub fn migrate(store: &Store, author: &str, opts: &MigrateOpts) -> Result<Migrated> {
    let repo = Repo::open(store).ok_or_else(|| Error::msg(no_branch(store)))?;
    if opts.publish {
        // Learn whether another clone already moved the board.
        let _ = repo.fetch();
    }
    normalize_eol(store)?;
    let (name, email) = crate::git::author_ident(author);
    let mut out = Migrated {
        commit: String::new(),
        adopted: false,
        published: None,
        publish_error: None,
        code_commit: None,
        staged: false,
        conflicts: Vec::new(),
    };
    let mut state = load_state(store);

    match repo.upstream().filter(|u| repo.has_board(u)) {
        Some(upstream) => {
            // Adopt: base = the board as of where this branch and the branch's seed
            // diverged; ours = the working copy; theirs = the branch.
            out.adopted = true;
            let seed = seed_source(&repo, &upstream);
            let fork = seed.and_then(|s| repo.git.merge_base("HEAD", &s));
            let base_index = match &fork {
                Some(f) => repo.git.ls_tree(f, &repo.dir)?,
                None => {
                    // No common history: keep the branch's board, and a copy of
                    // the working copy in the cache.
                    backup(store)?;
                    work_index(store)
                }
            };
            let up_index = repo.tree(Some(&upstream))?;
            let work = work_index(store);
            let mut paths = changed(&base_index, &up_index);
            paths.extend(changed(&base_index, &work));
            let paths = with_board(paths);
            let b = if fork.is_some() {
                repo.read(&base_index, &paths)?
            } else {
                read_work(store, &paths)
            };
            let t = repo.read(&up_index, &paths)?;
            let o = read_work(store, &paths);
            check_version(&t)?;
            let merged = merge::merge(&b, &o, &t, &opts.resolutions);
            if !merged.conflicts.is_empty() {
                out.conflicts = merged.conflicts;
                return Ok(out);
            }
            write_work(store, &merged.files, &o)?;
            if repo.local().as_deref() != Some(upstream.as_str()) {
                repo.git
                    .update_ref(LOCAL_REF, &upstream, repo.local().as_deref())?;
            }
            state.rebase(upstream.clone());
            out.commit = upstream;
        }
        None => {
            // Seed from the last commit when the code branch tracks the board, so
            // uncommitted edits show as local changes; else from the files.
            let head_index = if repo.git.rev("HEAD").is_some() {
                repo.git.ls_tree("HEAD", &repo.dir).unwrap_or_default()
            } else {
                Index::new()
            };
            let (entries, from): (Vec<(String, Option<String>)>, Option<String>) =
                if head_index.contains_key("board.json") {
                    let head = repo.git.rev("HEAD");
                    (
                        head_index.into_iter().map(|(p, s)| (p, Some(s))).collect(),
                        head,
                    )
                } else {
                    let paths = work_paths(store);
                    let shas = repo.git.hash_files(
                        &paths
                            .iter()
                            .map(|p| disk_path(store, p))
                            .collect::<Vec<_>>(),
                    )?;
                    (
                        paths.into_iter().zip(shas.into_iter().map(Some)).collect(),
                        None,
                    )
                };
            // Another board of the repo may already be on the branch.
            let parent = repo.upstream();
            let tree = repo
                .git
                .build_tree(parent.as_deref(), &repo.dir, true, &entries)?;
            let branch_name = repo
                .git
                .try_run(&["rev-parse", "--abbrev-ref", "HEAD"])
                .unwrap_or_else(|| "HEAD".into());
            let mut msg = format!("board: start the wipe branch with {}", repo.dir);
            if let Some(f) = &from {
                msg.push_str(&format!(
                    " from {branch_name}@{}\n\nWipe-Seed: {f}\n",
                    &f[..f.len().min(7)]
                ));
            }
            let parents: Vec<&str> = parent.iter().map(String::as_str).collect();
            let commit = repo
                .git
                .commit_tree(&tree, &parents, &msg, (&name, &email))?;
            repo.git
                .update_ref(LOCAL_REF, &commit, repo.local().as_deref())?;
            state.rebase(commit.clone());
            out.commit = commit.clone();
            if let (true, Some(remote)) = (opts.publish, &repo.remote) {
                match repo.git.push(remote, &commit, BRANCH, NET_TIMEOUT) {
                    Ok(Push::Done) => out.published = Some(remote.clone()),
                    Ok(Push::Rejected) => {
                        out.publish_error = Some(format!(
                            "{remote} already has a `{BRANCH}` branch that doesn't hold this board's history - run `wipe sync`"
                        ))
                    }
                    Err(e) => out.publish_error = Some(e.to_string()),
                }
            }
        }
    }
    save_state(store, &state)?;
    untrack(&repo, opts.code_commit, &name, &email, &mut out)?;
    Ok(out)
}

/// Rewrite CRLF line endings in the board's text files to LF, as wipe writes
/// them. A board tracked with the code may have been checked out with
/// `core.autocrlf`, which would make every file look changed against the branch.
fn normalize_eol(store: &Store) -> Result<()> {
    for rel in work_paths(store) {
        if !(rel.ends_with(".json") || rel.ends_with(".gitignore") || rel.ends_with(".gitkeep")) {
            continue;
        }
        let path = disk_path(store, &rel);
        let bytes = std::fs::read(&path)?;
        if bytes.windows(2).any(|w| w == b"\r\n") {
            let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
            crate::store::write_bytes_atomic(&path, text.as_bytes())?;
        }
    }
    Ok(())
}

/// The code commit the branch was seeded from (its `Wipe-Seed:` trailer).
fn seed_source(repo: &Repo, upstream: &str) -> Option<String> {
    let first = repo
        .git
        .try_run(&["rev-list", "--reverse", upstream, "--", &repo.dir])?
        .lines()
        .next()?
        .to_string();
    let msg = repo.git.try_run(&["log", "-1", "--format=%B", &first])?;
    let seed = msg
        .lines()
        .find_map(|l| l.strip_prefix("Wipe-Seed:"))?
        .trim()
        .to_string();
    repo.git.rev(&seed)
}

/// Keep a copy of the working copy under `.cache/` before it is replaced.
fn backup(store: &Store) -> Result<()> {
    let dir = store.cache_dir().join(format!(
        "pre-migrate-{}",
        Utc::now().format("%Y%m%d-%H%M%S")
    ));
    for rel in work_paths(store) {
        let from = disk_path(store, &rel);
        let to = rel.split('/').fold(dir.clone(), |p, s| p.join(s));
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&from, &to)?;
    }
    Ok(())
}

/// Stop tracking the board on the checked-out code branch and ignore it there.
fn untrack(repo: &Repo, commit: bool, name: &str, email: &str, out: &mut Migrated) -> Result<()> {
    let tracked = repo.tracked();
    let ignore_line = format!("/{}/", repo.dir);
    let top = PathBuf::from(repo.git.run(&["rev-parse", "--show-toplevel"])?);
    let gitignore = top.join(".gitignore");
    let current = std::fs::read_to_string(&gitignore).unwrap_or_default();
    let ignored = current.lines().any(|l| {
        let l = l.trim();
        l == ignore_line || l == ignore_line.trim_end_matches('/') || l == format!("{}/", repo.dir)
    });
    if !ignored {
        let mut next = current.clone();
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(&format!(
            "# The wipe board lives on the `{BRANCH}` branch; this is its working copy.\n{ignore_line}\n"
        ));
        crate::store::write_bytes_atomic(&gitignore, next.as_bytes())?;
    }
    if !tracked && (ignored || !commit) {
        return Ok(());
    }
    let top_git = Git::new(&top);
    // Anything else already staged? Then stage ours too, but don't commit theirs.
    let staged_before = top_git
        .run(&["diff", "--cached", "--name-only"])
        .unwrap_or_default();
    let others = staged_before
        .lines()
        .any(|l| !l.starts_with(&format!("{}/", repo.dir)) && l != ".gitignore");
    if tracked {
        top_git.run(&["rm", "-r", "--cached", "-q", "--", &repo.dir])?;
    }
    top_git.run(&["add", "--", ".gitignore"])?;
    if !commit || others || top_git.rev("HEAD").is_none() {
        out.staged = true;
        return Ok(());
    }
    let msg = format!("chore(wipe): move the board to the `{BRANCH}` branch\n\n{} is now a working copy of the board on the `{BRANCH}` branch\n(see `wipe status`), so it is no longer tracked here.\n", repo.dir);
    let author = format!("{name} <{email}>");
    let o = top_git.exec(
        &["commit", "-q", "--author", &author, "-m", &msg],
        &[("GIT_COMMITTER_NAME", name), ("GIT_COMMITTER_EMAIL", email)],
        None,
    )?;
    if o.ok {
        out.code_commit = top_git.rev("HEAD");
    } else {
        out.staged = true;
    }
    Ok(())
}

/// After `wipe init` in a git repo: put the new board straight onto the branch
/// (not published - the first `wipe commit` does that) and ignore `.wipe/` on
/// the code branch (left as an ordinary change to `.gitignore`).
pub fn adopt_new_board(store: &Store, author: &str) -> Result<Option<Migrated>> {
    if Repo::open(store).is_none() {
        return Ok(None);
    }
    migrate(
        store,
        author,
        &MigrateOpts {
            publish: false,
            code_commit: false,
            resolutions: Resolutions::default(),
        },
    )
    .map(Some)
}

// --- finding the working copy ----------------------------------------------

/// Where the board for `start` lives when no `.wipe/` is found walking up: in a
/// linked worktree, the main worktree's working copy (worktrees share one
/// board); in a fresh clone, a working copy checked out from the wipe branch
/// now. Returns the board root (the directory holding `.wipe`).
pub(crate) fn locate(start: &Path) -> Option<PathBuf> {
    let git = Git::new(start);
    let out = git.try_run(&[
        "rev-parse",
        "--show-toplevel",
        "--show-prefix",
        "--git-common-dir",
    ])?;
    let mut lines = out.lines();
    let top = PathBuf::from(lines.next()?);
    let prefix = lines.next().unwrap_or("").to_string();
    let common = lines.next().map(PathBuf::from)?;
    let common = if common.is_absolute() {
        common
    } else {
        top.join(common)
    };
    // The main worktree: the parent of a `.git` common dir.
    let main = (common.file_name().is_some_and(|n| n == ".git"))
        .then(|| common.parent().map(Path::to_path_buf))
        .flatten()
        .unwrap_or_else(|| top.clone());

    // Boards on the branch, by their directory prefix; the deepest one containing
    // `start` wins.
    let remote = git.remote();
    let tip = remote
        .as_ref()
        .and_then(|r| git.rev(&format!("refs/remotes/{r}/{BRANCH}")))
        .or_else(|| git.rev(LOCAL_REF));
    let mut prefixes: Vec<String> = match &tip {
        Some(t) => git
            .try_run(&["ls-tree", "-r", "--name-only", "--full-tree", t])
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.strip_suffix(".wipe/board.json"))
            .map(str::to_string)
            .collect(),
        None => Vec::new(),
    };
    prefixes.retain(|p| prefix.starts_with(p.as_str()));
    prefixes.sort_by_key(|p| std::cmp::Reverse(p.len()));

    // A working copy already in the main worktree (shared by all worktrees).
    let candidates: Vec<String> = if prefixes.is_empty() {
        // Not on the branch: still share an existing main-worktree board.
        let mut v = Vec::new();
        let mut p = prefix.clone();
        loop {
            v.push(p.clone());
            if p.is_empty() {
                break;
            }
            p = match p.trim_end_matches('/').rsplit_once('/') {
                Some((head, _)) => format!("{head}/"),
                None => String::new(),
            };
        }
        v
    } else {
        prefixes.clone()
    };
    for p in &candidates {
        let root = join_prefix(&main, p);
        if root.join(crate::WIPE_DIR).join("board.json").is_file() {
            return Some(root);
        }
    }

    // Check the board out of the branch.
    let (tip, p) = (tip?, prefixes.into_iter().next()?);
    let root = join_prefix(&main, &p);
    let dir = format!("{p}.wipe");
    let index = git.ls_tree(&tip, &dir).ok()?;
    let shas: Vec<&str> = index.values().map(String::as_str).collect();
    let blobs = git.cat_blobs(&shas).ok()?;
    let wipe = root.join(crate::WIPE_DIR);
    for ((rel, _), bytes) in index.iter().zip(blobs) {
        let path = rel.split('/').fold(wipe.clone(), |acc, s| acc.join(s));
        crate::store::write_bytes_atomic(&path, &bytes).ok()?;
    }
    let store = Store::open(&root).ok()?;
    let mut state = State::default();
    state.rebase(tip);
    save_state(&store, &state).ok()?;
    Some(root)
}

fn join_prefix(root: &Path, prefix: &str) -> PathBuf {
    prefix
        .split('/')
        .filter(|s| !s.is_empty())
        .fold(root.to_path_buf(), |p, s| p.join(s))
}

#[cfg(test)]
mod tests;

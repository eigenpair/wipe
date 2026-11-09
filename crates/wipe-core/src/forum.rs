//! The project forum: a git-tracked discussion hub beside the board, under
//! `.wipe/forum/`.
//!
//! Where tickets track *work*, the forum is for everything around it - decisions,
//! discovered rules, gotchas, questions, and knowledge that should compound over a
//! project's life. Any human or agent can post.
//!
//! Since 1.1 every post is its own file (`forum/F-01A.json`, see [`ForumEntry`])
//! with a mandatory one-sentence summary. Posts have no built-in hierarchy: a
//! post *replies* to the posts its text opens with tags for (`@F-01A ...`, see
//! [`crate::refs`]) and merely references any it tags later. The sidebar's
//! threads are the posts that reply to nothing; a thread is such a post plus
//! everything that (transitively) replies to it. Boards from before 1.1 keep
//! their nested thread files ([`Thread`]) until `wipe board translate-ids`
//! converts them; every read goes through the same flat [`PostView`]s, so both
//! formats look alike to callers.
//!
//! Reads go through a small on-disk cache (`.wipe/.cache/forum-index-v2.json`)
//! that is rebuilt only when the forum files change, so search / list / watch
//! stay cheap as the forum grows. Writes are last-writer-wins atomic renames like
//! the rest of wipe; post ids are never reused (an existing file is never
//! overwritten, even with a rewound counter).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{Attachment, ForumEntry, ForumFormat, Post, Thread, FORMAT_VERSION};
use crate::refs;
use crate::store::{ForumFile, Store};

/// A new post: an opening post, or a reply (`reply_to`).
#[derive(Debug, Default, Clone)]
pub struct NewPost {
    /// One sentence stating the post's core point. Required.
    pub summary: String,
    /// The body (Markdown). Opening `@F-...` tags make it a reply too.
    pub body: String,
    /// Labels (same pool as tickets).
    pub labels: Vec<String>,
    /// Free-form references (ticket IDs, post IDs, URLs).
    pub refs: Vec<String>,
    /// Pre-staged attachments (see [`crate::ops::stage_media`]).
    pub attachments: Vec<Attachment>,
    /// Posts this one replies to (their tags are put in front of the body).
    pub reply_to: Vec<String>,
}

/// Spec for opening a new thread (an opening post; `title` is its summary).
#[derive(Debug, Default, Clone)]
pub struct NewThread {
    /// Thread headline - the opening post's summary.
    pub title: String,
    /// Root message body.
    pub body: String,
    /// Labels (same pool as tickets).
    pub labels: Vec<String>,
    /// Free-form references (ticket IDs, post IDs, URLs).
    pub refs: Vec<String>,
    /// Pre-staged attachments (see [`crate::ops::stage_media`]).
    pub attachments: Vec<Attachment>,
}

/// Spec for replying to a post.
#[derive(Debug, Default, Clone)]
pub struct NewReply {
    /// The reply's one-sentence summary.
    pub summary: String,
    /// Reply body.
    pub body: String,
    /// Labels.
    pub labels: Vec<String>,
    /// References.
    pub refs: Vec<String>,
    /// Pre-staged attachments.
    pub attachments: Vec<Attachment>,
}

/// The thread ID that owns a legacy dotted post ID (`F-1.2.3` -> `F-1`).
pub fn thread_of(post_id: &str) -> String {
    match post_id.split_once('.') {
        Some((head, _)) => head.to_string(),
        None => post_id.to_string(),
    }
}

fn clean_summary(s: &str) -> Result<String> {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        return Err(Error::msg(
            "a post needs a summary: one sentence stating its core point (--summary)",
        ));
    }
    Ok(s)
}

/// Resolve a post reference as typed (`F-01A`, `f1a`, `@F-01A`, an old `F-3.1`
/// on a converted board) to the id of an existing post.
pub fn resolve_post_id(store: &Store, raw: &str) -> Result<String> {
    let t = raw.trim().trim_start_matches(['@', '#']);
    let all = index(store)?;
    if let Some(p) = all.iter().find(|p| p.id == t) {
        return Ok(p.id.clone());
    }
    if let Some(p) = all.iter().find(|p| p.legacy_id.as_deref() == Some(t)) {
        return Ok(p.id.clone());
    }
    // `f1a`, `F 01A`: the hex counter.
    let digits = t
        .strip_prefix(['F', 'f'])
        .map(|r| r.trim_start_matches(['-', ' ', '_']))
        .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_hexdigit()));
    if let Some(n) = digits.and_then(|d| u64::from_str_radix(d, 16).ok()) {
        let id = crate::id::hex_post_id(n);
        if all.iter().any(|p| p.id == id) {
            return Ok(id);
        }
    }
    Err(Error::PostNotFound(t.to_string()))
}

/// Write a new post. Returns its id. On a board still on the thread forum, a
/// reply goes into its first parent's thread and an opening post starts one.
pub fn create_post(
    store: &Store,
    spec: NewPost,
    author: &str,
    now: DateTime<Utc>,
) -> Result<String> {
    let summary = clean_summary(&spec.summary)?;
    let mut parents: Vec<String> = Vec::new();
    for raw in spec
        .reply_to
        .iter()
        .chain(refs::reply_parents(&spec.body).iter())
    {
        let id = resolve_post_id(store, raw)?;
        if !parents.contains(&id) {
            parents.push(id);
        }
    }
    let mut board = store.load_board()?;
    if board.forum == ForumFormat::Threads {
        return match parents.first() {
            None => legacy_create_thread(store, summary, spec, author, now),
            Some(parent) => legacy_reply(store, parent, Some(summary), spec, author, now),
        };
    }
    // Never reuse an id: skip past any file already on disk (board.json and the
    // forum files can diverge across git operations or a crash between writes).
    let mut n = board.next_thread;
    let mut id = crate::id::hex_post_id(n);
    while store.thread_exists(&id) {
        n += 1;
        id = crate::id::hex_post_id(n);
    }
    board.next_thread = n + 1;
    let entry = ForumEntry {
        version: FORMAT_VERSION,
        id: id.clone(),
        uid: Some(crate::id::uid()),
        legacy_id: None,
        summary,
        author: author.to_string(),
        body: refs::with_reply_prefix(&spec.body, &parents),
        labels: spec.labels,
        attachments: spec.attachments,
        refs: spec.refs,
        created: now,
        edited: None,
    };
    // Counter first: an interruption over-allocates (a gap), never reuses.
    store.save_board(&board)?;
    store.save_entry(&entry)?;
    Ok(id)
}

/// Open a new thread: an opening post whose summary is `title`. Returns its id.
pub fn create_thread(
    store: &Store,
    spec: NewThread,
    author: &str,
    now: DateTime<Utc>,
) -> Result<String> {
    create_post(
        store,
        NewPost {
            summary: spec.title,
            body: spec.body,
            labels: spec.labels,
            refs: spec.refs,
            attachments: spec.attachments,
            reply_to: Vec::new(),
        },
        author,
        now,
    )
}

/// Reply to an existing post. Returns the new post's ID.
pub fn reply(
    store: &Store,
    parent_id: &str,
    spec: NewReply,
    author: &str,
    now: DateTime<Utc>,
) -> Result<String> {
    create_post(
        store,
        NewPost {
            summary: spec.summary,
            body: spec.body,
            labels: spec.labels,
            refs: spec.refs,
            attachments: spec.attachments,
            reply_to: vec![parent_id.to_string()],
        },
        author,
        now,
    )
}

fn legacy_create_thread(
    store: &Store,
    title: String,
    spec: NewPost,
    author: &str,
    now: DateTime<Utc>,
) -> Result<String> {
    let mut board = store.load_board()?;
    let mut n = board.next_thread;
    let mut id = format!("F-{n}");
    while store.thread_exists(&id) {
        n += 1;
        id = format!("F-{n}");
    }
    board.next_thread = n + 1;
    let mut root = Post::new(id.clone(), author, spec.body, now);
    root.labels = spec.labels;
    root.refs = spec.refs;
    root.attachments = spec.attachments;
    let thread = Thread {
        version: FORMAT_VERSION,
        id: id.clone(),
        uid: Some(crate::id::uid()),
        title,
        root,
        created: now,
        updated: now,
    };
    store.save_board(&board)?;
    store.save_thread(&thread)?;
    Ok(id)
}

fn legacy_reply(
    store: &Store,
    parent_id: &str,
    summary: Option<String>,
    spec: NewPost,
    author: &str,
    now: DateTime<Utc>,
) -> Result<String> {
    let mut thread = store.load_thread(&thread_of(parent_id))?;
    let parent = thread
        .root
        .find_mut(parent_id)
        .ok_or_else(|| Error::PostNotFound(parent_id.to_string()))?;
    let child_id = format!("{parent_id}.{}", parent.next_reply);
    parent.next_reply += 1;
    let mut post = Post::new(child_id.clone(), author, spec.body, now);
    post.labels = spec.labels;
    post.refs = spec.refs;
    post.attachments = spec.attachments;
    post.summary = summary;
    parent.replies.push(post);
    thread.updated = now;
    store.save_thread(&thread)?;
    Ok(child_id)
}

/// Where a post lives on disk.
enum Located {
    Entry(ForumEntry),
    /// A post inside a legacy thread file.
    InThread(Thread),
}

fn locate(store: &Store, id: &str) -> Result<Located> {
    if !id.contains('.') {
        match store.load_forum_file(id) {
            Ok(ForumFile::Post(p)) => return Ok(Located::Entry(p)),
            Ok(ForumFile::Thread(t)) => return Ok(Located::InThread(t)),
            Err(_) => {}
        }
    }
    let t = store
        .load_thread(&thread_of(id))
        .map_err(|_| Error::PostNotFound(id.to_string()))?;
    if t.root.find(id).is_none() {
        return Err(Error::PostNotFound(id.to_string()));
    }
    Ok(Located::InThread(t))
}

/// Edit a post's body and/or summary (records an `edited` timestamp). Editing
/// a body's opening tags changes what the post replies to.
pub fn edit_post(
    store: &Store,
    id: &str,
    body: Option<&str>,
    summary: Option<&str>,
    now: DateTime<Utc>,
) -> Result<()> {
    let summary = summary.map(clean_summary).transpose()?;
    match locate(store, id)? {
        Located::Entry(mut p) => {
            if let Some(b) = body {
                p.body = b.to_string();
            }
            if let Some(s) = summary {
                p.summary = s;
            }
            p.edited = Some(now);
            store.save_entry(&p)
        }
        Located::InThread(mut thread) => {
            let is_root = thread.id == id;
            if let (true, Some(s)) = (is_root, &summary) {
                thread.title = s.clone();
            }
            let post = thread
                .root
                .find_mut(id)
                .ok_or_else(|| Error::PostNotFound(id.to_string()))?;
            if let Some(b) = body {
                post.body = b.to_string();
            }
            if let (false, Some(s)) = (is_root, summary) {
                post.summary = Some(s);
            }
            post.edited = Some(now);
            thread.updated = now;
            store.save_thread(&thread)
        }
    }
}

/// Reattribute a post to `new_author`, stamping `edited` so the correction is
/// visible. Returns the previous author.
pub fn reattribute_post(
    store: &Store,
    id: &str,
    new_author: &str,
    now: DateTime<Utc>,
) -> Result<String> {
    match locate(store, id)? {
        Located::Entry(mut p) => {
            let old = std::mem::replace(&mut p.author, new_author.to_string());
            p.edited = Some(now);
            store.save_entry(&p)?;
            Ok(old)
        }
        Located::InThread(mut thread) => {
            let post = thread
                .root
                .find_mut(id)
                .ok_or_else(|| Error::PostNotFound(id.to_string()))?;
            let old = std::mem::replace(&mut post.author, new_author.to_string());
            post.edited = Some(now);
            thread.updated = now;
            store.save_thread(&thread)?;
            Ok(old)
        }
    }
}

/// Add (`on`) or remove a label on a post - e.g. `pinned` on an opening post.
/// Returns whether anything changed. Not stamped as an edit: labels are
/// metadata. (On a thread-forum board, only a thread's root carries labels.)
pub fn set_label(store: &Store, id: &str, label: &str, on: bool) -> Result<bool> {
    fn toggle(labels: &mut Vec<String>, label: &str, on: bool) -> bool {
        let has = labels.iter().any(|l| l == label);
        if has == on {
            return false;
        }
        if on {
            labels.push(label.to_string());
        } else {
            labels.retain(|l| l != label);
        }
        true
    }
    match locate(store, id)? {
        Located::Entry(mut p) => {
            let changed = toggle(&mut p.labels, label, on);
            if changed {
                store.save_entry(&p)?;
            }
            Ok(changed)
        }
        Located::InThread(mut thread) => {
            let changed = toggle(&mut thread.root.labels, label, on);
            if changed {
                store.save_thread(&thread)?;
            }
            Ok(changed)
        }
    }
}

/// [`set_label`] on a thread's opening post.
pub fn set_root_label(
    store: &Store,
    thread_id: &str,
    label: &str,
    on: bool,
    _now: DateTime<Utc>,
) -> Result<bool> {
    set_label(store, &thread_of(thread_id), label, on)
}

/// Delete a post. A post is just its own file: replies to it stay (one whose
/// only parent is gone becomes an opening post). On a thread-forum board a
/// post takes its reply subtree with it, and a root the whole thread.
pub fn delete_post(store: &Store, id: &str, now: DateTime<Utc>) -> Result<()> {
    match locate(store, id)? {
        Located::Entry(p) => store.delete_thread(&p.id),
        Located::InThread(mut thread) => {
            if thread.id == id {
                return store.delete_thread(id);
            }
            if !thread.root.remove_child(id) {
                return Err(Error::PostNotFound(id.to_string()));
            }
            thread.updated = now;
            store.save_thread(&thread)
        }
    }
}

/// One post by id (resolving typed forms and old ids).
pub fn get_post(store: &Store, id: &str) -> Result<PostView> {
    let id = resolve_post_id(store, id)?;
    index(store)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or(Error::PostNotFound(id))
}

/// The whole thread a post belongs to - its opening post and everything that
/// (transitively) replies to it - oldest first.
pub fn thread_posts(store: &Store, id: &str) -> Result<Vec<PostView>> {
    let post = get_post(store, id)?;
    let mut posts: Vec<PostView> = index(store)?
        .into_iter()
        .filter(|p| p.thread_id == post.thread_id)
        .collect();
    posts.sort_by_key(|p| p.created);
    Ok(posts)
}

/// `id` and every post that replies to it, directly or not (any parent counts),
/// oldest first.
pub fn subtree(all: &[PostView], id: &str) -> Vec<PostView> {
    let mut keep: HashSet<&str> = HashSet::from([id]);
    // Posts are few; iterate to a fixed point instead of building an index.
    loop {
        let before = keep.len();
        for p in all {
            if p.parents.iter().any(|x| keep.contains(x.as_str())) {
                keep.insert(&p.id);
            }
        }
        if keep.len() == before {
            break;
        }
    }
    let mut out: Vec<PostView> = all
        .iter()
        .filter(|p| keep.contains(p.id.as_str()))
        .cloned()
        .collect();
    out.sort_by_key(|p| p.created);
    out
}

// --- flattened view + cache ------------------------------------------------

/// A single post with its context, whichever format it is stored in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostView {
    /// Post ID (`F-01A`; dotted `F-1.2` on a thread-forum board).
    pub id: String,
    /// The id it had before the forum was converted (`F-1.2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
    /// The post's one-sentence summary (derived from the body for replies
    /// written before summaries existed).
    pub summary: String,
    /// The opening post of its thread (its own id for an opening post).
    pub thread_id: String,
    /// That opening post's summary - the thread title.
    pub thread_title: String,
    /// The posts it replies to (empty for an opening post).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    /// Author identity.
    pub author: String,
    /// Message body.
    pub body: String,
    /// Labels on this post.
    pub labels: Vec<String>,
    /// References on this post.
    pub refs: Vec<String>,
    /// Replies between it and its opening post (opening post = 0).
    pub depth: usize,
    /// Number of posts replying to it directly.
    pub replies: usize,
    /// Number of attachments.
    pub attachments: usize,
    /// The attachments themselves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<Attachment>,
    /// Created timestamp.
    pub created: DateTime<Utc>,
    /// Edited timestamp, if any.
    pub edited: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    /// (file count, summed size + mtime nanos) - changes on any add/remove/edit.
    signature: (usize, u128),
    posts: Vec<PostView>,
}

/// A cheap fingerprint of the forum directory: the file count plus a running sum
/// of each file's size and modification time. Any post added, removed, or edited
/// changes it. Size is included so the cache still invalidates when two writes
/// land within the filesystem's mtime resolution.
fn forum_signature(store: &Store) -> (usize, u128) {
    let dir = store.forum_dir();
    let mut count = 0usize;
    let mut sum: u128 = 0;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            count += 1;
            if let Ok(meta) = entry.metadata() {
                sum = sum.wrapping_add(meta.len() as u128);
                if let Ok(mt) = meta.modified() {
                    if let Ok(d) = mt.duration_since(UNIX_EPOCH) {
                        sum = sum.wrapping_add(d.as_nanos());
                    }
                }
            }
        }
    }
    (count, sum)
}

fn cache_path(store: &Store) -> std::path::PathBuf {
    store.cache_dir().join("forum-index-v2.json")
}

fn view_of_entry(e: &ForumEntry) -> PostView {
    PostView {
        id: e.id.clone(),
        legacy_id: e.legacy_id.clone(),
        summary: e.summary.clone(),
        thread_id: String::new(),
        thread_title: String::new(),
        parents: refs::reply_parents(&e.body),
        author: e.author.clone(),
        body: e.body.clone(),
        labels: e.labels.clone(),
        refs: e.refs.clone(),
        depth: 0,
        replies: 0,
        attachments: e.attachments.len(),
        media: e.attachments.clone(),
        created: e.created,
        edited: e.edited,
    }
}

fn views_of_thread(t: &Thread, out: &mut Vec<PostView>) {
    fn walk(t: &Thread, p: &Post, parent: Option<&str>, out: &mut Vec<PostView>) {
        let summary = match parent {
            None => t.title.clone(),
            Some(_) => p
                .summary
                .clone()
                .unwrap_or_else(|| refs::derive_summary(&p.body)),
        };
        out.push(PostView {
            id: p.id.clone(),
            legacy_id: None,
            summary,
            thread_id: String::new(),
            thread_title: String::new(),
            parents: parent.map(str::to_string).into_iter().collect(),
            author: p.author.clone(),
            body: p.body.clone(),
            labels: p.labels.clone(),
            refs: p.refs.clone(),
            depth: 0,
            replies: 0,
            attachments: p.attachments.len(),
            media: p.attachments.clone(),
            created: p.created,
            edited: p.edited,
        });
        for r in &p.replies {
            walk(t, r, Some(&p.id), out);
        }
    }
    walk(t, &t.root, None, out);
}

/// Turn forum files into flat views with parents, threads and depths resolved,
/// newest first.
pub fn views(files: &[ForumFile]) -> Vec<PostView> {
    let mut posts: Vec<PostView> = Vec::new();
    for f in files {
        match f {
            ForumFile::Post(e) => posts.push(view_of_entry(e)),
            ForumFile::Thread(t) => views_of_thread(t, &mut posts),
        }
    }
    // Parents: old ids to current ones; missing posts and self-tags dropped.
    let known: HashSet<String> = posts.iter().map(|p| p.id.clone()).collect();
    let legacy: HashMap<String, String> = posts
        .iter()
        .filter_map(|p| Some((p.legacy_id.clone()?, p.id.clone())))
        .collect();
    for p in &mut posts {
        let mut resolved: Vec<String> = Vec::new();
        for x in &p.parents {
            let id = if known.contains(x) {
                x.clone()
            } else if let Some(n) = legacy.get(x) {
                n.clone()
            } else {
                continue;
            };
            if id != p.id && !resolved.contains(&id) {
                resolved.push(id);
            }
        }
        p.parents = resolved;
    }
    // Reply counts, then each post's opening post along its first parents.
    let mut replies: HashMap<String, usize> = HashMap::new();
    for p in &posts {
        for x in &p.parents {
            *replies.entry(x.clone()).or_default() += 1;
        }
    }
    let first: HashMap<String, String> = posts
        .iter()
        .filter_map(|p| Some((p.id.clone(), p.parents.first()?.clone())))
        .collect();
    let summaries: HashMap<String, String> = posts
        .iter()
        .map(|p| (p.id.clone(), p.summary.clone()))
        .collect();
    for p in &mut posts {
        let mut at = p.id.clone();
        let mut seen: HashSet<String> = HashSet::from([at.clone()]);
        let mut depth = 0;
        // A cycle (posts opening with each other's tags) stops where it closes.
        while let Some(up) = first.get(&at) {
            if !seen.insert(up.clone()) {
                break;
            }
            at = up.clone();
            depth += 1;
        }
        p.depth = depth;
        p.thread_title = summaries.get(&at).cloned().unwrap_or_default();
        p.thread_id = at;
        p.replies = replies.get(&p.id).copied().unwrap_or(0);
    }
    posts.sort_by_key(|p| std::cmp::Reverse(p.created));
    posts
}

/// Every post across the forum, flattened, newest-first. Served from an on-disk
/// cache that is rebuilt only when the forum files change.
pub fn index(store: &Store) -> Result<Vec<PostView>> {
    let sig = forum_signature(store);
    let cpath = cache_path(store);

    if let Ok(bytes) = std::fs::read(&cpath) {
        if let Ok(cache) = serde_json::from_slice::<CacheFile>(&bytes) {
            if cache.signature == sig {
                return Ok(cache.posts);
            }
        }
    }

    let posts = views(&store.load_forum()?);

    // Best-effort cache write; never fail a read over a cache issue.
    if let Some(dir) = cpath.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut s) = serde_json::to_string(&CacheFile {
        signature: sig,
        posts: posts.clone(),
    }) {
        s.push('\n');
        let _ = std::fs::write(&cpath, s);
    }
    Ok(posts)
}

/// The opening posts labeled `label`, oldest first, each with its replies
/// (oldest first) - what `wipe forum digest` prints.
pub fn labeled_threads(store: &Store, label: &str) -> Result<Vec<(PostView, Vec<PostView>)>> {
    let all = index(store)?;
    let mut by_thread: BTreeMap<String, Vec<PostView>> = BTreeMap::new();
    for p in &all {
        if p.depth > 0 {
            by_thread
                .entry(p.thread_id.clone())
                .or_default()
                .push(p.clone());
        }
    }
    let mut out: Vec<(PostView, Vec<PostView>)> = all
        .iter()
        .filter(|p| p.depth == 0 && p.labels.iter().any(|l| l == label))
        .map(|r| {
            let mut replies = by_thread.remove(&r.id).unwrap_or_default();
            replies.sort_by_key(|p| p.created);
            (r.clone(), replies)
        })
        .collect();
    out.sort_by_key(|(r, _)| r.created);
    Ok(out)
}

// --- search ----------------------------------------------------------------

/// A forum search. All set filters must match (AND).
#[derive(Debug, Default, Clone)]
pub struct SearchQuery {
    /// Regex matched against the body and summary (only opening posts'
    /// summaries with `titles_only`).
    pub pattern: Option<Regex>,
    /// Author substring (case-insensitive).
    pub author: Option<String>,
    /// Labels that must all be present.
    pub labels: Vec<String>,
    /// Restrict to a thread, or a post and its direct replies (`F-01A`; on a
    /// thread-forum board also a dotted subtree like `F-1.2`).
    pub scope: Option<String>,
    /// Only consider posts at this depth or shallower.
    pub max_depth: Option<usize>,
    /// Match only thread titles (root posts).
    pub titles_only: bool,
    /// Cap the number of results.
    pub limit: Option<usize>,
}

/// Compile a search pattern. `case_insensitive` adds the `i` flag; multi-line
/// mode is on so `^`/`$` anchor to each line of a multi-line post body (the
/// intuitive behavior for line-oriented content). Rust's `regex` is linear-time,
/// so user patterns cannot cause catastrophic backtracking / ReDoS.
pub fn compile_pattern(pat: &str, case_insensitive: bool) -> Result<Regex> {
    regex::RegexBuilder::new(pat)
        .case_insensitive(case_insensitive)
        .multi_line(true)
        .build()
        .map_err(|e| Error::msg(format!("invalid search pattern: {e}")))
}

/// True if `p` is within `scope`: its thread, the post itself or a reply to it,
/// or (dotted ids) its subtree - the trailing `.` makes `F-1` match `F-1.2` but
/// not `F-10`.
fn in_scope(p: &PostView, scope: &str) -> bool {
    p.id == scope
        || p.thread_id == scope
        || p.parents.iter().any(|x| x == scope)
        || p.id.starts_with(&format!("{scope}."))
}

/// Whether a post matches a query (all set filters AND together). Shared by
/// [`search`] and `wipe forum watch` so both apply identical semantics.
pub fn matches(p: &PostView, q: &SearchQuery) -> bool {
    if q.titles_only && p.depth != 0 {
        return false;
    }
    if let Some(scope) = &q.scope {
        if !in_scope(p, scope) {
            return false;
        }
    }
    if let Some(md) = q.max_depth {
        if p.depth > md {
            return false;
        }
    }
    if let Some(a) = &q.author {
        if !p.author.to_lowercase().contains(&a.to_lowercase()) {
            return false;
        }
    }
    if !q.labels.iter().all(|l| p.labels.contains(l)) {
        return false;
    }
    if let Some(re) = &q.pattern {
        let hit = if q.titles_only {
            re.is_match(&p.summary)
        } else {
            // A post's summary states its point: searched with the body.
            re.is_match(&p.body) || re.is_match(&p.summary)
        };
        if !hit {
            return false;
        }
    }
    true
}

/// Run a search over the (cached) flattened forum.
pub fn search(store: &Store, q: &SearchQuery) -> Result<Vec<PostView>> {
    let mut out: Vec<PostView> = index(store)?
        .into_iter()
        .filter(|p| matches(p, q))
        .collect();
    if let Some(n) = q.limit {
        out.truncate(n);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 3, 12, min, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        at(0)
    }

    fn project() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path(), "Forum", now()).unwrap();
        (dir, store)
    }

    /// A board still on the thread forum (before 1.1).
    fn legacy_project() -> (tempfile::TempDir, Store) {
        let (d, s) = project();
        let mut b = s.load_board().unwrap();
        b.forum = ForumFormat::Threads;
        s.save_board(&b).unwrap();
        (d, s)
    }

    fn post(body: &str) -> NewReply {
        NewReply {
            summary: format!("re: {body}"),
            body: body.into(),
            ..Default::default()
        }
    }

    fn thread(s: &Store, title: &str, body: &str, labels: &[&str], by: &str) -> String {
        let spec = NewThread {
            title: title.into(),
            body: body.into(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
            ..Default::default()
        };
        create_thread(s, spec, by, now()).unwrap()
    }

    fn ids(s: &Store) -> Vec<String> {
        let mut v: Vec<String> = index(s).unwrap().into_iter().map(|p| p.id).collect();
        v.sort();
        v
    }

    #[test]
    fn posts_are_files_and_replies_are_opening_tags() {
        let (_d, s) = project();
        let op = thread(&s, "Use OAuth", "We use OAuth 2.1", &[], "ada");
        assert_eq!(op, "F-001");
        let r1 = reply(&s, "f1", post("agree"), "bob", at(1)).unwrap();
        assert_eq!(r1, "F-002");
        assert_eq!(s.load_entry(&r1).unwrap().body, "@F-001 agree");
        // A tag later in the text is a reference, not a reply.
        let side = NewPost {
            summary: "Token refresh races".into(),
            body: "related to @F-001".into(),
            ..Default::default()
        };
        let other = create_post(&s, side, "cy", at(2)).unwrap();
        // Two parents: a reply to both.
        let both = NewPost {
            summary: "Merging the two".into(),
            body: "@F-002 @F-003 both true".into(),
            ..Default::default()
        };
        let r2 = create_post(&s, both, "dan", at(3)).unwrap();

        let v = |id: &str| get_post(&s, id).unwrap();
        assert_eq!(v(&op).depth, 0);
        assert_eq!(
            v(&other).thread_id,
            other,
            "a mid-text tag doesn't make a reply"
        );
        assert_eq!(v(&r2).parents, vec!["F-002", "F-003"]);
        assert_eq!((v(&r2).thread_id.as_str(), v(&r2).depth), ("F-001", 2));
        assert_eq!(v(&op).replies, 1);
        assert_eq!(v(&r2).thread_title, "Use OAuth");
        let in_thread: Vec<String> = thread_posts(&s, &r1)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(in_thread, ["F-001", "F-002", "F-004"]);
        let sub: Vec<String> = subtree(&index(&s).unwrap(), &other)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(sub, ["F-003", "F-004"]);

        // A summary is required.
        let bare = NewPost {
            body: "x".into(),
            ..Default::default()
        };
        assert!(create_post(&s, bare, "a", now()).is_err());

        // Deleting a post leaves its replies; one with no parent left opens a thread.
        delete_post(&s, &r1, now()).unwrap();
        assert_eq!(v(&r2).parents, vec!["F-003"]);
        assert_eq!(v(&r2).thread_id, "F-003");

        // Editing the opening tags re-parents; never reusing an id.
        edit_post(
            &s,
            &other,
            Some("@F-001 now a reply"),
            Some("Now a reply"),
            now(),
        )
        .unwrap();
        assert_eq!(v(&other).thread_id, "F-001");
        assert_eq!(v(&other).summary, "Now a reply");
        let mut b = s.load_board().unwrap();
        b.next_thread = 1;
        s.save_board(&b).unwrap();
        // Existing files are skipped; a rewound counter can only refill a gap.
        assert_eq!(thread(&s, "T2", "two", &[], "a"), "F-002");
    }

    #[test]
    fn thread_forum_boards_keep_dotted_ids() {
        let (_d, s) = legacy_project();
        assert_eq!(thread(&s, "Decisions", "Use OAuth", &[], "ada"), "F-1");
        assert_eq!(
            reply(&s, "F-1", post("agree"), "bob", now()).unwrap(),
            "F-1.1"
        );
        assert_eq!(
            reply(&s, "F-1", post("also"), "cy", now()).unwrap(),
            "F-1.2"
        );
        assert_eq!(
            reply(&s, "F-1.1", post("why?"), "dan", now()).unwrap(),
            "F-1.1.1"
        );
        // The cached index follows every write.
        assert_eq!(ids(&s), ["F-1", "F-1.1", "F-1.1.1", "F-1.2"]);
        let deep = get_post(&s, "F-1.1.1").unwrap();
        assert_eq!(
            (deep.depth, deep.parents.clone()),
            (2, vec!["F-1.1".to_string()])
        );
        assert_eq!(deep.summary, "re: why?");
        assert_eq!(get_post(&s, "F-1").unwrap().summary, "Decisions");

        delete_post(&s, "F-1.1", now()).unwrap();
        assert_eq!(
            ids(&s),
            ["F-1", "F-1.2"],
            "a post takes its subtree with it"
        );

        // Even with the counter rewound, an existing thread is never overwritten.
        let mut b = s.load_board().unwrap();
        b.next_thread = 1;
        s.save_board(&b).unwrap();
        assert_eq!(thread(&s, "T2", "two", &[], "a"), "F-2");
        assert_eq!(s.load_thread("F-1").unwrap().root.body, "Use OAuth");

        delete_post(&s, "F-1", now()).unwrap();
        assert_eq!(ids(&s), ["F-2"]);
    }

    #[test]
    fn search_by_pattern_author_label_title_and_scope() {
        let (_d, s) = project();
        thread(
            &s,
            "Auth design",
            "We use OAuth 2.1\nERROR: boom",
            &["decision"],
            "ada@x.com",
        );
        let gotcha = NewReply {
            summary: "Refresh races".into(),
            body: "beware token refresh races".into(),
            labels: vec!["gotcha".into()],
            ..Default::default()
        };
        reply(&s, "F-001", gotcha, "claude", now()).unwrap();
        thread(&s, "Unrelated", "nothing to see", &[], "bob");

        let re = |p: &str| Some(compile_pattern(p, true).unwrap());
        let cases: Vec<(&str, SearchQuery, Vec<&str>)> = vec![
            (
                "regex over bodies",
                SearchQuery {
                    pattern: re("oauth|token"),
                    ..Default::default()
                },
                vec!["F-001", "F-002"],
            ),
            (
                "summaries are searched too",
                SearchQuery {
                    pattern: re("design"),
                    ..Default::default()
                },
                vec!["F-001"],
            ),
            (
                "anchors are line-scoped",
                SearchQuery {
                    pattern: re("^ERROR"),
                    ..Default::default()
                },
                vec!["F-001"],
            ),
            (
                "author",
                SearchQuery {
                    author: Some("claude".into()),
                    ..Default::default()
                },
                vec!["F-002"],
            ),
            (
                "label",
                SearchQuery {
                    labels: vec!["gotcha".into()],
                    ..Default::default()
                },
                vec!["F-002"],
            ),
            (
                "titles only",
                SearchQuery {
                    pattern: re("auth|races"),
                    titles_only: true,
                    ..Default::default()
                },
                vec!["F-001"],
            ),
            (
                "scope",
                SearchQuery {
                    scope: Some("F-001".into()),
                    ..Default::default()
                },
                vec!["F-001", "F-002"],
            ),
        ];
        for (what, q, want) in cases {
            let mut got: Vec<String> = search(&s, &q).unwrap().into_iter().map(|h| h.id).collect();
            got.sort();
            assert_eq!(got, want, "{what}");
        }
    }

    #[test]
    fn path_traversal_ids_are_rejected() {
        let (_d, s) = project();
        thread(&s, "T", "x", &[], "a");
        for bad in [
            "F-1/../../secret",
            "..",
            "F-1/../F-1",
            "/etc/passwd",
            "F-1\\..\\x",
        ] {
            assert!(get_post(&s, bad).is_err(), "get_post({bad})");
            assert!(
                reply(&s, bad, post("x"), "a", now()).is_err(),
                "reply({bad})"
            );
            assert!(delete_post(&s, bad, now()).is_err(), "delete_post({bad})");
        }
        assert!(get_post(&s, "F-001").is_ok());
    }
}

//! One-time translation of a legacy board to the current id formats: decimal
//! ticket IDs (`T-23`) become hex (`T-017`), and a thread forum (one file per
//! thread, nested replies `F-3.1`) becomes one file per post (`F-00A`), each
//! reply opening with a tag for the post it answered (`@F-009 ...`).
//!
//! Forum posts get ids in the order they were written; each keeps its old id as
//! `legacy_id`, and a reply without a summary gets one derived from its first
//! sentence. `F-3.1` mentions anywhere (tickets, comments, posts) follow.
//!
//! Every place a ticket ID is stored is rewritten consistently: the ticket files
//! (renamed), the board's cards, relations, subscriptions, trash entries, forum
//! refs, and `T-<n>` mentions inside titles, bodies, comments, checklist items
//! and activity. Each ticket keeps its old ID as `legacy_id`, so `T-23` typed
//! later (or found in a commit message) still resolves.
//!
//! Crash safety: the complete result (every file's new content, plus which old
//! files go away) is computed first and written as one journal in the
//! gitignored cache - the commit point. Applying the journal only writes
//! precomputed values and removes files no new one reuses, so it is idempotent:
//! if it is interrupted (a crash, a file held open by another process), the next
//! run - or any CLI command - replays it. `board.json` is written last, so a
//! board only reads as translated once everything else is in place. Boards left
//! half-converted by 0.4.1 (tickets renamed, `board.json` still decimal) are
//! finished the same way.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{
    Board, ForumEntry, ForumFormat, IdFormat, Post, Subscriptions, Thread, Ticket, FORMAT_VERSION,
};
use crate::trash::TrashEntry;
use crate::Store;

const JOURNAL: &str = "translate-journal.json";

/// Everything a translation writes, computed before anything is touched.
#[derive(Serialize, Deserialize)]
struct Journal {
    /// `(old, new)` ids in counter order.
    pairs: Vec<(String, String)>,
    tickets: Vec<Ticket>,
    /// Ticket files to remove: old names that no translated ticket reuses.
    remove_tickets: Vec<String>,
    threads: Vec<Thread>,
    /// Forum posts in the one-file-per-post format (a converted thread forum).
    #[serde(default)]
    posts: Vec<ForumEntry>,
    /// Thread files to remove once their posts are written.
    #[serde(default)]
    remove_forum: Vec<String>,
    subs: Option<Subscriptions>,
    trash: Vec<TrashEntry>,
    remove_trash: Vec<String>,
    board: Board,
}

/// Rewrite every whole-word `T-<digits>` in `text` that names a translated ticket.
fn rewrite_refs(text: &str, map: &BTreeMap<String, String>, re: &Regex) -> String {
    re.replace_all(text, |c: &regex::Captures| {
        let old = &c[0];
        map.get(old).cloned().unwrap_or_else(|| old.to_string())
    })
    .into_owned()
}

fn rewrite_ticket(t: &mut Ticket, map: &BTreeMap<String, String>, re: &Regex) {
    let r = |s: &str| rewrite_refs(s, map, re);
    if let Some(new) = map.get(&t.id) {
        t.legacy_id = Some(std::mem::replace(&mut t.id, new.clone()));
    }
    t.title = r(&t.title);
    t.body = r(&t.body);
    if let Some(o) = &mut t.original {
        o.title = r(&o.title);
        o.body = r(&o.body);
    }
    for rel in &mut t.relations {
        if let Some(new) = map.get(&rel.target) {
            rel.target = new.clone();
        }
    }
    for c in &mut t.comments {
        c.body = r(&c.body);
    }
    for i in t.checklist.iter_mut().chain(t.acceptance.iter_mut()) {
        i.text = r(&i.text);
    }
    for a in &mut t.activity {
        a.detail = r(&a.detail);
    }
}

fn decimal_counter(id: &str) -> Result<u64> {
    id.strip_prefix("T-")
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| Error::msg(format!("unexpected ticket id `{id}` on a decimal board")))
}

/// Whether an interrupted translation is waiting to be finished: a journal,
/// or a decimal board with a 0.4.1 staging directory (a run that renamed the
/// tickets but never got to write `board.json`). Cheap: two directory lookups.
pub fn interrupted(store: &Store) -> bool {
    store.cache_dir().join(JOURNAL).is_file()
        || (!stage_dirs(store).is_empty()
            && store.load_board().is_ok_and(|b| b.ids == IdFormat::Decimal))
}

/// Translate a legacy decimal board to hex ticket IDs, or finish a translation
/// that was interrupted. Returns the `(old, new)` pairs in counter order; empty
/// when the board is already hex. Callers must hold the board's write lock.
pub fn translate_ids(store: &Store, now: DateTime<Utc>) -> Result<Vec<(String, String)>> {
    let journal_path = store.cache_dir().join(JOURNAL);
    if journal_path.is_file() {
        let j: Journal = serde_json::from_slice(&std::fs::read(&journal_path)?)?;
        apply(store, &j).map_err(unfinished)?;
        return Ok(j.pairs);
    }
    let Some(j) = plan(store, now)? else {
        cleanup(store);
        return Ok(Vec::new());
    };
    std::fs::create_dir_all(store.cache_dir())?;
    crate::store::write_json_atomic(&journal_path, &j)?;
    apply(store, &j).map_err(unfinished)?;
    Ok(j.pairs)
}

fn unfinished(e: Error) -> Error {
    Error::msg(format!(
        "ticket-id translation interrupted: {e}. Nothing is lost - it is saved and \
         finishes on the next wipe command (or `wipe board translate-ids`). If this \
         repeats, close whatever has the board's files open (an editor, a sync client)"
    ))
}

/// What [`repair`] changed on a hex board.
#[derive(Debug, Default, PartialEq)]
pub struct Repair {
    /// Ticket / trash ids converted from leftover decimal form, `(old, new)`.
    pub renamed: Vec<(String, String)>,
    /// Forum threads, subscriptions and relations whose refs were rewritten.
    pub refs_fixed: usize,
    /// Decimal-looking ids that could also be a real hex id - left untouched.
    pub ambiguous: Vec<String>,
}

impl Repair {
    /// Whether anything was changed.
    pub fn changed(&self) -> bool {
        !self.renamed.is_empty() || self.refs_fixed > 0
    }
}

/// Whether `id` on a hex board is a leftover decimal id rather than a real hex
/// one. Hex ids are zero-padded to three digits and always below the board's
/// counter, so `T-5`/`T-42` are decimal, `T-005`/`T-02A` are hex, and a
/// three-digit `T-300` is decimal only if hex 0x300 was never allocated.
fn leftover_decimal(id: &str, next_ticket: u64) -> Option<u64> {
    let digits = id.strip_prefix("T-")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if digits.len() >= 3 && digits.starts_with('0') {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    if digits.len() < 3 || u64::from_str_radix(digits, 16).map_or(true, |h| h >= next_ticket) {
        Some(n)
    } else {
        None
    }
}

/// Finish a hex board that still holds decimal references the translation
/// never reached: trash entries and stray ticket files with decimal ids, forum
/// refs and `T-<n>` mentions in threads, subscriptions and relations. This
/// happens to boards repaired by hand after 0.4.1 (`board.json` flipped to hex,
/// the forum and trash left as they were). Idempotent; callers must hold the
/// board's write lock.
pub fn repair(store: &Store, now: DateTime<Utc>) -> Result<Repair> {
    let mut board = store.load_board()?;
    let mut report = Repair::default();
    if board.ids != IdFormat::Hex {
        return Ok(report);
    }
    let next = board.next_ticket;
    let tickets = store.load_all_tickets()?;
    let trash = crate::trash::read_entries(store)?;

    // Every id in use, and old -> new for everything already translated.
    let mut taken: BTreeSet<String> = tickets
        .iter()
        .map(|t| t.id.clone())
        .chain(trash.iter().map(|e| e.ticket.id.clone()))
        .collect();
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for t in tickets.iter().chain(trash.iter().map(|e| &e.ticket)) {
        if let Some(old) = &t.legacy_id {
            map.insert(old.clone(), t.id.clone());
        }
    }
    // Leftover decimal ids get their hex id, unless it is already taken.
    let mut convert = |t: &Ticket, report: &mut Repair| -> Option<String> {
        if t.legacy_id.is_some() {
            return None;
        }
        let n = leftover_decimal(&t.id, next)?;
        let new = crate::id::hex_ticket_id(n);
        if taken.contains(&new) || map.values().any(|v| *v == new) {
            report.ambiguous.push(t.id.clone());
            return None;
        }
        taken.insert(new.clone());
        map.insert(t.id.clone(), new.clone());
        Some(new)
    };
    let stray: Vec<&Ticket> = tickets
        .iter()
        .filter(|t| convert(t, &mut report).is_some())
        .collect();
    let stray_trash: Vec<&TrashEntry> = trash
        .iter()
        .filter(|e| convert(&e.ticket, &mut report).is_some())
        .collect();
    let re = Regex::new(r"\bT-\d+\b").expect("static regex");

    for t in stray {
        let mut t = t.clone();
        let old = t.id.clone();
        rewrite_ticket(&mut t, &map, &re);
        store.save_ticket(&t)?;
        store.remove_ticket_file(&old)?;
        report.renamed.push((old, t.id));
    }
    for e in stray_trash {
        let mut e = e.clone();
        let old = e.ticket.id.clone();
        rewrite_ticket(&mut e.ticket, &map, &re);
        crate::trash::write_entry(store, &e)?;
        crate::trash::remove_entry_file(store, &old)?;
        report.renamed.push((old, e.ticket.id));
    }
    // A ref is stale when it names an old id that is not also a live one (a
    // live hex `T-100` wins over a legacy `T-100`, as in `resolve_ticket_id`).
    let renamed: BTreeSet<&str> = report.renamed.iter().map(|(o, _)| o.as_str()).collect();
    let stale: BTreeMap<String, String> = map
        .iter()
        .filter(|(old, _)| !taken.contains(old.as_str()) || renamed.contains(old.as_str()))
        .map(|(o, n)| (o.clone(), n.clone()))
        .collect();
    if stale.is_empty() {
        return Ok(report);
    }
    let fix = |r: &mut String| -> bool {
        match stale.get(r.as_str()) {
            Some(new) => {
                *r = new.clone();
                true
            }
            None => false,
        }
    };

    let mut cards_changed = false;
    for c in board.lists.iter_mut().flat_map(|l| l.cards.iter_mut()) {
        cards_changed |= fix(c);
    }
    if cards_changed {
        board.updated = now;
        store.save_board(&board)?;
    }
    for mut t in store.load_all_tickets()? {
        let mut changed = false;
        for rel in &mut t.relations {
            changed |= fix(&mut rel.target);
        }
        if changed {
            store.save_ticket(&t)?;
            report.refs_fixed += 1;
        }
    }
    let mut subs = store.load_subscriptions()?;
    let mut subs_changed = false;
    for r in subs.subs.values_mut().flat_map(|v| v.iter_mut()) {
        subs_changed |= fix(r);
    }
    if subs_changed {
        store.save_subscriptions(&subs)?;
        report.refs_fixed += 1;
    }
    // Forum text was never translated (`T-30` -> `T-01E`; a real `T-030` stays).
    for mut thread in store.load_all_threads()? {
        let before = thread.clone();
        thread.title = rewrite_refs(&thread.title, &stale, &re);
        let mut stack = vec![&mut thread.root];
        while let Some(p) = stack.pop() {
            p.body = rewrite_refs(&p.body, &stale, &re);
            for r in &mut p.refs {
                fix(r);
            }
            stack.extend(p.replies.iter_mut());
        }
        if thread != before {
            store.save_thread(&thread)?;
            report.refs_fixed += 1;
        }
    }
    Ok(report)
}

/// Whether a board still uses an old id format: decimal ticket ids, or the
/// thread forum.
pub fn needed(board: &Board) -> bool {
    board.ids != IdFormat::Hex || board.forum == ForumFormat::Threads
}

/// Compute the translation without writing anything (`None` if nothing is left
/// to translate).
fn plan(store: &Store, now: DateTime<Utc>) -> Result<Option<Journal>> {
    let board = store.load_board()?;
    if !needed(&board) {
        return Ok(None);
    }
    let mut j = if board.ids == IdFormat::Hex {
        Journal {
            pairs: Vec::new(),
            tickets: Vec::new(),
            remove_tickets: Vec::new(),
            threads: Vec::new(),
            posts: Vec::new(),
            remove_forum: Vec::new(),
            subs: None,
            trash: Vec::new(),
            remove_trash: Vec::new(),
            board,
        }
    } else {
        plan_tickets(store, board, now)?
    };
    if j.board.forum == ForumFormat::Threads {
        plan_forum(store, &mut j, now)?;
    }
    Ok(Some(j))
}

/// The ticket half of [`plan`]: decimal ids to hex.
fn plan_tickets(store: &Store, mut board: Board, now: DateTime<Utc>) -> Result<Journal> {
    let on_disk = store.ticket_ids()?;

    // Tickets that already carry a legacy_id were translated by an interrupted
    // 0.4.1 run: keep them as they are. Its staging snapshot (complete, taken
    // under the lock) supplies any it deleted but never wrote back.
    let mut done: BTreeMap<String, Ticket> = BTreeMap::new();
    let mut todo: Vec<Ticket> = Vec::new();
    for t in store.load_all_tickets()? {
        if t.legacy_id.is_some() {
            done.insert(t.id.clone(), t);
        } else {
            todo.push(t);
        }
    }
    for t in staged_tickets(store) {
        if t.legacy_id.is_some() && !done.contains_key(&t.id) {
            done.insert(t.id.clone(), t);
        }
    }
    let already: BTreeSet<String> = done.values().filter_map(|t| t.legacy_id.clone()).collect();
    // A leftover old file of a ticket that was already translated is stale.
    todo.retain(|t| !already.contains(&t.id));

    let trash_on_disk = crate::trash::read_entries(store)?;
    let trash_ids: Vec<String> = trash_on_disk.iter().map(|e| e.ticket.id.clone()).collect();
    let (trash_done, trash_todo): (Vec<TrashEntry>, Vec<TrashEntry>) = trash_on_disk
        .into_iter()
        .partition(|e| e.ticket.legacy_id.is_some());

    // old -> new for every ticket on the board and in the trash.
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for t in done.values().chain(trash_done.iter().map(|e| &e.ticket)) {
        if let Some(old) = &t.legacy_id {
            map.insert(old.clone(), t.id.clone());
        }
    }
    for id in todo
        .iter()
        .map(|t| &t.id)
        .chain(trash_todo.iter().map(|e| &e.ticket.id))
    {
        map.insert(id.clone(), crate::id::hex_ticket_id(decimal_counter(id)?));
    }
    let mut seen = BTreeSet::new();
    if let Some(dup) = map.values().find(|new| !seen.insert(*new)) {
        return Err(Error::msg(format!(
            "cannot translate: two tickets would both become `{dup}`"
        )));
    }
    let re = Regex::new(r"\bT-\d+\b").expect("static regex");

    let mut tickets: Vec<Ticket> = done.into_values().collect();
    for mut t in todo {
        rewrite_ticket(&mut t, &map, &re);
        tickets.push(t);
    }
    let keep: BTreeSet<&str> = tickets.iter().map(|t| t.id.as_str()).collect();
    let remove_tickets = on_disk
        .into_iter()
        .filter(|id| !keep.contains(id.as_str()))
        .collect();

    let mut trash = trash_done;
    for mut e in trash_todo {
        rewrite_ticket(&mut e.ticket, &map, &re);
        trash.push(e);
    }
    let keep: BTreeSet<&str> = trash.iter().map(|e| e.ticket.id.as_str()).collect();
    let remove_trash = trash_ids
        .into_iter()
        .filter(|id| !keep.contains(id.as_str()))
        .collect();

    for l in &mut board.lists {
        for c in &mut l.cards {
            if let Some(new) = map.get(c) {
                *c = new.clone();
            }
        }
    }
    board.ids = IdFormat::Hex;
    board.updated = now;

    let mut subs = store.load_subscriptions()?;
    let mut subs_changed = false;
    for refs in subs.subs.values_mut() {
        for r in refs.iter_mut() {
            if let Some(new) = map.get(r) {
                *r = new.clone();
                subs_changed = true;
            }
        }
    }

    let mut threads = Vec::new();
    for mut thread in store.load_all_threads()? {
        let before = thread.clone();
        thread.title = rewrite_refs(&thread.title, &map, &re);
        let mut stack = vec![&mut thread.root];
        while let Some(p) = stack.pop() {
            p.body = rewrite_refs(&p.body, &map, &re);
            for r in &mut p.refs {
                if let Some(new) = map.get(r) {
                    *r = new.clone();
                }
            }
            stack.extend(p.replies.iter_mut());
        }
        if thread != before {
            threads.push(thread);
        }
    }

    let mut pairs: Vec<(String, String)> = map.into_iter().collect();
    pairs.sort_by_key(|(old, _)| decimal_counter(old).unwrap_or(u64::MAX));
    Ok(Journal {
        pairs,
        tickets,
        remove_tickets,
        threads,
        posts: Vec::new(),
        remove_forum: Vec::new(),
        subs: subs_changed.then_some(subs),
        trash,
        remove_trash,
        board,
    })
}

/// The forum half of [`plan`]: every thread's posts become files of their own,
/// numbered in the order they were written. Works on the ticket half's result
/// (threads with ticket refs already rewritten), so nothing is rewritten twice.
fn plan_forum(store: &Store, j: &mut Journal, now: DateTime<Utc>) -> Result<()> {
    // The threads as the ticket half left them (it keeps only changed ones).
    let mut threads: BTreeMap<String, Thread> = store
        .load_all_threads()?
        .into_iter()
        .map(|t| (t.id.clone(), t))
        .collect();
    for t in std::mem::take(&mut j.threads) {
        threads.insert(t.id.clone(), t);
    }
    // Every post with its thread, parent and summary, oldest first.
    struct Old {
        post: Post,
        parent: Option<String>,
        summary: String,
        uid: Option<String>,
    }
    let mut old: Vec<Old> = Vec::new();
    for t in threads.values() {
        fn walk(t: &Thread, p: &Post, parent: Option<&str>, out: &mut Vec<Old>) {
            let summary = match parent {
                None => t.title.clone(),
                Some(_) => p
                    .summary
                    .clone()
                    .unwrap_or_else(|| crate::refs::derive_summary(&p.body)),
            };
            let mut post = p.clone();
            post.replies.clear();
            out.push(Old {
                post,
                parent: parent.map(str::to_string),
                summary,
                uid: if parent.is_none() {
                    t.uid.clone()
                } else {
                    None
                },
            });
            for r in &p.replies {
                walk(t, r, Some(&p.id), out);
            }
        }
        walk(t, &t.root, None, &mut old);
    }
    // Stable: posts written in the same instant keep their thread order.
    old.sort_by_key(|o| o.post.created);
    let map: BTreeMap<String, String> = old
        .iter()
        .enumerate()
        .map(|(i, o)| (o.post.id.clone(), crate::id::hex_post_id(i as u64 + 1)))
        .collect();
    let fix_ref = |r: &mut String| {
        if let Some(new) = map.get(r.as_str()) {
            *r = new.clone();
        }
    };
    for o in old {
        let parent = o.parent.as_ref().and_then(|p| map.get(p)).cloned();
        let body = crate::refs::rewrite(&o.post.body, &map);
        let mut refs = o.post.refs.clone();
        refs.iter_mut().for_each(fix_ref);
        j.posts.push(ForumEntry {
            version: FORMAT_VERSION,
            id: map[&o.post.id].clone(),
            uid: Some(o.uid.unwrap_or_else(crate::id::uid)),
            legacy_id: Some(o.post.id.clone()),
            summary: crate::refs::rewrite(&o.summary, &map),
            author: o.post.author,
            body: crate::refs::with_reply_prefix(&body, &parent.into_iter().collect::<Vec<_>>()),
            labels: o.post.labels,
            attachments: o.post.attachments,
            refs,
            created: o.post.created,
            edited: o.post.edited,
        });
    }
    j.remove_forum = threads.into_keys().collect();

    // `F-3.1` mentions elsewhere: tickets (those the ticket half translated, or
    // all of them on a hex board), trash, subscriptions.
    if !map.is_empty() {
        let fix_text = |t: &mut Ticket| -> bool {
            let before = t.clone();
            let r = |x: &str| crate::refs::rewrite(x, &map);
            t.title = r(&t.title);
            t.body = r(&t.body);
            if let Some(o) = &mut t.original {
                o.title = r(&o.title);
                o.body = r(&o.body);
            }
            for c in &mut t.comments {
                c.body = r(&c.body);
            }
            for i in t.checklist.iter_mut().chain(t.acceptance.iter_mut()) {
                i.text = r(&i.text);
            }
            for a in &mut t.activity {
                a.detail = r(&a.detail);
            }
            *t != before
        };
        if j.pairs.is_empty() {
            for mut t in store.load_all_tickets()? {
                if fix_text(&mut t) {
                    j.tickets.push(t);
                }
            }
            for mut e in crate::trash::read_entries(store)? {
                if fix_text(&mut e.ticket) {
                    j.trash.push(e);
                }
            }
        } else {
            for t in &mut j.tickets {
                fix_text(t);
            }
            for e in &mut j.trash {
                fix_text(&mut e.ticket);
            }
        }
        let had = j.subs.is_some();
        let mut subs = match j.subs.take() {
            Some(s) => s,
            None => store.load_subscriptions()?,
        };
        let mut changed = false;
        for r in subs.subs.values_mut().flat_map(|v| v.iter_mut()) {
            if let Some(new) = r.strip_prefix("forum:").and_then(|id| map.get(id)) {
                *r = format!("forum:{new}");
                changed = true;
            }
        }
        if changed || had {
            j.subs = Some(subs);
        }
    }
    let mut pairs: Vec<(String, String)> = map.into_iter().collect();
    pairs.sort_by(|a, b| a.1.cmp(&b.1));
    j.pairs.extend(pairs);
    j.board.next_thread = j.posts.len() as u64 + 1;
    j.board.forum = ForumFormat::Posts;
    j.board.updated = now;
    Ok(())
}

/// Write the journal's values: new files first, then the removal of old names
/// no new file reuses (decimal `T-100` and hex `T-100` coincide), `board.json`
/// last. Safe to repeat after any interruption.
fn apply(store: &Store, j: &Journal) -> Result<()> {
    for t in &j.tickets {
        store.save_ticket(t)?;
    }
    for id in &j.remove_tickets {
        store.remove_ticket_file(id)?;
    }
    for t in &j.threads {
        store.save_thread(t)?;
    }
    for p in &j.posts {
        store.save_entry(p)?;
    }
    for id in &j.remove_forum {
        let path = store.forum_dir().join(format!("{id}.json"));
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    if let Some(subs) = &j.subs {
        store.save_subscriptions(subs)?;
    }
    for e in &j.trash {
        crate::trash::write_entry(store, e)?;
    }
    for id in &j.remove_trash {
        crate::trash::remove_entry_file(store, id)?;
    }
    store.save_board(&j.board)?;
    std::fs::remove_file(store.cache_dir().join(JOURNAL))?;
    cleanup(store);
    Ok(())
}

/// Tickets in 0.4.1's staging directories (`.cache/translate-<ts>/`).
fn staged_tickets(store: &Store) -> Vec<Ticket> {
    stage_dirs(store)
        .into_iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .filter_map(|f| std::fs::read(f.path()).ok())
        .filter_map(|b| serde_json::from_slice::<Ticket>(&b).ok())
        .collect()
}

fn stage_dirs(store: &Store) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(store.cache_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("translate-"))
        })
        .collect();
    dirs.sort();
    dirs
}

fn cleanup(store: &Store) {
    for d in stage_dirs(store) {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Board, Relation, RelationKind};
    use crate::ops::{self, NewTicket};

    /// A board as 0.4.0 wrote it: decimal ids, no `ids` field.
    fn legacy_board(n: usize) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::init(dir.path(), "Legacy", Utc::now()).unwrap();
        let mut b: Board = s.load_board().unwrap();
        b.ids = IdFormat::Decimal;
        b.forum = ForumFormat::Threads;
        s.save_board(&b).unwrap();
        for i in 0..n {
            let spec = NewTicket {
                title: format!("ticket {i}"),
                list: Some("todo".into()),
                ..Default::default()
            };
            ops::create_ticket(&s, spec, "ada", Utc::now()).unwrap();
        }
        (dir, s)
    }

    fn new_ticket(s: &Store, title: &str) -> String {
        let spec = NewTicket {
            title: title.into(),
            list: Some("todo".into()),
            ..Default::default()
        };
        ops::create_ticket(s, spec, "ada", Utc::now()).unwrap().id
    }

    #[test]
    fn translates_ids_everywhere_and_keeps_legacy_aliases() {
        let (_d, s) = legacy_board(300);
        // Before: plain decimal ids, ordered numerically.
        assert_eq!(s.resolve_ticket_id("t12").unwrap(), "T-12");
        assert!(s.resolve_ticket_id("T-301").is_err());
        assert_eq!(s.ticket_ids().unwrap()[8..11], ["T-9", "T-10", "T-11"]);

        // References in text, relations, subscriptions, forum, trash.
        let mut t = s.load_ticket("T-3").unwrap();
        t.body = "see T-10 and T-256, not T-1000 or XT-10".into();
        t.relations.push(Relation {
            kind: RelationKind::BlockedBy,
            target: "T-100".into(),
        });
        s.save_ticket(&t).unwrap();
        ops::add_comment(&s, "T-3", "ada", "follow-up in T-299", Utc::now()).unwrap();
        crate::inbox::subscribe(&s, "ada", "T-256").unwrap();
        let spec = crate::forum::NewThread {
            title: "about T-12".into(),
            body: "T-12 is done".into(),
            refs: vec!["T-12".into()],
            ..Default::default()
        };
        crate::forum::create_thread(&s, spec, "ada", Utc::now()).unwrap();
        // Old T-101 becomes T-065 while old T-257 becomes T-101: both trashed, both
        // must survive (they once shared a file name mid-rekey).
        for id in ["T-300", "T-101", "T-257"] {
            crate::trash::trash_ticket(&s, id, 7, Utc::now()).unwrap();
        }

        let pairs = translate_ids(&s, Utc::now()).unwrap();
        assert_eq!(pairs.len(), 301, "300 tickets and the forum's one post");
        assert_eq!(pairs[0], ("T-1".into(), "T-001".into()));
        assert_eq!(pairs[255], ("T-256".into(), "T-100".into()));

        let b = s.load_board().unwrap();
        assert_eq!(b.ids, IdFormat::Hex);
        assert_eq!(b.lists[1].cards[9], "T-00A");
        assert_eq!(b.lists[1].cards.len(), 297);
        // Old T-100 and new T-100 (= 256) did not clobber each other.
        assert_eq!(s.load_ticket("T-064").unwrap().title, "ticket 99");
        assert_eq!(s.load_ticket("T-100").unwrap().title, "ticket 255");

        let t = s.load_ticket("T-003").unwrap();
        assert_eq!(t.legacy_id.as_deref(), Some("T-3"));
        assert_eq!(t.body, "see T-00A and T-100, not T-1000 or XT-10");
        assert_eq!(t.relations[0].target, "T-064");
        assert_eq!(t.comments[0].body, "follow-up in T-12B");
        assert_eq!(
            crate::inbox::subscriptions_of(&s, "ada").unwrap(),
            ["T-100"]
        );
        let post = s.load_entry("F-001").unwrap();
        assert_eq!(
            (
                post.summary.as_str(),
                post.refs[0].as_str(),
                post.legacy_id.as_deref()
            ),
            ("about T-00C", "T-00C", Some("F-1"))
        );
        let mut trashed: Vec<String> = crate::trash::list_trash(&s, 7, Utc::now())
            .unwrap()
            .into_iter()
            .map(|e| e.ticket.id)
            .collect();
        trashed.sort();
        assert_eq!(trashed, ["T-065", "T-101", "T-12C"]);
        crate::trash::restore_ticket(&s, "T-12C", Utc::now()).unwrap();

        // Every way of naming a ticket still resolves.
        for (typed, want) in [
            ("T-003", "T-003"),
            ("t3", "T-003"),
            ("T-3", "T-003"),   // legacy decimal, short
            ("T-82", "T-052"),  // legacy decimal 82, not hex 0x82
            ("T-082", "T-082"), // padded: canonical hex
            ("T-2a", "T-02A"),
            ("T-100", "T-100"), // canonical wins over legacy T-100 (= T-064)
        ] {
            assert_eq!(s.resolve_ticket_id(typed).unwrap(), want, "{typed}");
        }
        // New tickets continue the counter in hex; a second run is a no-op.
        assert_eq!(new_ticket(&s, "new"), "T-12D");
        assert!(translate_ids(&s, Utc::now()).unwrap().is_empty());
        assert_eq!(s.ticket_ids().unwrap()[..3], ["T-001", "T-002", "T-003"]);
    }

    /// A small legacy board with cross-references in a ticket, the forum and
    /// subscriptions.
    fn legacy_board_with_refs() -> (tempfile::TempDir, Store) {
        let (d, s) = legacy_board(12);
        let mut t = s.load_ticket("T-3").unwrap();
        t.body = "see T-10".into();
        s.save_ticket(&t).unwrap();
        crate::inbox::subscribe(&s, "ada", "T-11").unwrap();
        let spec = crate::forum::NewThread {
            title: "about T-12".into(),
            body: "T-12".into(),
            refs: vec!["T-12".into()],
            ..Default::default()
        };
        crate::forum::create_thread(&s, spec, "ada", Utc::now()).unwrap();
        (d, s)
    }

    fn assert_fully_translated(s: &Store) {
        let b = s.load_board().unwrap();
        assert_eq!(b.ids, IdFormat::Hex);
        assert_eq!(b.lists[1].cards[9], "T-00A");
        let ids = s.ticket_ids().unwrap();
        assert_eq!(ids.len(), 12);
        assert!(ids.iter().all(|id| id.len() == 5), "stale files: {ids:?}");
        for c in b.lists.iter().flat_map(|l| &l.cards) {
            s.load_ticket(c).unwrap();
        }
        assert_eq!(s.load_ticket("T-003").unwrap().body, "see T-00A");
        assert_eq!(
            s.load_ticket("T-00C").unwrap().legacy_id.as_deref(),
            Some("T-12")
        );
        assert_eq!(crate::inbox::subscriptions_of(s, "ada").unwrap(), ["T-00B"]);
        assert_eq!(s.load_entry("F-001").unwrap().refs[0], "T-00C");
        assert_eq!(b.forum, ForumFormat::Posts);
        assert!(!interrupted(s));
        assert!(stage_dirs(s).is_empty());
        assert!(!s.cache_dir().join(JOURNAL).exists());
    }

    /// The states an interrupted translation can leave behind - each must be
    /// finished by the next run, never redone (redoing would re-map refs that
    /// are already translated).
    #[test]
    fn interrupted_translations_are_finished_not_redone() {
        // `stage`: write 0.4.1's staging snapshot; `remove`/`save`: how far its
        // swap got; `journal`: a 0.4.2+ run stopped while applying its journal.
        struct Case {
            what: &'static str,
            stage: bool,
            remove: usize,
            save: usize,
            journal: bool,
        }
        for c in [
            // What 0.4.1 left on real boards when board.json could not be written:
            // every ticket renamed, everything else still decimal. Before 0.4.2 the
            // re-run failed: "unexpected ticket id `T-001` on a decimal board".
            Case {
                what: "0.4.1: board.json not written",
                stage: true,
                remove: 12,
                save: 12,
                journal: false,
            },
            Case {
                what: "0.4.1: killed mid-swap",
                stage: true,
                remove: 8,
                save: 1,
                journal: false,
            },
            Case {
                what: "journal partly applied",
                stage: false,
                remove: 3,
                save: 5,
                journal: true,
            },
        ] {
            let (_d, s) = legacy_board_with_refs();
            let j = plan(&s, Utc::now()).unwrap().unwrap();
            if c.stage {
                let stage = s.cache_dir().join("translate-1790875456");
                std::fs::create_dir_all(&stage).unwrap();
                for t in &j.tickets {
                    let json = serde_json::to_string(t).unwrap();
                    std::fs::write(stage.join(format!("{}.json", t.id)), json).unwrap();
                }
            }
            if c.journal {
                crate::store::write_json_atomic(&s.cache_dir().join(JOURNAL), &j).unwrap();
            }
            for id in &j.remove_tickets[..c.remove] {
                s.remove_ticket_file(id).unwrap();
            }
            for t in &j.tickets[..c.save] {
                s.save_ticket(t).unwrap();
            }
            assert!(interrupted(&s), "{}", c.what);
            assert_eq!(
                translate_ids(&s, Utc::now()).unwrap().len(),
                13,
                "{}",
                c.what
            );
            assert_fully_translated(&s);
        }
    }

    /// A thread forum becomes one file per post, in the order they were
    /// written: replies open with their parent's tag, keep their old id, get a
    /// summary, and every `F-3.1` mention follows - also on hex boards.
    #[test]
    fn converts_the_thread_forum_to_posts() {
        use crate::forum::{self, NewReply, NewThread};
        let (_d, s) = legacy_board(2);
        let mut b = s.load_board().unwrap();
        b.ids = IdFormat::Hex;
        s.save_board(&b).unwrap();
        let t0 = Utc::now();
        let at = |m: i64| t0 + chrono::Duration::minutes(m);
        let thread = |title: &str, m: i64| {
            let spec = NewThread {
                title: title.into(),
                body: format!("{title}!"),
                ..Default::default()
            };
            forum::create_thread(&s, spec, "ada", at(m)).unwrap()
        };
        let reply = |parent: &str, body: &str, summary: &str, m: i64| {
            let spec = NewReply {
                summary: summary.into(),
                body: body.into(),
                ..Default::default()
            };
            forum::reply(&s, parent, spec, "bob", at(m)).unwrap()
        };
        assert_eq!(thread("Auth", 0), "F-1");
        assert_eq!(thread("Perf", 1), "F-2");
        assert_eq!(reply("F-1", "Agreed. See F-2 too", "x", 2), "F-1.1");
        // A reply from before summaries: none stored, derived on conversion.
        let mut th = s.load_thread("F-1").unwrap();
        th.root.replies[0].summary = None;
        s.save_thread(&th).unwrap();
        assert_eq!(reply("F-1.1", "why?", "Asking why", 3), "F-1.1.1");
        ops::add_comment(&s, "T-1", "ada", "decided in F-1.1", Utc::now()).unwrap();
        crate::inbox::subscribe(&s, "ada", "forum:F-1").unwrap();

        let pairs = translate_ids(&s, Utc::now()).unwrap();
        assert_eq!(
            pairs,
            [
                ("F-1".to_string(), "F-001".to_string()),
                ("F-2".into(), "F-002".into()),
                ("F-1.1".into(), "F-003".into()),
                ("F-1.1.1".into(), "F-004".into()),
            ]
        );
        let b = s.load_board().unwrap();
        assert_eq!((b.forum, b.next_thread), (ForumFormat::Posts, 5));
        let e = s.load_entry("F-003").unwrap();
        assert_eq!(e.body, "@F-001 Agreed. See F-002 too");
        assert_eq!(e.summary, "Agreed. See F-002 too");
        assert_eq!(e.legacy_id.as_deref(), Some("F-1.1"));
        assert_eq!(s.load_entry("F-004").unwrap().body, "@F-003 why?");
        assert!(!s.forum_dir().join("F-1.json").exists());
        let p = forum::get_post(&s, "F-1.1.1").unwrap();
        assert_eq!(
            (p.id.as_str(), p.thread_id.as_str(), p.depth),
            ("F-004", "F-001", 2)
        );
        assert_eq!(
            s.load_ticket("T-1").unwrap().comments[0].body,
            "decided in F-003"
        );
        assert_eq!(
            crate::inbox::subscriptions_of(&s, "ada").unwrap(),
            ["forum:F-001"]
        );
        // New posts continue the counter; a second run is a no-op.
        let spec = NewThread {
            title: "Next".into(),
            ..Default::default()
        };
        assert_eq!(
            forum::create_thread(&s, spec, "ada", Utc::now()).unwrap(),
            "F-005"
        );
        assert!(translate_ids(&s, Utc::now()).unwrap().is_empty());
    }

    /// Boards hand-repaired after 0.4.1 read as hex, but their trash and forum
    /// still use decimal ids (found on a real board). `repair` finishes them.
    #[test]
    fn repair_finishes_decimal_leftovers_on_a_hex_board() {
        let (_d, s) = legacy_board_with_refs();
        for id in ["T-5", "T-11"] {
            crate::trash::trash_ticket(&s, id, 7, Utc::now()).unwrap();
        }
        let snapshot = |dir: std::path::PathBuf| -> Vec<(std::path::PathBuf, Vec<u8>)> {
            std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .map(|e| (e.path(), std::fs::read(e.path()).unwrap()))
                .collect()
        };
        let (forum, trash) = (snapshot(s.forum_dir()), snapshot(s.trash_dir()));
        translate_ids(&s, Utc::now()).unwrap();
        // Put the old forum and trash back, as the hand repair left them.
        for dir in [s.forum_dir(), s.trash_dir()] {
            std::fs::remove_dir_all(&dir).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
        }
        for (p, bytes) in forum.iter().chain(&trash) {
            std::fs::write(p, bytes).unwrap();
        }
        assert_eq!(s.load_thread("F-1").unwrap().root.refs[0], "T-12");

        let r = repair(&s, Utc::now()).unwrap();
        let mut renamed = r.renamed.clone();
        renamed.sort();
        assert_eq!(
            renamed,
            [
                ("T-11".into(), "T-00B".into()),
                ("T-5".into(), "T-005".into())
            ]
        );
        assert!(r.ambiguous.is_empty() && r.refs_fixed >= 1, "{r:?}");
        let th = s.load_thread("F-1").unwrap();
        assert_eq!(
            (
                th.title.as_str(),
                th.root.body.as_str(),
                th.root.refs[0].as_str()
            ),
            ("about T-00C", "T-00C", "T-00C")
        );
        crate::trash::restore_ticket(&s, "T-5", Utc::now()).unwrap();
        assert_eq!(
            s.load_ticket("T-005").unwrap().legacy_id.as_deref(),
            Some("T-5")
        );

        assert!(!repair(&s, Utc::now()).unwrap().changed(), "idempotent");
        // A hex id that merely looks decimal is never touched.
        assert_eq!(leftover_decimal("T-100", 0x200), None);
        assert_eq!(leftover_decimal("T-100", 0x50), Some(100));
        assert_eq!(leftover_decimal("T-042", 0x200), None);
        assert_eq!(leftover_decimal("T-42", 0x200), Some(42));
    }
}

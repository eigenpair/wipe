//! Three-way merge of board snapshots.
//!
//! Boards are structured data, so they merge per field rather than per line:
//! different fields of one ticket merge cleanly, comments and forum posts are
//! unions, card moves and reorders combine, counters take the maximum, and
//! concurrent edits of a long text go through a line-level merge. Only a real
//! disagreement - the same field set to two different values, a delete against
//! an edit, a card moved to two different lists - becomes a [`Conflict`], which
//! the caller resolves by picking a side.
//!
//! IDs allocated on both sides from the same counter (two new `T-00D`s, two new
//! `c-4` comments on one ticket, two new `F-3` threads) are not conflicts: our
//! side's newcomers are renumbered past both counters, references included.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::Files;
use crate::model::{
    Board, Definitions, ForumEntry, ForumFormat, Identity, Post, Settings, Subscriptions, Thread,
    Ticket,
};

/// Which version of a conflicting value to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// The local working copy.
    Ours,
    /// The wipe branch.
    Theirs,
}

impl Side {
    pub fn parse(s: &str) -> Option<Side> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ours" | "mine" | "local" => Some(Side::Ours),
            "theirs" | "incoming" | "remote" => Some(Side::Theirs),
            _ => None,
        }
    }
}

/// How to settle conflicts: per conflict id, else a default for all of them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Resolutions {
    #[serde(default)]
    pub default: Option<Side>,
    #[serde(default)]
    pub picks: BTreeMap<String, Side>,
}

impl Resolutions {
    fn get(&self, id: &str) -> Option<Side> {
        self.picks.get(id).copied().or(self.default)
    }
}

/// A disagreement the merge can't settle on its own.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conflict {
    /// Stable id: `<file>#<pointer>` (e.g. `tickets/T-003.json#/title`).
    pub id: String,
    /// What it's about: `T-003`, `F-2`, `board`, `definitions`, a media path.
    pub entity: String,
    /// Which part: `title`, `comments[c-3]/body`, `cards[T-007]` (empty for a
    /// whole file).
    pub field: String,
    /// The value both started from (`null` when absent).
    pub base: Value,
    /// Our value (`null`: deleted).
    pub ours: Value,
    /// Their value (`null`: deleted).
    pub theirs: Value,
}

/// The outcome of a merge.
#[derive(Debug, Default)]
pub struct Merged {
    /// The merged content of every path any side touched (`None`: absent).
    pub files: BTreeMap<String, Option<Vec<u8>>>,
    /// Conflicts with no resolution; while any remain, `files` holds their side
    /// of each and must not be applied.
    pub conflicts: Vec<Conflict>,
    /// IDs of ours renumbered to avoid a clash, `(old, new)`.
    pub renumbered: Vec<(String, String)>,
}

/// Merge `ours` and `theirs`, both descended from `base`. The maps hold every
/// path that differs between any two sides, plus `board.json`; a path absent
/// from a map doesn't exist on that side.
pub fn merge(base: &Files, ours: &Files, theirs: &Files, res: &Resolutions) -> Merged {
    let mut ours = ours.clone();
    let mut out = Merged {
        renumbered: renumber_files(base, &mut ours, theirs),
        ..Default::default()
    };

    let paths: BTreeSet<&String> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .collect();
    for path in paths {
        let (b, o, t) = (base.get(path), ours.get(path), theirs.get(path));
        let merged = if o == t {
            o.cloned()
        } else if b == o {
            t.cloned()
        } else if b == t {
            o.cloned()
        } else {
            merge_file(path, b, o, t, res, &mut out.conflicts)
        };
        out.files.insert(path.clone(), merged);
    }
    fix_cards(base, &ours, theirs, res, &mut out);
    out
}

/// The entity a board file holds, for conflict labels.
pub fn entity_of(path: &str) -> String {
    let stem = |p: &str| {
        p.rsplit('/')
            .next()
            .unwrap_or(p)
            .trim_end_matches(".json")
            .to_string()
    };
    if path.starts_with("tickets/") || path.starts_with("forum/") {
        stem(path)
    } else if let Some(name) = path.strip_suffix(".json").filter(|p| !p.contains('/')) {
        if name == "board" {
            "board".into()
        } else {
            name.into()
        }
    } else {
        path.to_string()
    }
}

/// A file changed on both sides: field-level for board JSON, whole-file for the
/// rest (media).
fn merge_file(
    path: &str,
    b: Option<&Vec<u8>>,
    o: Option<&Vec<u8>>,
    t: Option<&Vec<u8>>,
    res: &Resolutions,
    conflicts: &mut Vec<Conflict>,
) -> Option<Vec<u8>> {
    let parse = |x: Option<&Vec<u8>>| -> Option<Option<Value>> {
        match x {
            None => Some(None),
            Some(bytes) => serde_json::from_slice::<Value>(bytes).ok().map(Some),
        }
    };
    let json = path.ends_with(".json");
    if let (true, Some(bv), Some(Some(ov)), Some(Some(tv))) = (json, parse(b), parse(o), parse(t)) {
        let (bv, ov, tv) = prepare(path, bv, ov, tv);
        let mut ctx = Ctx {
            file: path,
            res,
            conflicts,
        };
        let merged = merge_value(&mut ctx, "", bv.as_ref(), Some(&ov), Some(&tv))?;
        return Some(serialize(path, merged));
    }
    // Whole file: deleted against edited, unreadable, or not JSON.
    let id = format!("{path}#");
    let pick = res.get(&id);
    match pick {
        Some(Side::Ours) => o.cloned(),
        Some(Side::Theirs) => t.cloned(),
        None => {
            let show = |x: Option<&Vec<u8>>| match x {
                None => Value::Null,
                Some(bytes) if json => serde_json::from_slice(bytes).unwrap_or(Value::Null),
                Some(bytes) => Value::String(format!("{} bytes", bytes.len())),
            };
            conflicts.push(Conflict {
                id,
                entity: entity_of(path),
                field: String::new(),
                base: show(b),
                ours: show(o),
                theirs: show(t),
            });
            t.cloned()
        }
    }
}

/// Re-serialize a merged file the way the store writes it, through its typed
/// model when it fits (so key order and formatting match the store's).
fn serialize(path: &str, v: Value) -> Vec<u8> {
    fn typed<T: serde::de::DeserializeOwned + Serialize>(v: &Value) -> Option<String> {
        let t: T = serde_json::from_value(v.clone()).ok()?;
        serde_json::to_string_pretty(&t).ok()
    }
    let s = match path {
        "board.json" => typed::<Board>(&v),
        "definitions.json" => typed::<Definitions>(&v),
        "settings.json" => typed::<Settings>(&v),
        "identities.json" => typed::<Vec<Identity>>(&v),
        "subscriptions.json" => typed::<Subscriptions>(&v),
        p if p.starts_with("tickets/") => typed::<Ticket>(&v),
        p if p.starts_with("forum/") && v.get("root").is_some() => typed::<Thread>(&v),
        p if p.starts_with("forum/") => typed::<ForumEntry>(&v),
        _ => None,
    }
    .unwrap_or_else(|| serde_json::to_string_pretty(&v).unwrap_or_default());
    (s + "\n").into_bytes()
}

struct Ctx<'a> {
    file: &'a str,
    res: &'a Resolutions,
    conflicts: &'a mut Vec<Conflict>,
}

impl Ctx<'_> {
    /// Settle a disagreement at `ptr`: the chosen side, else record it and use
    /// theirs as a placeholder.
    fn conflict(
        &mut self,
        ptr: &str,
        b: Option<&Value>,
        o: Option<&Value>,
        t: Option<&Value>,
    ) -> Option<Value> {
        let id = format!("{}#{ptr}", self.file);
        match self.res.get(&id) {
            Some(Side::Ours) => o.cloned(),
            Some(Side::Theirs) => t.cloned(),
            None => {
                let v = |x: Option<&Value>| x.cloned().unwrap_or(Value::Null);
                self.conflicts.push(Conflict {
                    id,
                    entity: entity_of(self.file),
                    field: ptr.trim_start_matches('/').to_string(),
                    base: v(b),
                    ours: v(o),
                    theirs: v(t),
                });
                t.cloned()
            }
        }
    }
}

fn merge_value(
    ctx: &mut Ctx,
    ptr: &str,
    b: Option<&Value>,
    o: Option<&Value>,
    t: Option<&Value>,
) -> Option<Value> {
    if o == t {
        return o.cloned();
    }
    if b == o {
        return t.cloned();
    }
    if b == t {
        return o.cloned();
    }
    let key = ptr.rsplit('/').next().unwrap_or("");
    // A ticket's original wording is frozen by the first rewrite: theirs came first.
    if key == "original" && b.is_none() && t.is_some() {
        return t.cloned();
    }
    match (o, t) {
        (Some(Value::Object(om)), Some(Value::Object(tm))) => {
            let empty = Map::new();
            let bm = match b {
                Some(Value::Object(m)) => m,
                _ => &empty,
            };
            let mut keys: Vec<&String> = om.keys().collect();
            keys.extend(tm.keys().filter(|k| !om.contains_key(*k)));
            let mut out = Map::new();
            for k in keys {
                let child = format!("{ptr}/{k}");
                if let Some(v) = merge_value(ctx, &child, bm.get(k), om.get(k), tm.get(k)) {
                    out.insert(k.clone(), v);
                }
            }
            Some(Value::Object(out))
        }
        (Some(Value::Array(oa)), Some(Value::Array(ta))) => {
            let empty = Vec::new();
            let ba = match b {
                Some(Value::Array(a)) => a,
                _ => &empty,
            };
            Some(Value::Array(merge_array(ctx, ptr, ba, oa, ta)))
        }
        // Counters and timestamps only ever move forward.
        (Some(Value::Number(x)), Some(Value::Number(y)))
            if key.starts_with("next_") || key == "version" =>
        {
            let (x, y) = (x.as_u64().unwrap_or(0), y.as_u64().unwrap_or(0));
            Some(Value::from(x.max(y)))
        }
        (Some(Value::String(x)), Some(Value::String(y))) if matches!(key, "updated" | "edited") => {
            Some(Value::String(x.max(y).clone()))
        }
        (Some(Value::String(x)), Some(Value::String(y)))
            if matches!(key, "body" | "description") || x.contains('\n') || y.contains('\n') =>
        {
            let base = match b {
                Some(Value::String(s)) => s.as_str(),
                _ => "",
            };
            match merge_text(base, x, y) {
                Some(m) => Some(Value::String(m)),
                None => ctx.conflict(ptr, b, o, t),
            }
        }
        _ => ctx.conflict(ptr, b, o, t),
    }
}

/// The field that identifies the elements of an array of objects, if any.
fn key_field(items: &[&Value]) -> Option<&'static str> {
    ["id", "name", "path"].into_iter().find(|f| {
        !items.is_empty()
            && items
                .iter()
                .all(|v| v.get(f).and_then(Value::as_str).is_some())
    })
}

fn merge_array(ctx: &mut Ctx, ptr: &str, b: &[Value], o: &[Value], t: &[Value]) -> Vec<Value> {
    let all: Vec<&Value> = b.iter().chain(o).chain(t).collect();
    match key_field(&all) {
        Some(field) => {
            let key = |v: &Value| {
                v.get(field)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            };
            let find = |a: &[Value], k: &str| a.iter().find(|v| key(v) == k).cloned();
            let (bk, ok, tk): (Vec<String>, Vec<String>, Vec<String>) = (
                b.iter().map(key).collect(),
                o.iter().map(key).collect(),
                t.iter().map(key).collect(),
            );
            let mut merged: BTreeMap<String, Value> = BTreeMap::new();
            let union: BTreeSet<&String> = bk.iter().chain(&ok).chain(&tk).collect();
            for k in union {
                let child = format!("{ptr}[{k}]");
                let (bv, ov, tv) = (find(b, k), find(o, k), find(t, k));
                if let Some(v) = merge_value(ctx, &child, bv.as_ref(), ov.as_ref(), tv.as_ref()) {
                    merged.insert(k.clone(), v);
                }
            }
            order(&bk, &ok, &tk, &merged.keys().cloned().collect())
                .into_iter()
                .filter_map(|k| merged.remove(&k))
                .collect()
        }
        None => {
            // A set: kept unless our side removed it, plus our additions.
            let key = |v: &Value| v.to_string();
            let (bk, ok, tk): (Vec<String>, Vec<String>, Vec<String>) = (
                b.iter().map(key).collect(),
                o.iter().map(key).collect(),
                t.iter().map(key).collect(),
            );
            let (bs, os): (HashSet<&String>, HashSet<&String>) =
                (bk.iter().collect(), ok.iter().collect());
            let ts: HashSet<&String> = tk.iter().collect();
            let keep: BTreeSet<String> = tk
                .iter()
                .filter(|k| !(bs.contains(k) && !os.contains(k)))
                .chain(ok.iter().filter(|k| !bs.contains(k) || ts.contains(k)))
                .cloned()
                .collect();
            let values: BTreeMap<String, Value> =
                o.iter().chain(t).map(|v| (key(v), v.clone())).collect();
            order(&bk, &ok, &tk, &keep)
                .into_iter()
                .filter_map(|k| values.get(&k).cloned())
                .collect()
        }
    }
}

/// The order of a merged sequence: the side that reordered wins (theirs when
/// both did), and the other side's additions slot in after the element they
/// followed there.
fn order(
    base: &[String],
    ours: &[String],
    theirs: &[String],
    keep: &BTreeSet<String>,
) -> Vec<String> {
    let (primary, secondary) = if theirs == base {
        (ours, theirs)
    } else {
        (theirs, ours)
    };
    let mut out: Vec<String> = primary
        .iter()
        .filter(|k| keep.contains(*k))
        .cloned()
        .collect();
    for (i, k) in secondary.iter().enumerate() {
        if !keep.contains(k) || out.contains(k) {
            continue;
        }
        // After the element it followed; else before the one it preceded; else
        // at the end.
        let at = secondary[..i]
            .iter()
            .rev()
            .find_map(|prev| out.iter().position(|x| x == prev))
            .map(|p| p + 1)
            .or_else(|| {
                secondary[i + 1..]
                    .iter()
                    .find_map(|next| out.iter().position(|x| x == next))
            })
            .unwrap_or(out.len());
        out.insert(at, k.clone());
    }
    // Anything kept that neither sequence placed (resolved delete conflicts).
    for k in keep {
        if !out.contains(k) {
            out.push(k.clone());
        }
    }
    out
}

/// Line-level three-way merge of a text (git merge-file); `None` on overlap.
fn merge_text(base: &str, ours: &str, theirs: &str) -> Option<String> {
    let dir = tempfile::tempdir().ok()?;
    let write = |name: &str, s: &str| -> Option<std::path::PathBuf> {
        let p = dir.path().join(name);
        // A trailing newline keeps the last line mergeable like any other.
        std::fs::write(&p, format!("{s}\n")).ok()?;
        Some(p)
    };
    let (o, b, t) = (
        write("ours", ours)?,
        write("base", base)?,
        write("theirs", theirs)?,
    );
    let out = crate::vcs::command("git")
        .arg("merge-file")
        .arg("-p")
        .arg(&o)
        .arg(&b)
        .arg(&t)
        .output()
        .ok()?;
    if out.status.code() != Some(0) {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    Some(s.strip_suffix('\n').unwrap_or(&s).to_string())
}

// --- renumbering -----------------------------------------------------------

/// Renumber the tickets and threads both sides created under the same id: ours
/// get fresh ids past both counters, and every reference in our changed files
/// follows.
pub(crate) fn renumber_files(
    base: &Files,
    ours: &mut Files,
    theirs: &Files,
) -> Vec<(String, String)> {
    let mut renamed = rename_identities(base, ours, theirs);
    renamed.extend(renumber_entities(base, ours, theirs));
    renamed
}

/// Identities both sides registered under one id that are different identities
/// (two people's agents both called `claude`, or a person and an agent): the
/// incoming one keeps the id, ours yields to the next free numbered one
/// (`claude02`), and every reference to it in our changed files follows -
/// authors, assignees, subscriptions, `@claude` mentions.
fn rename_identities(base: &Files, ours: &mut Files, theirs: &Files) -> Vec<(String, String)> {
    let list = |f: &Files| -> Vec<Identity> {
        f.get("identities.json")
            .and_then(|b| serde_json::from_slice(b).ok())
            .unwrap_or_default()
    };
    let (b, o, t) = (list(base), list(ours), list(theirs));
    let in_base = |id: &str| b.iter().any(|i| i.id.eq_ignore_ascii_case(id));
    let mut taken: Vec<String> = b
        .iter()
        .chain(&o)
        .chain(&t)
        .map(|i| i.id.to_ascii_lowercase())
        .collect();
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for oi in o.iter().filter(|i| !in_base(&i.id)) {
        let Some(ti) = t.iter().find(|ti| ti.id.eq_ignore_ascii_case(&oi.id)) else {
            continue;
        };
        let same = ti.kind == oi.kind
            && (ti.owner == oi.owner || ti.owner.is_none() || oi.owner.is_none());
        if same {
            continue;
        }
        let new = (2..)
            .map(|n| format!("{}{n:02}", oi.id))
            .find(|c| !taken.contains(&c.to_ascii_lowercase()))
            .expect("an unbounded range has a free id");
        taken.push(new.to_ascii_lowercase());
        map.insert(oi.id.clone(), new);
    }
    if map.is_empty() {
        return Vec::new();
    }
    let changed: Vec<String> = ours
        .iter()
        .filter(|(p, bytes)| p.ends_with(".json") && base.get(*p) != Some(*bytes))
        .map(|(p, _)| p.clone())
        .collect();
    for p in changed {
        let Some(mut v) = ours
            .get(&p)
            .and_then(|b| serde_json::from_slice::<Value>(b).ok())
        else {
            continue;
        };
        if rename_in(&mut v, &map) {
            ours.insert(p.clone(), serialize(&p, v));
        }
    }
    map.into_iter().collect()
}

/// Rename identity ids in a JSON value: whole string values, object keys
/// (subscriptions), and `@id` mentions inside text.
fn rename_in(v: &mut Value, map: &BTreeMap<String, String>) -> bool {
    match v {
        Value::String(s) => {
            if let Some(n) = map.get(s.as_str()) {
                *s = n.clone();
                return true;
            }
            if !s.contains('@') {
                return false;
            }
            let mut out = String::with_capacity(s.len());
            let mut last = 0;
            for (range, tag) in crate::refs::scan(s) {
                if let crate::refs::Tag::Identity(id) = &tag {
                    if let Some(n) = map.get(id) {
                        out.push_str(&s[last..range.start]);
                        out.push('@');
                        out.push_str(n);
                        last = range.start + 1 + id.len();
                    }
                }
            }
            if last == 0 {
                return false;
            }
            out.push_str(&s[last..]);
            *s = out;
            true
        }
        Value::Array(a) => a.iter_mut().fold(false, |acc, x| rename_in(x, map) | acc),
        Value::Object(o) => {
            let keys: Vec<(String, String)> = o
                .keys()
                .filter_map(|k| map.get(k).map(|n| (k.clone(), n.clone())))
                .collect();
            let mut changed = !keys.is_empty();
            for (k, n) in keys {
                if let Some(x) = o.remove(&k) {
                    o.insert(n, x);
                }
            }
            for x in o.values_mut() {
                changed |= rename_in(x, map);
            }
            changed
        }
        _ => false,
    }
}

/// Renumber the tickets and posts both sides created under the same id.
fn renumber_entities(base: &Files, ours: &mut Files, theirs: &Files) -> Vec<(String, String)> {
    let board = |f: &Files| -> Option<Board> { serde_json::from_slice(f.get("board.json")?).ok() };
    let (Some(ob), Some(tb)) = (board(ours), board(theirs)) else {
        return Vec::new();
    };
    // Two new files under one id are two entities - unless they carry the same
    // uid (the same ticket reached both sides). Without uids (before 1.0),
    // identical content is the only sign of sameness.
    let uid = |bytes: &[u8]| -> Option<String> {
        serde_json::from_slice::<Value>(bytes)
            .ok()?
            .get("uid")?
            .as_str()
            .map(str::to_string)
    };
    let clashes = |dir: &str| -> Vec<String> {
        ours.iter()
            .filter(|(p, bytes)| {
                p.starts_with(dir)
                    && !base.contains_key(*p)
                    && theirs.get(*p).is_some_and(|t| match (uid(t), uid(bytes)) {
                        (Some(a), Some(b)) => a != b,
                        _ => t != *bytes,
                    })
            })
            .filter_map(|(p, _)| Some(p.strip_prefix(dir)?.strip_suffix(".json")?.to_string()))
            .collect()
    };
    let mut map: BTreeMap<String, String> = BTreeMap::new();

    // Tickets: the next counter past both boards and every id either side has.
    let hex = tb.ids == crate::model::IdFormat::Hex;
    let counter = |id: &str| -> Option<u64> {
        let n = id.strip_prefix("T-")?;
        if hex {
            u64::from_str_radix(n, 16).ok()
        } else {
            n.parse().ok()
        }
    };
    let mut next = ob.next_ticket.max(tb.next_ticket);
    for p in ours.keys().chain(theirs.keys()) {
        if let Some(n) = p
            .strip_prefix("tickets/")
            .and_then(|s| s.strip_suffix(".json"))
            .and_then(counter)
        {
            next = next.max(n + 1);
        }
    }
    for old in clashes("tickets/") {
        map.insert(old, tb.ticket_id(next));
        next += 1;
    }
    let next_ticket = next;

    // Forum posts (hex) or threads (decimal), by the board's forum format.
    let posts = tb.forum == ForumFormat::Posts;
    let mut next_thread = ob.next_thread.max(tb.next_thread);
    for p in ours.keys().chain(theirs.keys()) {
        if let Some(n) = p
            .strip_prefix("forum/F-")
            .and_then(|s| s.strip_suffix(".json"))
            .and_then(|s| {
                if posts {
                    u64::from_str_radix(s, 16).ok()
                } else {
                    s.parse::<u64>().ok()
                }
            })
        {
            next_thread = next_thread.max(n + 1);
        }
    }
    for old in clashes("forum/") {
        let new = if posts {
            crate::id::hex_post_id(next_thread)
        } else {
            format!("F-{next_thread}")
        };
        map.insert(old, new);
        next_thread += 1;
    }
    if map.is_empty() {
        return Vec::new();
    }

    // Rewrite our changed files (only they can mention our new ids) and move the
    // renumbered ones to their new paths.
    let changed: Vec<String> = ours
        .iter()
        .filter(|(p, bytes)| base.get(*p) != Some(*bytes))
        .map(|(p, _)| p.clone())
        .collect();
    for p in changed {
        let Some(bytes) = ours.remove(&p) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let rewritten = rewrite_ids(&text, &map).into_bytes();
        let moved = p
            .strip_suffix(".json")
            .and_then(|s| s.rsplit_once('/'))
            .and_then(|(dir, id)| map.get(id).map(|new| format!("{dir}/{new}.json")))
            .unwrap_or(p);
        ours.insert(moved, rewritten);
    }
    if let Some(mut b) = board(ours) {
        b.next_ticket = b.next_ticket.max(next_ticket);
        b.next_thread = b.next_thread.max(next_thread);
        if let Ok(s) = serde_json::to_string_pretty(&b) {
            ours.insert("board.json".into(), (s + "\n").into_bytes());
        }
    }
    map.into_iter().collect()
}

/// Replace whole `T-<id>` / `F-<n>` tokens found in `map` (a thread id also as
/// the head of a post id: `F-3.1` -> `F-9.1`).
pub(crate) fn rewrite_ids(text: &str, map: &BTreeMap<String, String>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let start_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if start_ok
            && i + 2 < bytes.len()
            && matches!(bytes[i], b'T' | b'F')
            && bytes[i + 1] == b'-'
        {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
                j += 1;
            }
            if j > i + 2 {
                if let Some(new) = map.get(&text[i..j]) {
                    out.push_str(new);
                    i = j;
                    continue;
                }
            }
        }
        let ch = text[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Per-file preparation before the field merge: renumber the comments,
/// checklist items, criteria and replies both sides added under the same id.
fn prepare(path: &str, b: Option<Value>, o: Value, t: Value) -> (Option<Value>, Value, Value) {
    if path.starts_with("tickets/") {
        if let (Ok(mut ot), Ok(tt)) = (
            serde_json::from_value::<Ticket>(o.clone()),
            serde_json::from_value::<Ticket>(t.clone()),
        ) {
            let bt = b
                .clone()
                .and_then(|v| serde_json::from_value::<Ticket>(v).ok());
            if renumber_ticket(bt.as_ref(), &mut ot, &tt) {
                if let Ok(v) = serde_json::to_value(&ot) {
                    return (b, v, t);
                }
            }
        }
    } else if path.starts_with("forum/") {
        if let (Ok(mut oth), Ok(tth)) = (
            serde_json::from_value::<Thread>(o.clone()),
            serde_json::from_value::<Thread>(t.clone()),
        ) {
            let mut base_ids = HashSet::new();
            if let Some(bth) = b
                .clone()
                .and_then(|v| serde_json::from_value::<Thread>(v).ok())
            {
                bth.root.walk(0, &mut |p, _| {
                    base_ids.insert(p.id.clone());
                });
            }
            if renumber_replies(&mut oth.root, &tth.root, &base_ids) {
                if let Ok(v) = serde_json::to_value(&oth) {
                    return (b, v, t);
                }
            }
        }
    }
    (b, o, t)
}

fn renumber_ticket(b: Option<&Ticket>, o: &mut Ticket, t: &Ticket) -> bool {
    let mut changed = false;
    let base_has = |ids: &dyn Fn(&Ticket) -> Vec<String>, id: &str| {
        b.is_some_and(|b| ids(b).iter().any(|x| x == id))
    };

    // Comments.
    let ids = |x: &Ticket| x.comments.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
    let mut next = o.next_comment.max(t.next_comment);
    for c in &mut o.comments {
        let clash = !base_has(&ids, &c.id) && t.comments.iter().any(|tc| tc.id == c.id && tc != c);
        if clash {
            c.id = crate::id::comment_id(next);
            next += 1;
            changed = true;
        }
    }
    o.next_comment = o.next_comment.max(next);

    // Checklist and acceptance criteria.
    let check_ids = |x: &Ticket| x.checklist.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
    let mut next = o.next_check.max(t.next_check);
    for c in &mut o.checklist {
        if !base_has(&check_ids, &c.id) && t.checklist.iter().any(|tc| tc.id == c.id && tc != c) {
            c.id = crate::id::checklist_id(next);
            next += 1;
            changed = true;
        }
    }
    o.next_check = o.next_check.max(next);
    let acc_ids = |x: &Ticket| {
        x.acceptance
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>()
    };
    let mut next = o.next_accept.max(t.next_accept);
    for c in &mut o.acceptance {
        if !base_has(&acc_ids, &c.id) && t.acceptance.iter().any(|tc| tc.id == c.id && tc != c) {
            c.id = crate::id::acceptance_id(next);
            next += 1;
            changed = true;
        }
    }
    o.next_accept = o.next_accept.max(next);
    changed
}

/// Renumber replies our side added under an id their side also added. Returns
/// whether anything changed.
fn renumber_replies(o: &mut Post, theirs_root: &Post, base_ids: &HashSet<String>) -> bool {
    let theirs_here = theirs_root.find(&o.id);
    let mut next = o
        .next_reply
        .max(theirs_here.map(|p| p.next_reply).unwrap_or(1));
    let mut changed = false;
    for r in &mut o.replies {
        let clash = !base_ids.contains(&r.id)
            && theirs_root.find(&r.id).is_some_and(|tp| {
                tp.body != r.body || tp.author != r.author || tp.created != r.created
            });
        if clash {
            let new = format!("{}.{next}", o.id);
            next += 1;
            rename_subtree(r, &new);
            changed = true;
        }
    }
    o.next_reply = o.next_reply.max(next);
    for r in &mut o.replies {
        changed |= renumber_replies(r, theirs_root, base_ids);
    }
    changed
}

fn rename_subtree(p: &mut Post, new: &str) {
    let old = std::mem::replace(&mut p.id, new.to_string());
    for r in &mut p.replies {
        let tail = r.id.strip_prefix(&old).unwrap_or("").to_string();
        rename_subtree(r, &format!("{new}{tail}"));
    }
}

// --- card placement ---------------------------------------------------------

/// After the merge, every ticket sits in exactly one list: a card both sides
/// moved to different lists is a conflict, and a ticket kept against a delete
/// gets its card back.
fn fix_cards(base: &Files, ours: &Files, theirs: &Files, res: &Resolutions, out: &mut Merged) {
    let parse = |bytes: Option<&Vec<u8>>| -> Option<Board> { serde_json::from_slice(bytes?).ok() };
    let Some(Some(merged_bytes)) = out.files.get("board.json") else {
        return;
    };
    let Some(mut board) = parse(Some(merged_bytes)) else {
        return;
    };
    let (ob, tb, bb) = (
        parse(ours.get("board.json")),
        parse(theirs.get("board.json")),
        parse(base.get("board.json")),
    );
    let list_of =
        |b: &Option<Board>, id: &str| b.as_ref().and_then(|b| b.locate_card(id)).map(|(l, _)| l);
    let mut changed = false;

    // Cards in more than one list.
    let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for l in &board.lists {
        for c in &l.cards {
            seen.entry(c.clone()).or_default().push(l.id.clone());
        }
    }
    for (card, lists) in seen.into_iter().filter(|(_, ls)| ls.len() > 1) {
        let (ol, tl) = (list_of(&ob, &card), list_of(&tb, &card));
        let id = format!("board.json#/cards[{card}]");
        let keep = match res.get(&id) {
            Some(Side::Ours) => ol.clone(),
            Some(Side::Theirs) => tl.clone(),
            None => {
                out.conflicts.push(Conflict {
                    id,
                    entity: card.clone(),
                    field: "list".into(),
                    base: list_of(&bb, &card)
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                    ours: ol.clone().map(Value::String).unwrap_or(Value::Null),
                    theirs: tl.clone().map(Value::String).unwrap_or(Value::Null),
                });
                tl.clone()
            }
        };
        let keep = keep.unwrap_or_else(|| lists[0].clone());
        for l in &mut board.lists {
            if l.id != keep {
                l.cards.retain(|c| c != &card);
            }
        }
        changed = true;
    }

    // Tickets that exist after the merge but lost their card, and cards whose
    // ticket the merge removed.
    for (path, content) in &out.files {
        let Some(id) = path
            .strip_prefix("tickets/")
            .and_then(|s| s.strip_suffix(".json"))
        else {
            continue;
        };
        let placed = board.locate_card(id).is_some();
        match (content.is_some(), placed) {
            (true, false) => {
                let home = list_of(&tb, id).or_else(|| list_of(&ob, id));
                if let Some(l) = home.and_then(|h| board.list_mut(&h)) {
                    l.cards.push(id.to_string());
                    changed = true;
                }
            }
            (false, true) => {
                for l in &mut board.lists {
                    l.cards.retain(|c| c != id);
                }
                changed = true;
            }
            _ => {}
        }
    }
    if changed {
        if let Ok(s) = serde_json::to_string_pretty(&board) {
            out.files
                .insert("board.json".into(), Some((s + "\n").into_bytes()));
        }
    }
}

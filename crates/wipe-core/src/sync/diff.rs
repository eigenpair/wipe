//! What changed between two board snapshots, as typed changes rather than a
//! file diff: the same list drives the CLI's status lines, the default commit
//! message, and the UI's highlighting.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::Files;
use crate::events::snippet;
use crate::model::{Board, ChecklistItem, Definitions, Thread, Ticket};

/// One change on the board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    /// What it's about: `ticket`, `thread`, `list`, `label`, `board`, `file`.
    pub scope: String,
    /// The entity: `T-003`, `F-2`, a list or label id, a file path.
    pub id: String,
    /// `created`, `deleted`, `moved`, `reordered`, `renamed`, `edited`,
    /// `commented`, `checked`, `posted`.
    pub kind: String,
    /// The ticket / thread / list title (before the change for a delete).
    pub title: String,
    /// For `moved`: the list it left; `renamed`: the old name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// For `moved`: the list it entered; `renamed`: the new name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// For `edited`: which fields (`body`, `labels`, `checklist`, ...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    /// How many (comments, posts, checked items) when more than one.
    pub count: usize,
    /// A short excerpt: the latest comment or post, the checked item.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Who made it, when the data says (comment and post authors, activity).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by: Vec<String>,
}

impl Change {
    fn new(scope: &'static str, id: &str, kind: &'static str, title: &str) -> Self {
        Change {
            scope: scope.to_string(),
            id: id.to_string(),
            kind: kind.to_string(),
            title: title.to_string(),
            from: None,
            to: None,
            fields: Vec::new(),
            count: 1,
            detail: String::new(),
            by: Vec::new(),
        }
    }

    /// One plain line: `T-003 moved todo → done`.
    pub fn line(&self) -> String {
        if self.scope == "board" {
            return match self.kind.as_str() {
                "reordered" => "lists reordered".to_string(),
                _ if self.fields.is_empty() => "board updated".to_string(),
                _ => format!("{} updated", self.fields.join(", ")),
            };
        }
        let title = if self.title.is_empty() {
            String::new()
        } else {
            format!(" \"{}\"", snippet(&self.title, 40))
        };
        let who = if self.id.is_empty() {
            String::new()
        } else {
            format!("{} ", self.id)
        };
        let what = match self.kind.as_str() {
            "moved" => format!(
                "moved {} → {}",
                self.from.as_deref().unwrap_or("?"),
                self.to.as_deref().unwrap_or("?")
            ),
            "renamed" => "renamed".to_string(),
            "edited" if !self.fields.is_empty() => format!("edited ({})", self.fields.join(", ")),
            "commented" if self.count > 1 => format!("{} comments", self.count),
            "commented" => "commented".to_string(),
            "posted" if self.count > 1 => format!("{} posts", self.count),
            "checked" if self.count > 1 => format!("{} items checked", self.count),
            k => k.to_string(),
        };
        let scope = match self.scope.as_str() {
            "list" => "list ",
            "label" => "label ",
            _ => "",
        };
        format!("{scope}{who}{what}{title}").trim().to_string()
    }
}

fn parse<T: serde::de::DeserializeOwned>(f: &Files, path: &str) -> Option<T> {
    serde_json::from_slice(f.get(path)?).ok()
}

/// The changes from `before` to `after`. Both hold every path that differs
/// between them, plus `board.json`.
pub fn diff(before: &Files, after: &Files) -> Vec<Change> {
    let mut out = Vec::new();
    let (bb, ab): (Option<Board>, Option<Board>) =
        (parse(before, "board.json"), parse(after, "board.json"));
    let list_of = |b: &Option<Board>, id: &str| b.as_ref().and_then(|b| b.locate_card(id));
    let list_name = |b: &Option<Board>, l: &str| {
        b.as_ref()
            .and_then(|b| b.list(l))
            .map(|l| l.name.clone())
            .unwrap_or_else(|| l.to_string())
    };

    // Tickets: every ticket file that changed, plus cards that moved.
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for p in before.keys().chain(after.keys()) {
        if let Some(id) = p
            .strip_prefix("tickets/")
            .and_then(|s| s.strip_suffix(".json"))
        {
            if before.get(p) != after.get(p) {
                ids.insert(id.to_string());
            }
        }
    }
    let cards = |b: &Option<Board>| -> BTreeMap<String, (String, usize)> {
        let mut m = BTreeMap::new();
        if let Some(b) = b {
            for l in &b.lists {
                for (i, c) in l.cards.iter().enumerate() {
                    m.insert(c.clone(), (l.id.clone(), i));
                }
            }
        }
        m
    };
    let (bc, ac) = (cards(&bb), cards(&ab));
    for (id, (l, _)) in &ac {
        if bc.get(id).is_some_and(|(bl, _)| bl != l) {
            ids.insert(id.clone());
        }
    }
    for id in &ids {
        let path = format!("tickets/{id}.json");
        let (bt, at): (Option<Ticket>, Option<Ticket>) =
            (parse(before, &path), parse(after, &path));
        let in_before = before.contains_key(&path);
        let in_after = after.contains_key(&path);
        match (in_before, in_after) {
            (false, true) => {
                let t = at.as_ref();
                let mut c = Change::new(
                    "ticket",
                    id,
                    "created",
                    t.map(|t| t.title.as_str()).unwrap_or(""),
                );
                c.to = list_of(&ab, id).map(|(l, _)| list_name(&ab, &l));
                if let Some(who) = t.and_then(|t| t.creator()) {
                    c.by.push(who.to_string());
                }
                out.push(c);
            }
            (true, false) => {
                let mut c = Change::new(
                    "ticket",
                    id,
                    "deleted",
                    bt.as_ref().map(|t| t.title.as_str()).unwrap_or(""),
                );
                c.from = list_of(&bb, id).map(|(l, _)| list_name(&bb, &l));
                out.push(c);
            }
            _ => {
                let title = at
                    .as_ref()
                    .or(bt.as_ref())
                    .map(|t| t.title.clone())
                    .or_else(|| ticket_title(after, id))
                    .unwrap_or_default();
                if let (Some((fl, _)), Some((tl, _))) = (bc.get(id), ac.get(id)) {
                    if fl != tl {
                        let mut c = Change::new("ticket", id, "moved", &title);
                        c.from = Some(list_name(&bb, fl));
                        c.to = Some(list_name(&ab, tl));
                        if let Some(a) = at
                            .as_ref()
                            .and_then(|t| t.activity.iter().rev().find(|a| a.kind == "moved"))
                        {
                            c.by.push(a.actor.clone());
                        }
                        out.push(c);
                    }
                }
                if let (Some(b), Some(a)) = (&bt, &at) {
                    ticket_changes(b, a, &mut out);
                }
            }
        }
    }

    // Reordered cards within a list (no other change to that ticket).
    if let (Some(b), Some(a)) = (&bb, &ab) {
        for l in &a.lists {
            let Some(bl) = b.list(&l.id) else { continue };
            let common: Vec<&String> = l.cards.iter().filter(|c| bl.cards.contains(c)).collect();
            let before_order: Vec<&String> =
                bl.cards.iter().filter(|c| l.cards.contains(c)).collect();
            if common != before_order {
                let mut c = Change::new("list", &l.id, "reordered", &l.name);
                c.count = common
                    .iter()
                    .zip(&before_order)
                    .filter(|(x, y)| x != y)
                    .count();
                out.push(c);
            }
        }
    }

    // Lists.
    if let (Some(b), Some(a)) = (&bb, &ab) {
        for l in &a.lists {
            match b.list(&l.id) {
                None => out.push(Change::new("list", &l.id, "created", &l.name)),
                Some(bl) => {
                    if bl.name != l.name {
                        let mut c = Change::new("list", &l.id, "renamed", &l.name);
                        c.from = Some(bl.name.clone());
                        c.to = Some(l.name.clone());
                        out.push(c);
                    }
                    let mut fields = Vec::new();
                    if bl.color != l.color {
                        fields.push("color".to_string());
                    }
                    if bl.wip_limit != l.wip_limit {
                        fields.push("wip limit".to_string());
                    }
                    if !fields.is_empty() {
                        let mut c = Change::new("list", &l.id, "edited", &l.name);
                        c.fields = fields;
                        out.push(c);
                    }
                }
            }
        }
        for l in &b.lists {
            if a.list(&l.id).is_none() {
                out.push(Change::new("list", &l.id, "deleted", &l.name));
            }
        }
        let order = |x: &Board| -> Vec<String> { x.lists.iter().map(|l| l.id.clone()).collect() };
        let (bo, ao) = (order(b), order(a));
        let common: Vec<&String> = ao.iter().filter(|x| bo.contains(x)).collect();
        let bcommon: Vec<&String> = bo.iter().filter(|x| ao.contains(x)).collect();
        if common != bcommon {
            out.push(Change::new("board", "", "reordered", "lists"));
        }
        let mut fields = Vec::new();
        if b.name != a.name {
            fields.push("name".to_string());
        }
        if b.description != a.description {
            fields.push("description".to_string());
        }
        if b.ids != a.ids {
            fields.push("ticket ids".to_string());
        }
        if !fields.is_empty() {
            let mut c = Change::new("board", "", "edited", &a.name);
            c.fields = fields;
            out.push(c);
        }
    }

    // Labels and priorities.
    let (bd, ad): (Option<Definitions>, Option<Definitions>) = (
        parse(before, "definitions.json"),
        parse(after, "definitions.json"),
    );
    if before.get("definitions.json") != after.get("definitions.json") {
        let empty = Definitions {
            version: 0,
            labels: Vec::new(),
            priorities: Vec::new(),
        };
        let (b, a) = (bd.as_ref().unwrap_or(&empty), ad.as_ref().unwrap_or(&empty));
        for l in &a.labels {
            match b.labels.iter().find(|x| x.name == l.name) {
                None => out.push(Change::new("label", &l.name, "created", "")),
                Some(bl) if bl != l => out.push(Change::new("label", &l.name, "edited", "")),
                _ => {}
            }
        }
        for l in &b.labels {
            if !a.labels.iter().any(|x| x.name == l.name) {
                out.push(Change::new("label", &l.name, "deleted", ""));
            }
        }
        if a.priorities != b.priorities {
            let mut c = Change::new("board", "", "edited", "");
            c.fields = vec!["priorities".into()];
            out.push(c);
        }
    }

    // Forum threads.
    for p in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        let Some(id) = p
            .strip_prefix("forum/")
            .and_then(|s| s.strip_suffix(".json"))
        else {
            continue;
        };
        if before.get(p) == after.get(p) {
            continue;
        }
        let (bt, at): (Option<Thread>, Option<Thread>) = (parse(before, p), parse(after, p));
        match (bt, at) {
            (None, Some(t)) => {
                let mut c = Change::new("thread", id, "created", &t.title);
                c.by.push(t.root.author.clone());
                c.detail = snippet(&t.root.body, 80);
                out.push(c);
                let mut replies = Vec::new();
                t.root.walk(0, &mut |p, d| {
                    if d > 0 {
                        replies.push(p.clone());
                    }
                });
                if !replies.is_empty() {
                    out.push(posts_change(id, &t.title, &replies));
                }
            }
            (Some(t), None) => out.push(Change::new("thread", id, "deleted", &t.title)),
            (Some(b), Some(a)) => thread_changes(id, &b, &a, &mut out),
            (None, None) => {}
        }
    }

    // Everything else: settings, identities, subscriptions, media.
    for p in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        if p == "board.json"
            || p == "definitions.json"
            || p.starts_with("tickets/")
            || p.starts_with("forum/")
            || before.get(p) == after.get(p)
        {
            continue;
        }
        let kind = match (before.contains_key(p), after.contains_key(p)) {
            (false, true) => "created",
            (true, false) => "deleted",
            _ => "edited",
        };
        let mut c = Change::new("file", p, kind, "");
        if let Some(name) = p.strip_suffix(".json").filter(|n| !n.contains('/')) {
            c.scope = "board".into();
            c.id = String::new();
            c.kind = "edited".into();
            c.fields = vec![name.to_string()];
        }
        out.push(c);
    }
    out
}

fn ticket_title(f: &Files, id: &str) -> Option<String> {
    parse::<Ticket>(f, &format!("tickets/{id}.json")).map(|t| t.title)
}

fn ticket_changes(b: &Ticket, a: &Ticket, out: &mut Vec<Change>) {
    let id = a.id.as_str();
    if b.title != a.title {
        let mut c = Change::new("ticket", id, "renamed", &a.title);
        c.from = Some(b.title.clone());
        c.to = Some(a.title.clone());
        out.push(c);
    }
    let mut fields: Vec<String> = Vec::new();
    let mut field = |name: &str, changed: bool| {
        if changed {
            fields.push(name.to_string());
        }
    };
    field("body", b.body != a.body);
    field("priority", b.priority != a.priority);
    field("labels", b.labels != a.labels);
    field("assignees", b.assignees != a.assignees);
    field("relations", b.relations != a.relations);
    field("attachments", b.attachments != a.attachments);
    let items_changed = |x: &[ChecklistItem], y: &[ChecklistItem]| {
        let strip = |v: &[ChecklistItem]| {
            v.iter()
                .map(|i| (i.id.clone(), i.text.clone()))
                .collect::<Vec<_>>()
        };
        strip(x) != strip(y)
    };
    field("checklist", items_changed(&b.checklist, &a.checklist));
    field("criteria", items_changed(&b.acceptance, &a.acceptance));
    let edited_comments = a
        .comments
        .iter()
        .filter(|c| b.comments.iter().any(|bc| bc.id == c.id && bc != *c))
        .count();
    field(
        "comments",
        edited_comments > 0 || a.comments.len() < b.comments.len(),
    );
    if !fields.is_empty() {
        let mut c = Change::new("ticket", id, "edited", &a.title);
        c.fields = fields;
        let mut by: Vec<String> = Vec::new();
        for act in a.activity.iter().skip(b.activity.len()) {
            if !by.contains(&act.actor) {
                by.push(act.actor.clone());
            }
        }
        c.by = by;
        out.push(c);
    }

    // New comments.
    let added: Vec<_> = a
        .comments
        .iter()
        .filter(|c| !b.comments.iter().any(|bc| bc.id == c.id))
        .collect();
    if let Some(last) = added.last() {
        let mut c = Change::new("ticket", id, "commented", &a.title);
        c.count = added.len();
        c.detail = snippet(&last.body, 80);
        for x in &added {
            if !c.by.contains(&x.author) {
                c.by.push(x.author.clone());
            }
        }
        out.push(c);
    }

    // Checked / unchecked items.
    let toggled: Vec<&ChecklistItem> = a
        .checklist
        .iter()
        .chain(&a.acceptance)
        .filter(|i| {
            b.checklist
                .iter()
                .chain(&b.acceptance)
                .any(|bi| bi.id == i.id && bi.done != i.done)
        })
        .collect();
    if let Some(last) = toggled.last() {
        let mut c = Change::new("ticket", id, "checked", &a.title);
        c.count = toggled.len();
        c.detail = format!(
            "[{}] {}",
            if last.done { "x" } else { " " },
            snippet(&last.text, 60)
        );
        out.push(c);
    }
}

fn posts_change(id: &str, title: &str, posts: &[crate::model::Post]) -> Change {
    let mut c = Change::new("thread", id, "posted", title);
    c.count = posts.len();
    if let Some(last) = posts.last() {
        c.detail = snippet(&last.body, 80);
    }
    for p in posts {
        if !c.by.contains(&p.author) {
            c.by.push(p.author.clone());
        }
    }
    c
}

fn thread_changes(id: &str, b: &Thread, a: &Thread, out: &mut Vec<Change>) {
    if b.title != a.title {
        let mut c = Change::new("thread", id, "renamed", &a.title);
        c.from = Some(b.title.clone());
        c.to = Some(a.title.clone());
        out.push(c);
    }
    let mut before: BTreeMap<String, crate::model::Post> = BTreeMap::new();
    b.root.walk(0, &mut |p, _| {
        before.insert(p.id.clone(), p.clone());
    });
    let mut added = Vec::new();
    let mut edited = 0;
    let mut after_ids = BTreeSet::new();
    a.root.walk(0, &mut |p, _| {
        after_ids.insert(p.id.clone());
        match before.get(&p.id) {
            None => added.push(p.clone()),
            Some(bp) => {
                let same = bp.body == p.body
                    && bp.labels == p.labels
                    && bp.attachments == p.attachments
                    && bp.refs == p.refs;
                if !same {
                    edited += 1;
                }
            }
        }
    });
    if !added.is_empty() {
        out.push(posts_change(id, &a.title, &added));
    }
    let removed = before.keys().filter(|k| !after_ids.contains(*k)).count();
    if edited > 0 || removed > 0 {
        let mut c = Change::new("thread", id, "edited", &a.title);
        if edited > 0 {
            c.fields.push(format!("{edited} post(s) edited"));
        }
        if removed > 0 {
            c.fields.push(format!("{removed} post(s) removed"));
        }
        out.push(c);
    }
}

/// A one-line summary of `changes` for a commit subject.
pub fn summary(changes: &[Change], max: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in changes {
        let s = c.line();
        let s = match s.find(" \"") {
            Some(i) => s[..i].to_string(),
            None => s,
        };
        parts.push(s);
    }
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        let sep = if out.is_empty() { "" } else { ", " };
        if out.len() + sep.len() + p.len() > max {
            out.push_str(&format!(" (+{} more)", parts.len() - i));
            break;
        }
        out.push_str(sep);
        out.push_str(p);
    }
    out
}

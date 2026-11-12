//! High-level, transactional board operations shared by every front-end
//! (CLI, daemon, desktop). Each function loads what it needs through [`Store`],
//! mutates the in-memory model, and writes it back deterministically. Keeping
//! these here means the mutation rules live in exactly one place.

use std::fs;

use chrono::{DateTime, Utc};

use crate::error::{Error, Result};
use crate::git;
use crate::id::slug;
use crate::model::{
    next_label_color, Attachment, AttachmentSource, Board, ChecklistItem, Identity, IdentityKind,
    LabelDef, List, OriginalNote, Relation, RelationKind, Ticket,
};
use crate::store::Store;

/// Specification for a new ticket. Only `title` is required.
#[derive(Debug, Default, Clone)]
pub struct NewTicket {
    /// Short title.
    pub title: String,
    /// Optional long-form body.
    pub body: Option<String>,
    /// Optional priority.
    pub priority: Option<String>,
    /// Target list ID; defaults to the board's first list.
    pub list: Option<String>,
    /// Labels to apply.
    pub labels: Vec<String>,
    /// Assignees.
    pub assignees: Vec<String>,
}

/// Create a ticket, allocate its ID, place it on a list, and persist both the
/// ticket file and the board. Returns the created ticket.
pub fn create_ticket(
    store: &Store,
    spec: NewTicket,
    actor: &str,
    now: DateTime<Utc>,
) -> Result<Ticket> {
    let mut board = store.load_board()?;

    let list_id = match spec.list {
        Some(l) => {
            if board.list(&l).is_none() {
                return Err(Error::ListNotFound(l));
            }
            l
        }
        None => board
            .lists
            .first()
            .map(|l| l.id.clone())
            .ok_or_else(|| Error::msg("board has no lists"))?,
    };

    let id = board.ticket_id(board.next_ticket);
    board.next_ticket += 1;

    let mut ticket = Ticket::new(id.clone(), spec.title, now);
    ticket.body = spec.body.unwrap_or_default();
    ticket.priority = spec.priority;
    ticket.labels = spec.labels;
    ticket.assignees = spec.assignees;
    ticket.log_activity(actor, "created", "", now);

    board
        .list_mut(&list_id)
        .expect("checked above")
        .cards
        .push(id.clone());
    // Card-only change: `board.updated` is left alone so concurrent card
    // moves in different worktrees do not all conflict on that one line.

    store.save_ticket(&ticket)?;
    store.save_board(&board)?;
    Ok(ticket)
}

/// Duplicate a ticket: create a copy on the same list, immediately after the
/// original. The copy carries the title (suffixed " (copy)"), body, priority,
/// labels, assignees, checklist, acceptance criteria, and attachment references;
/// comments and activity start fresh (a single `created` entry). Returns the copy.
pub fn duplicate_ticket(
    store: &Store,
    ticket_id: &str,
    actor: &str,
    now: DateTime<Utc>,
) -> Result<Ticket> {
    let ticket_id = &store.resolve_ticket_id(ticket_id)?;
    let src = store.load_ticket(ticket_id)?;
    let mut board = store.load_board()?;

    let new_id = board.ticket_id(board.next_ticket);
    board.next_ticket += 1;

    let mut copy = src.clone();
    copy.id = new_id.clone();
    copy.legacy_id = None;
    copy.uid = Some(crate::id::uid());
    copy.title = format!("{} (copy)", src.title);
    copy.comments.clear();
    copy.activity.clear();
    copy.next_comment = 1;
    copy.created = now;
    copy.updated = now;
    copy.log_activity(actor, "created", "", now);

    // Insert directly after the original on its list (or append to the first list
    // if the original card isn't found for some reason).
    let (list_id, at) = board
        .lists
        .iter()
        .find_map(|l| {
            l.cards
                .iter()
                .position(|c| c == ticket_id)
                .map(|p| (l.id.clone(), p + 1))
        })
        .or_else(|| board.lists.first().map(|l| (l.id.clone(), l.cards.len())))
        .ok_or_else(|| Error::msg("board has no lists"))?;
    let dest = board.list_mut(&list_id).expect("list id from board");
    let at = at.min(dest.cards.len());
    dest.cards.insert(at, new_id.clone());
    // Card-only change: `board.updated` is left alone so concurrent card
    // moves in different worktrees do not all conflict on that one line.

    store.save_ticket(&copy)?;
    store.save_board(&board)?;
    Ok(copy)
}

/// Move a ticket to `to_list` at an optional 0-based `position` (appended if
/// `None`). Removes it from whatever list currently holds it.
pub fn move_ticket(
    store: &Store,
    ticket_id: &str,
    to_list: &str,
    position: Option<usize>,
    actor: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    let ticket_id = &store.resolve_ticket_id(ticket_id)?;
    // Ensure the ticket exists.
    let _ = store.load_ticket(ticket_id)?;
    let mut board = store.load_board()?;
    let dest_name = match board.list(to_list) {
        Some(l) => l.name.clone(),
        None => return Err(Error::ListNotFound(to_list.to_string())),
    };

    // Where is it now? (skip logging a no-op move within the same list)
    let from_list = board
        .lists
        .iter()
        .find(|l| l.cards.iter().any(|c| c == ticket_id))
        .map(|l| l.id.clone());

    // Remove from current list (if present).
    for list in &mut board.lists {
        list.cards.retain(|c| c != ticket_id);
    }

    let dest = board.list_mut(to_list).expect("checked above");
    let pos = position.unwrap_or(dest.cards.len()).min(dest.cards.len());
    dest.cards.insert(pos, ticket_id.to_string());
    // Card-only change: `board.updated` is left alone so concurrent card
    // moves in different worktrees do not all conflict on that one line.

    // Touch the ticket so its own `updated` reflects the move; log it as activity
    // only when the containing list actually changed.
    let mut ticket = store.load_ticket(ticket_id)?;
    ticket.updated = now;
    if from_list.as_deref() != Some(to_list) {
        ticket.log_activity(actor, "moved", dest_name, now);
    }
    store.save_ticket(&ticket)?;
    store.save_board(&board)?;
    Ok(())
}

/// Delete a ticket file and remove its card from the board.
pub fn delete_ticket(store: &Store, ticket_id: &str, _now: DateTime<Utc>) -> Result<()> {
    let ticket_id = &store.resolve_ticket_id(ticket_id)?;
    store.delete_ticket(ticket_id)?; // errors if missing
    let mut board = store.load_board()?;
    for list in &mut board.lists {
        list.cards.retain(|c| c != ticket_id);
    }
    // Card-only change: `board.updated` is left alone so concurrent card
    // moves in different worktrees do not all conflict on that one line.
    store.save_board(&board)?;
    Ok(())
}

/// Append a comment to a ticket. Returns the new comment ID.
pub fn add_comment(
    store: &Store,
    ticket_id: &str,
    author: &str,
    body: &str,
    now: DateTime<Utc>,
) -> Result<String> {
    let mut ticket = store.load_ticket(ticket_id)?;
    let id = ticket.add_comment(author, body, now);
    store.save_ticket(&ticket)?;
    Ok(id)
}

/// Delete a comment from a ticket by its ID. Errors if the comment is absent.
/// `next_comment` is deliberately NOT decremented, so IDs are never reused.
pub fn delete_comment(
    store: &Store,
    ticket_id: &str,
    comment_id: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    let mut ticket = store.load_ticket(ticket_id)?;
    let before = ticket.comments.len();
    ticket.comments.retain(|c| c.id != comment_id);
    if ticket.comments.len() == before {
        return Err(Error::msg(format!("comment `{comment_id}` not found")));
    }
    ticket.updated = now;
    store.save_ticket(&ticket)?;
    Ok(())
}

/// Edit a comment's body in place. Sets the `edited` timestamp so the change is
/// visible. Errors if the comment is absent.
pub fn edit_comment(
    store: &Store,
    ticket_id: &str,
    comment_id: &str,
    body: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    let mut ticket = store.load_ticket(ticket_id)?;
    let c = ticket
        .comments
        .iter_mut()
        .find(|c| c.id == comment_id)
        .ok_or_else(|| Error::msg(format!("comment `{comment_id}` not found")))?;
    c.body = body.to_string();
    c.edited = Some(now);
    ticket.updated = now;
    store.save_ticket(&ticket)?;
    Ok(())
}

/// Reattribute a comment to `new_author`, recording an **audit** activity entry
/// (`reattributed`, `<c-id>: <old> -> <new>`) rather than silently overwriting -
/// so a misattributed comment (e.g. from an identity stomp) can be corrected with a
/// trail. Returns the previous author.
pub fn reattribute_comment(
    store: &Store,
    ticket_id: &str,
    comment_id: &str,
    new_author: &str,
    actor: &str,
    now: DateTime<Utc>,
) -> Result<String> {
    let mut ticket = store.load_ticket(ticket_id)?;
    let old = {
        let c = ticket
            .comments
            .iter_mut()
            .find(|c| c.id == comment_id)
            .ok_or_else(|| Error::msg(format!("comment `{comment_id}` not found")))?;
        let old = c.author.clone();
        c.author = new_author.to_string();
        c.edited = Some(now);
        old
    };
    ticket.log_activity(
        actor,
        "reattributed",
        format!("{comment_id}: {old} -> {new_author}"),
        now,
    );
    ticket.updated = now;
    store.save_ticket(&ticket)?;
    Ok(old)
}

/// Reattribute a ticket's *creation* to `new_author` (rewrites the `created`
/// activity actor), recording an audit `reattributed` entry. Corrects a ticket
/// created under a stomped identity. Returns the previous creator (if any).
pub fn reattribute_ticket(
    store: &Store,
    ticket_id: &str,
    new_author: &str,
    actor: &str,
    now: DateTime<Utc>,
) -> Result<Option<String>> {
    let mut ticket = store.load_ticket(ticket_id)?;
    let old = ticket
        .activity
        .iter()
        .find(|a| a.kind == "created")
        .map(|a| a.actor.clone());
    for a in ticket.activity.iter_mut().filter(|a| a.kind == "created") {
        a.actor = new_author.to_string();
    }
    ticket.log_activity(
        actor,
        "reattributed",
        format!("created: {} -> {new_author}", old.as_deref().unwrap_or("?")),
        now,
    );
    ticket.updated = now;
    store.save_ticket(&ticket)?;
    Ok(old)
}

// --- checklist & acceptance criteria ------------------------------------------
//
// The two tickable surfaces on a ticket share one item shape and identical
// operations; [`Checks`] selects which list (and ID namespace) a call targets.

/// Which tickable list on a ticket an operation targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checks {
    /// The worker-facing to-do checklist (`ck-<n>` items).
    Checklist,
    /// The reviewer-facing acceptance criteria (`ac-<n>` items).
    Acceptance,
}

impl Checks {
    fn items(self, t: &mut Ticket) -> &mut Vec<ChecklistItem> {
        match self {
            Checks::Checklist => &mut t.checklist,
            Checks::Acceptance => &mut t.acceptance,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Checks::Checklist => "checklist item",
            Checks::Acceptance => "acceptance criterion",
        }
    }

    fn append(self, t: &mut Ticket, text: &str, now: DateTime<Utc>) -> String {
        match self {
            Checks::Checklist => t.add_checklist_item(text, now),
            Checks::Acceptance => t.add_acceptance_item(text, now),
        }
    }
}

fn checks_item_mut<'t>(
    kind: Checks,
    ticket: &'t mut Ticket,
    item_id: &str,
) -> Result<&'t mut ChecklistItem> {
    kind.items(ticket)
        .iter_mut()
        .find(|i| i.id == item_id)
        .ok_or_else(|| Error::msg(format!("{} `{item_id}` not found", kind.noun())))
}

/// Append an item to a ticket's checklist or acceptance criteria. Returns the
/// new item's ID.
pub fn checks_add(
    store: &Store,
    kind: Checks,
    ticket_id: &str,
    text: &str,
    now: DateTime<Utc>,
) -> Result<String> {
    let mut ticket = store.load_ticket(ticket_id)?;
    let id = kind.append(&mut ticket, text, now);
    store.save_ticket(&ticket)?;
    Ok(id)
}

/// Set an item's checked state. `done = None` toggles it. Returns the
/// resulting state. A change is logged as `checked` / `unchecked` activity.
pub fn checks_set(
    store: &Store,
    kind: Checks,
    ticket_id: &str,
    item_id: &str,
    done: Option<bool>,
    actor: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let mut ticket = store.load_ticket(ticket_id)?;
    let item = checks_item_mut(kind, &mut ticket, item_id)?;
    let was = item.done;
    item.done = done.unwrap_or(!item.done);
    let state = item.done;
    let detail = format!("{} {}", item.id, item.text);
    if state != was {
        let verb = if state { "checked" } else { "unchecked" };
        ticket.log_activity(actor, verb, detail, now);
    }
    ticket.updated = now;
    store.save_ticket(&ticket)?;
    Ok(state)
}

/// Edit an item's text.
pub fn checks_edit(
    store: &Store,
    kind: Checks,
    ticket_id: &str,
    item_id: &str,
    text: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    let mut ticket = store.load_ticket(ticket_id)?;
    checks_item_mut(kind, &mut ticket, item_id)?.text = text.to_string();
    ticket.updated = now;
    store.save_ticket(&ticket)?;
    Ok(())
}

/// Remove an item.
pub fn checks_remove(
    store: &Store,
    kind: Checks,
    ticket_id: &str,
    item_id: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    let mut ticket = store.load_ticket(ticket_id)?;
    let items = kind.items(&mut ticket);
    let before = items.len();
    items.retain(|i| i.id != item_id);
    if items.len() == before {
        return Err(Error::msg(format!("{} `{item_id}` not found", kind.noun())));
    }
    ticket.updated = now;
    store.save_ticket(&ticket)?;
    Ok(())
}

/// Move an item to a new 0-based position (clamped to the list length).
pub fn checks_move(
    store: &Store,
    kind: Checks,
    ticket_id: &str,
    item_id: &str,
    index: usize,
    now: DateTime<Utc>,
) -> Result<()> {
    let mut ticket = store.load_ticket(ticket_id)?;
    let items = kind.items(&mut ticket);
    let from = items
        .iter()
        .position(|i| i.id == item_id)
        .ok_or_else(|| Error::msg(format!("{} `{item_id}` not found", kind.noun())))?;
    let item = items.remove(from);
    let to = index.min(items.len());
    items.insert(to, item);
    ticket.updated = now;
    store.save_ticket(&ticket)?;
    Ok(())
}

/// Add a new list to the end of the board. Returns the new list's ID.
pub fn add_list(store: &Store, name: &str, now: DateTime<Utc>) -> Result<String> {
    let mut board = store.load_board()?;
    let mut id = slug(name);
    // Ensure the slug is unique.
    if board.list(&id).is_some() {
        let mut n = 2;
        loop {
            let candidate = format!("{id}-{n}");
            if board.list(&candidate).is_none() {
                id = candidate;
                break;
            }
            n += 1;
        }
    }
    let mut list = List::new(name);
    list.id = id.clone();
    board.lists.push(list);
    board.updated = now;
    store.save_board(&board)?;
    Ok(id)
}

/// Remove a list. Fails if the list still holds cards, unless `force` is set (in
/// which case the contained tickets are also deleted).
pub fn remove_list(store: &Store, list_id: &str, force: bool, now: DateTime<Utc>) -> Result<()> {
    let mut board = store.load_board()?;
    let idx = board
        .lists
        .iter()
        .position(|l| l.id == list_id)
        .ok_or_else(|| Error::ListNotFound(list_id.to_string()))?;

    let cards = board.lists[idx].cards.clone();
    if !cards.is_empty() && !force {
        return Err(Error::msg(format!(
            "list `{list_id}` still holds {} ticket(s); pass --force to delete them too",
            cards.len()
        )));
    }
    for id in cards {
        // Ignore missing files; the goal state is "gone".
        let _ = store.delete_ticket(&id);
    }
    board.lists.remove(idx);
    board.updated = now;
    store.save_board(&board)?;
    Ok(())
}

/// Reorder a list to a new 0-based index.
pub fn move_list(store: &Store, list_id: &str, to_index: usize, now: DateTime<Utc>) -> Result<()> {
    let mut board = store.load_board()?;
    let from = board
        .lists
        .iter()
        .position(|l| l.id == list_id)
        .ok_or_else(|| Error::ListNotFound(list_id.to_string()))?;
    let list = board.lists.remove(from);
    let to = to_index.min(board.lists.len());
    board.lists.insert(to, list);
    board.updated = now;
    store.save_board(&board)?;
    Ok(())
}

/// Rename a list's display name (its ID stays stable).
pub fn rename_list(store: &Store, list_id: &str, name: &str, now: DateTime<Utc>) -> Result<()> {
    let mut board = store.load_board()?;
    let list = board
        .list_mut(list_id)
        .ok_or_else(|| Error::ListNotFound(list_id.to_string()))?;
    list.name = name.to_string();
    board.updated = now;
    store.save_board(&board)?;
    Ok(())
}

/// One list's ID paired with the tickets currently on it, in card order.
pub type ListView = (String, Vec<Ticket>);

/// Load the whole board as an ordered sequence of `(list_id, tickets)`.
pub fn board_view(store: &Store) -> Result<(Board, Vec<ListView>)> {
    let board = store.load_board()?;
    let mut out = Vec::with_capacity(board.lists.len());
    for list in &board.lists {
        let mut tickets = Vec::with_capacity(list.cards.len());
        for id in &list.cards {
            // A dangling card is skipped quietly; a ticket file that exists but
            // can't be read (a merge conflict) is skipped with a warning, so one
            // broken file never hides the whole board - nor itself.
            match store.load_ticket(id) {
                Ok(t) => tickets.push(t),
                Err(Error::TicketNotFound(_)) => {}
                Err(e) => eprintln!("warning: skipping {id}: {e}"),
            }
        }
        out.push((list.id.clone(), tickets));
    }
    Ok((board, out))
}

/// A partial update to a ticket. `None` fields are left unchanged. For `priority`,
/// an inner `Some(None)` clears the value.
#[derive(Debug, Default, Clone)]
pub struct TicketPatch {
    /// New title.
    pub title: Option<String>,
    /// New body.
    pub body: Option<String>,
    /// Set/clear the priority.
    pub priority: Option<Option<String>>,
    /// Replace the label set.
    pub labels: Option<Vec<String>>,
    /// Replace the assignee set.
    pub assignees: Option<Vec<String>>,
}

/// Apply a [`TicketPatch`] and persist. Returns the updated ticket.
pub fn update_ticket(
    store: &Store,
    id: &str,
    patch: TicketPatch,
    actor: &str,
    now: DateTime<Utc>,
) -> Result<Ticket> {
    let mut t = store.load_ticket(id)?;
    // The first time someone other than the creator rewrites the title or body,
    // freeze the creator's own wording so it survives the rewrite.
    let rewrites = patch.title.as_ref().is_some_and(|v| *v != t.title)
        || patch.body.as_ref().is_some_and(|v| *v != t.body);
    if rewrites && t.original.is_none() {
        if let Some(creator) = t.creator().map(str::to_string) {
            if creator != actor {
                t.original = Some(OriginalNote {
                    title: t.title.clone(),
                    body: t.body.clone(),
                    author: creator,
                    preserved: now,
                    rewritten_by: actor.to_string(),
                });
            }
        }
    }
    if let Some(v) = patch.title {
        if v != t.title {
            t.log_activity(actor, "renamed", v.clone(), now);
        }
        t.title = v;
    }
    if let Some(v) = patch.body {
        if v != t.body {
            t.log_activity(actor, "edited", "", now);
        }
        t.body = v;
    }
    if let Some(v) = patch.priority {
        if v != t.priority {
            t.log_activity(actor, "priority", v.clone().unwrap_or_default(), now);
        }
        t.priority = v;
    }
    if let Some(v) = patch.labels {
        let added: Vec<String> = v
            .iter()
            .filter(|l| !t.labels.contains(l))
            .cloned()
            .collect();
        let removed: Vec<String> = t
            .labels
            .iter()
            .filter(|l| !v.contains(l))
            .cloned()
            .collect();
        for l in added {
            t.log_activity(actor, "label-added", l, now);
        }
        for l in removed {
            t.log_activity(actor, "label-removed", l, now);
        }
        t.labels = v;
    }
    if let Some(v) = patch.assignees {
        let added: Vec<String> = v
            .iter()
            .filter(|a| !t.assignees.contains(a))
            .cloned()
            .collect();
        let removed: Vec<String> = t
            .assignees
            .iter()
            .filter(|a| !v.contains(a))
            .cloned()
            .collect();
        for a in added {
            t.log_activity(actor, "assigned", a, now);
        }
        for a in removed {
            t.log_activity(actor, "unassigned", a, now);
        }
        t.assignees = v;
    }
    t.updated = now;
    store.save_ticket(&t)?;
    Ok(t)
}

/// Record that `ticket_id` is blocked by `blocker` (stored once, as a
/// `blocked-by` relation on the waiting ticket). Rejects unknown tickets, a
/// ticket blocking itself, and any link that would close a dependency cycle.
/// Returns false when the link already existed.
pub fn add_blocker(
    store: &Store,
    ticket_id: &str,
    blocker: &str,
    actor: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let ticket_id = &store.resolve_ticket_id(ticket_id)?;
    let blocker = &store.resolve_ticket_id(blocker)?;
    if ticket_id == blocker {
        return Err(Error::msg(format!("{ticket_id} cannot block itself")));
    }
    let mut t = store.load_ticket(ticket_id)?;
    store.load_ticket(blocker)?;
    if t.blocked_by().any(|b| b == blocker) {
        return Ok(false);
    }
    // Would `blocker` (transitively) wait on `ticket_id`? Then this closes a cycle.
    let mut stack = vec![blocker.to_string()];
    let mut seen = std::collections::HashSet::new();
    while let Some(cur) = stack.pop() {
        if &cur == ticket_id {
            return Err(Error::msg(format!(
                "{blocker} already waits on {ticket_id} (directly or through other tickets) - \
                 blocking {ticket_id} by it would create a dependency cycle"
            )));
        }
        if seen.insert(cur.clone()) {
            if let Ok(next) = store.load_ticket(&cur) {
                stack.extend(next.blocked_by().map(str::to_string));
            }
        }
    }
    t.relations.push(Relation {
        kind: RelationKind::BlockedBy,
        target: blocker.to_string(),
    });
    t.log_activity(actor, "blocked-by", blocker, now);
    t.updated = now;
    store.save_ticket(&t)?;
    Ok(true)
}

/// Remove a `blocked-by` link. Returns false when there was none.
pub fn remove_blocker(
    store: &Store,
    ticket_id: &str,
    blocker: &str,
    actor: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let ticket_id = &store.resolve_ticket_id(ticket_id)?;
    // A blocker that no longer exists can still be unlinked by its stored id.
    let resolved = store.resolve_ticket_id(blocker).ok();
    let blocker = resolved.as_deref().unwrap_or(blocker);
    let mut t = store.load_ticket(ticket_id)?;
    let before = t.relations.len();
    t.relations
        .retain(|r| !(r.kind == RelationKind::BlockedBy && r.target == blocker));
    if t.relations.len() == before {
        return Ok(false);
    }
    t.log_activity(actor, "unblocked", blocker, now);
    t.updated = now;
    store.save_ticket(&t)?;
    Ok(true)
}

/// The blockers of `t` that are still open: not on `done_list` and not gone
/// from the board. A ticket with none is "ready".
pub fn open_blockers(t: &Ticket, board: &Board, done_list: &str) -> Vec<String> {
    t.blocked_by()
        .filter(|b| match board.locate_card(b) {
            Some((list, _)) => list != done_list,
            // Deleted/trashed blockers no longer hold anything up.
            None => false,
        })
        .map(str::to_string)
        .collect()
}

/// Resolve the board's workflow lists: `(review, done, rework)`. Configured
/// settings win when they name an existing list; otherwise they are detected by
/// convention. `review` is `None` when the board has no review-like list.
pub fn workflow_lists(
    board: &Board,
    settings: &crate::model::Settings,
) -> (Option<String>, Option<String>, Option<String>) {
    let exists = |id: &Option<String>| id.clone().filter(|i| board.list(i).is_some());
    let review = exists(&settings.review_list).or_else(|| {
        board
            .lists
            .iter()
            .find(|l| l.id.contains("review") || l.name.to_ascii_lowercase().contains("review"))
            .map(|l| l.id.clone())
    });
    let done = exists(&settings.done_list).or_else(|| {
        board
            .list("done")
            .map(|l| l.id.clone())
            .or_else(|| board.lists.last().map(|l| l.id.clone()))
    });
    let rework = exists(&settings.rework_list).or_else(|| {
        board
            .list("todo")
            .map(|l| l.id.clone())
            .or_else(|| board.lists.first().map(|l| l.id.clone()))
    });
    (review, done, rework)
}

/// Create a label. If `color` is `None`, auto-pick the first unused palette color.
pub fn create_label(
    store: &Store,
    name: &str,
    color: Option<String>,
    description: Option<String>,
) -> Result<LabelDef> {
    let mut defs = store.load_definitions()?;
    if defs.labels.iter().any(|l| l.name == name) {
        return Err(Error::msg(format!("label `{name}` already exists")));
    }
    let color = color.or_else(|| Some(next_label_color(&defs.labels)));
    let label = LabelDef {
        name: name.to_string(),
        color,
        description: description.unwrap_or_default(),
    };
    defs.labels.push(label.clone());
    store.save_definitions(&defs)?;
    Ok(label)
}

/// Set a label's color. Errors if the label is not defined.
pub fn set_label_color(store: &Store, name: &str, color: &str) -> Result<LabelDef> {
    let mut defs = store.load_definitions()?;
    let label = defs
        .labels
        .iter_mut()
        .find(|l| l.name == name)
        .ok_or_else(|| Error::msg(format!("label `{name}` not found")))?;
    label.color = Some(color.to_string());
    let out = label.clone();
    store.save_definitions(&defs)?;
    Ok(out)
}

/// Delete a label definition and strip it from every ticket.
pub fn delete_label(store: &Store, name: &str, now: DateTime<Utc>) -> Result<()> {
    let mut defs = store.load_definitions()?;
    defs.labels.retain(|l| l.name != name);
    store.save_definitions(&defs)?;
    for id in store.ticket_ids()? {
        let mut t = store.load_ticket(&id)?;
        if t.labels.iter().any(|l| l == name) {
            t.labels.retain(|l| l != name);
            t.updated = now;
            store.save_ticket(&t)?;
        }
    }
    Ok(())
}

/// List identities: the saved registry merged with human authors discovered from
/// the git history so contributors show up without manual
/// setup. Ids are unique (see [`normalize_identities`]).
pub fn list_identities(store: &Store) -> Result<Vec<Identity>> {
    let mut registry = store.load_identities()?;
    normalize_identities(&mut registry);
    // The people in the history, and whoever works here now (before their
    // first commit), so they can be assigned and mentioned.
    let mut people = crate::vcs::authors(store.root());
    if let Some((name, key)) = account(store) {
        people.push((name, key));
    }
    for (name, email) in people {
        if !registry.iter().any(|i| i.id.eq_ignore_ascii_case(&email)) {
            registry.push(Identity {
                id: email,
                display_name: name,
                kind: IdentityKind::Human,
                owner: None,
            });
        }
    }
    Ok(registry)
}

/// The person working in this repository, from git: `(name,
/// key)`, the key being their email (the id of their human identity) or, with
/// no email configured, their name. Agents act for this account.
pub fn account(store: &Store) -> Option<(String, String)> {
    let raw = crate::vcs::identity(store.root())?;
    Some(split_account(&raw))
}

/// `Name <email>` -> `(Name, email)`; a bare value is both.
pub fn split_account(raw: &str) -> (String, String) {
    match raw.trim().rsplit_once('<') {
        Some((name, rest)) => {
            let email = rest.trim_end_matches('>').trim().to_string();
            let name = name.trim();
            let name = if name.is_empty() {
                email.clone()
            } else {
                name.to_string()
            };
            (name, email)
        }
        None => (raw.trim().to_string(), raw.trim().to_string()),
    }
}

/// Make identity ids unique, case-insensitively: entries for one id that agree
/// (same kind and owner) are one identity; a clashing one (e.g. two people's
/// agents both called `claude`, met in a merge) is renamed with a numbered
/// suffix - `claude02`. Returns whether anything changed.
pub fn normalize_identities(list: &mut Vec<Identity>) -> bool {
    let mut out: Vec<Identity> = Vec::with_capacity(list.len());
    let mut changed = false;
    for i in std::mem::take(list) {
        match out.iter_mut().find(|o| o.id.eq_ignore_ascii_case(&i.id)) {
            None => out.push(i),
            Some(o) if o.kind == i.kind && (o.owner == i.owner || i.owner.is_none()) => {
                changed = true;
            }
            Some(o) if o.kind == i.kind && o.owner.is_none() => {
                o.owner = i.owner;
                changed = true;
            }
            Some(_) => {
                let mut i = i;
                i.id = free_id(&out, &i.id);
                out.push(i);
                changed = true;
            }
        }
    }
    *list = out;
    changed
}

/// `base` with the first free two-digit suffix: `claude02`, `claude03`, ...
fn free_id(list: &[Identity], base: &str) -> String {
    (2..)
        .map(|n| format!("{base}{n:02}"))
        .find(|c| !list.iter().any(|i| i.id.eq_ignore_ascii_case(c)))
        .expect("an unbounded range has a free id")
}

/// Whether `id` is `base` plus a two-digit suffix (`claude02` of `claude`).
fn is_variant(id: &str, base: &str) -> bool {
    id.to_ascii_lowercase()
        .strip_prefix(&base.to_ascii_lowercase())
        .is_some_and(|n| n.len() == 2 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// How [`claim_agent`] settled a requested agent id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// An existing identity of this account (or an unowned one, now claimed).
    Existing,
    /// A new identity was registered.
    Created,
    /// The id belongs to another person's agent: this account's own variant
    /// (`claude02`) is used instead.
    Variant,
}

/// Choose the agent identity `requested` for an agent working for `owner` (the
/// account key, see [`account`]). Agents always belong to a person: an existing
/// agent of `owner`'s (or an unowned one, which `owner` claims) is reused as is;
/// a name another person's agent holds resolves to `owner`'s own numbered
/// variant (`claude02`), created on first use. A person's identity is never
/// handed to an agent.
pub fn claim_agent(
    store: &Store,
    requested: &str,
    display_name: Option<&str>,
    owner: Option<&str>,
) -> Result<(Identity, Claim)> {
    let requested = requested.trim();
    if requested.is_empty() {
        return Err(Error::msg("an identity id cannot be empty"));
    }
    let owner = owner.map(str::trim).filter(|o| !o.is_empty());
    let display_name = display_name.map(str::trim).filter(|n| !n.is_empty());
    let mut registry = store.load_identities()?;
    let mut changed = normalize_identities(&mut registry);
    let human = registry
        .iter()
        .any(|i| i.id.eq_ignore_ascii_case(requested) && i.kind == IdentityKind::Human)
        || crate::vcs::authors(store.root())
            .iter()
            .any(|(_, e)| e.eq_ignore_ascii_case(requested));
    if human {
        return Err(Error::msg(format!(
            "`{requested}` is a person's identity - an agent needs its own (e.g. `claude`)"
        )));
    }
    let mine = |i: &Identity| i.owner.is_none() || owner.is_none() || i.owner.as_deref() == owner;
    let (ident, claim) = match registry
        .iter()
        .position(|i| i.id.eq_ignore_ascii_case(requested))
    {
        Some(at) if mine(&registry[at]) => {
            let i = &mut registry[at];
            if i.owner.is_none() && owner.is_some() {
                i.owner = owner.map(str::to_string);
                changed = true;
            }
            if let Some(n) = display_name.filter(|n| *n != i.display_name) {
                i.display_name = n.to_string();
                changed = true;
            }
            (i.clone(), Claim::Existing)
        }
        Some(_) => {
            // Another person's agent: this account's own variant of the name.
            match registry.iter().find(|i| {
                i.kind == IdentityKind::Agent
                    && i.owner.as_deref() == owner
                    && is_variant(&i.id, requested)
            }) {
                Some(v) => (v.clone(), Claim::Variant),
                None => {
                    let ident = Identity {
                        id: free_id(&registry, requested),
                        display_name: display_name.unwrap_or(requested).to_string(),
                        kind: IdentityKind::Agent,
                        owner: owner.map(str::to_string),
                    };
                    registry.push(ident.clone());
                    changed = true;
                    (ident, Claim::Variant)
                }
            }
        }
        None => {
            let ident = Identity {
                id: requested.to_string(),
                display_name: display_name.unwrap_or(requested).to_string(),
                kind: IdentityKind::Agent,
                owner: owner.map(str::to_string),
            };
            registry.push(ident.clone());
            changed = true;
            (ident, Claim::Created)
        }
    };
    if changed {
        store.save_identities(&registry)?;
    }
    Ok((ident, claim))
}

/// The identity an agent named `requested` acts as for `owner`, without
/// registering anything: `requested` itself when it is `owner`'s (or unowned,
/// unknown, or a person), else `owner`'s numbered variant if one exists.
pub fn agent_for(store: &Store, requested: &str, owner: Option<&str>) -> String {
    let Ok(mut registry) = store.load_identities() else {
        return requested.to_string();
    };
    normalize_identities(&mut registry);
    let Some(found) = registry
        .iter()
        .find(|i| i.id.eq_ignore_ascii_case(requested))
    else {
        return requested.to_string();
    };
    if found.kind == IdentityKind::Human
        || found.owner.is_none()
        || owner.is_none()
        || found.owner.as_deref() == owner
    {
        return found.id.clone();
    }
    registry
        .iter()
        .find(|i| i.owner.as_deref() == owner && is_variant(&i.id, requested))
        .map(|i| i.id.clone())
        .unwrap_or_else(|| requested.to_string())
}

/// Resolve the acting identity for an authored action, honoring
/// the user's default-identity preference. Precedence:
/// explicit → (prefer-default) → git user → board default → global default →
/// system user → `"unknown"`. Session/env overrides are layered on by the CLI.
pub fn resolve_identity(store: Option<&Store>, explicit: Option<&str>) -> String {
    if let Some(e) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return e.to_string();
    }
    let g = crate::GlobalConfig::load();
    let default_identity = g.default_identity.filter(|s| !s.trim().is_empty());
    if g.prefer_default_identity.unwrap_or(false) {
        if let Some(d) = &default_identity {
            return d.clone();
        }
    }
    let root = store
        .map(|s| s.root().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    if let Some(v) = crate::vcs::identity(&root) {
        return v;
    }
    if let Some(s) = store {
        if let Ok(settings) = s.load_settings() {
            if let Some(a) = settings.default_author.filter(|s| !s.trim().is_empty()) {
                return a;
            }
        }
    }
    if let Some(d) = default_identity {
        return d;
    }
    // Never attribute to "unknown": fall back to the OS user, then to "human".
    crate::vcs::system_user().unwrap_or_else(|| "human".to_string())
}

/// Create or update an identity's display name (and optionally its kind).
pub fn upsert_identity(
    store: &Store,
    id: &str,
    display_name: &str,
    kind: Option<IdentityKind>,
) -> Result<Identity> {
    let mut registry = store.load_identities()?;
    if let Some(existing) = registry.iter_mut().find(|i| i.id == id) {
        existing.display_name = display_name.to_string();
        if let Some(k) = kind {
            existing.kind = k;
        }
        let out = existing.clone();
        store.save_identities(&registry)?;
        Ok(out)
    } else {
        let ident = Identity {
            id: id.to_string(),
            display_name: display_name.to_string(),
            kind: kind.unwrap_or_default(),
            owner: None,
        };
        registry.push(ident.clone());
        store.save_identities(&registry)?;
        Ok(ident)
    }
}

/// Remove an identity from the saved registry. Git-derived humans that aren't in
/// the registry can't be removed (they're re-discovered from history); this is
/// meant for pruning agent identities and manual overrides.
pub fn delete_identity(store: &Store, id: &str) -> Result<()> {
    let mut registry = store.load_identities()?;
    let before = registry.len();
    registry.retain(|i| i.id != id);
    if registry.len() != before {
        store.save_identities(&registry)?;
    }
    Ok(())
}

/// Stage bytes as an [`Attachment`] without binding it to any ticket/post: if a
/// file with identical content is already tracked in the repo it is referenced in
/// place (no copy); otherwise the bytes are written under `.wipe/media/`. Shared by
/// ticket and forum attachments.
pub fn stage_media(store: &Store, name: &str, bytes: &[u8], mime: &str) -> Result<Attachment> {
    let root = store.root();
    let hash = git::blob_hash(bytes);

    // Prefer referencing an already-tracked file with identical content.
    let existing = if git::is_repo(root) {
        git::tracked_blobs(root)
            .ok()
            .and_then(|blobs| blobs.into_iter().find(|(h, _)| *h == hash).map(|(_, p)| p))
    } else {
        None
    };

    if let Some(path) = existing {
        Ok(Attachment {
            name: name.to_string(),
            path,
            source: AttachmentSource::Repo,
            size: bytes.len() as u64,
            mime: mime.to_string(),
        })
    } else {
        let file_name = format!("{}-{}", &hash[..8.min(hash.len())], sanitize(name));
        fs::create_dir_all(store.media_dir())?;
        fs::write(store.media_dir().join(&file_name), bytes)?;
        Ok(Attachment {
            name: name.to_string(),
            path: format!(".wipe/media/{file_name}"),
            source: AttachmentSource::Media,
            size: bytes.len() as u64,
            mime: mime.to_string(),
        })
    }
}

/// Attach a file to a ticket. If a file with identical content is already tracked
/// in the repo, it is referenced in place (no copy); otherwise the bytes are
/// stored under `.wipe/media/`. Returns the created [`Attachment`].
pub fn add_attachment(
    store: &Store,
    ticket_id: &str,
    name: &str,
    bytes: &[u8],
    mime: &str,
    actor: &str,
    now: DateTime<Utc>,
) -> Result<Attachment> {
    let mut ticket = store.load_ticket(ticket_id)?;
    let attachment = stage_media(store, name, bytes, mime)?;

    if !ticket.attachments.iter().any(|a| a.path == attachment.path) {
        ticket.attachments.push(attachment.clone());
        ticket.log_activity(actor, "attached", attachment.name.clone(), now);
        ticket.updated = now;
        store.save_ticket(&ticket)?;
    }
    Ok(attachment)
}

/// Detach a file from a ticket by its `path`. The underlying file is left in place
/// (it may be shared or tracked in the repo).
pub fn remove_attachment(
    store: &Store,
    ticket_id: &str,
    path: &str,
    actor: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    let mut ticket = store.load_ticket(ticket_id)?;
    let name = ticket
        .attachments
        .iter()
        .find(|a| a.path == path)
        .map(|a| a.name.clone());
    ticket.attachments.retain(|a| a.path != path);
    if let Some(name) = name {
        ticket.log_activity(actor, "detached", name, now);
    }
    ticket.updated = now;
    store.save_ticket(&ticket)?;
    Ok(())
}

/// Sanitize a file name to a safe, stable form for on-disk storage.
fn sanitize(name: &str) -> String {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap()
    }

    fn project() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path(), "Test", now()).unwrap();
        (dir, store)
    }

    fn new_ticket(s: &Store, title: &str, list: Option<&str>, actor: &str) -> Ticket {
        create_ticket(
            s,
            NewTicket {
                title: title.into(),
                body: Some(format!("{title} body")),
                list: list.map(Into::into),
                ..Default::default()
            },
            actor,
            now(),
        )
        .unwrap()
    }

    #[test]
    fn create_patch_and_activity_log() {
        let (_d, s) = project();
        let before = s.load_board().unwrap().updated;
        assert_eq!(new_ticket(&s, "A", None, "ada").id, "T-001");
        assert_eq!(new_ticket(&s, "B", None, "ada").id, "T-002");
        let board = s.load_board().unwrap();
        assert_eq!(
            board.lists[0].cards,
            vec!["T-001", "T-002"],
            "first list by default"
        );
        assert_eq!(board.next_ticket, 3);

        move_ticket(&s, "T-1", "done", None, "ada", now()).unwrap();
        let patch = TicketPatch {
            title: Some("A2".into()),
            priority: Some(Some("high".into())),
            labels: Some(vec!["blocked".into()]),
            assignees: Some(vec!["bob".into()]),
            ..Default::default()
        };
        let t = update_ticket(&s, "T-1", patch, "ada", now()).unwrap();
        assert_eq!(
            (t.title.as_str(), t.priority.as_deref()),
            ("A2", Some("high"))
        );
        let clear = TicketPatch {
            priority: Some(None),
            labels: Some(vec![]),
            assignees: Some(vec![]),
            ..Default::default()
        };
        let t = update_ticket(&s, "T-1", clear, "ada", now()).unwrap();
        assert_eq!(t.priority, None);

        let kinds: Vec<&str> = t.activity.iter().map(|a| a.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "created",
                "moved",
                "renamed",
                "priority",
                "label-added",
                "assigned",
                "priority",
                "label-removed",
                "unassigned"
            ]
        );
        // A move records the destination's display name.
        assert_eq!(t.activity[1].detail, "Done");

        // Card moves leave `board.updated` alone (fewer merge conflicts); list
        // changes stamp it.
        assert_eq!(s.load_board().unwrap().updated, before);
        let later = now() + chrono::Duration::hours(1);
        add_list(&s, "Review", later).unwrap();
        assert_eq!(s.load_board().unwrap().updated, later);
    }

    #[test]
    fn checklists_and_criteria() {
        let (_d, s) = project();
        new_ticket(&s, "A", None, "ada");
        let k = Checks::Checklist;
        let ids: Vec<String> = ["first", "second", "third"]
            .iter()
            .map(|t| checks_add(&s, k, "T-1", t, now()).unwrap())
            .collect();
        assert_eq!(ids, ["ck-1", "ck-2", "ck-3"]);
        // Criteria are a separate surface with their own counter and namespace.
        assert_eq!(
            checks_add(&s, Checks::Acceptance, "T-1", "tests pass", now()).unwrap(),
            "ac-1"
        );
        assert!(checks_set(&s, Checks::Acceptance, "T-1", "ck-1", None, "ada", now()).is_err());

        assert!(
            checks_set(&s, k, "T-1", "ck-1", None, "ada", now()).unwrap(),
            "toggle"
        );
        assert!(!checks_set(&s, k, "T-1", "ck-1", Some(false), "ada", now()).unwrap());
        checks_set(&s, k, "T-1", "ck-2", Some(true), "bob", now()).unwrap();
        checks_set(&s, k, "T-1", "ck-2", Some(true), "bob", now()).unwrap(); // no-op
        checks_edit(&s, k, "T-1", "ck-2", "second!", now()).unwrap();
        checks_move(&s, k, "T-1", "ck-3", 0, now()).unwrap();
        checks_remove(&s, k, "T-1", "ck-1", now()).unwrap();
        assert!(checks_remove(&s, k, "T-1", "ck-99", now()).is_err());

        let t = s.load_ticket("T-1").unwrap();
        let order: Vec<&str> = t.checklist.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(order, ["ck-3", "ck-2"]);
        assert!(t.checklist[1].done && t.checklist[1].text == "second!");
        assert!(!t.acceptance[0].done, "criteria untouched by the checklist");
        assert_eq!(t.next_check, 4, "ids are never reused");
        // Real state changes are logged (with who), no-ops are not.
        let checks: Vec<(&str, &str)> = t
            .activity
            .iter()
            .filter(|a| a.kind.ends_with("checked"))
            .map(|a| (a.kind.as_str(), a.actor.as_str()))
            .collect();
        assert_eq!(
            checks,
            [("checked", "ada"), ("unchecked", "ada"), ("checked", "bob")]
        );
    }

    #[test]
    fn moves_deletes_and_list_lifecycle() {
        let (_d, s) = project();
        new_ticket(&s, "A", None, "t");
        new_ticket(&s, "B", None, "t");
        move_ticket(&s, "T-1", "done", None, "t", now()).unwrap();
        let board = s.load_board().unwrap();
        assert_eq!(board.list("backlog").unwrap().cards, vec!["T-002"]);
        assert_eq!(board.list("done").unwrap().cards, vec!["T-001"]);
        assert!(matches!(
            move_ticket(&s, "T-1", "nope", None, "t", now()),
            Err(Error::ListNotFound(_))
        ));

        delete_ticket(&s, "T-1", now()).unwrap();
        assert!(matches!(
            s.load_ticket("T-1"),
            Err(Error::TicketNotFound(_))
        ));
        assert!(s
            .load_board()
            .unwrap()
            .list("done")
            .unwrap()
            .cards
            .is_empty());

        let id = add_list(&s, "In Review", now()).unwrap();
        assert_eq!(id, "in-review");
        rename_list(&s, &id, "Review", now()).unwrap();
        move_list(&s, &id, 0, now()).unwrap();
        let board = s.load_board().unwrap();
        assert_eq!(
            (board.lists[0].id.as_str(), board.lists[0].name.as_str()),
            ("in-review", "Review")
        );
        remove_list(&s, &id, false, now()).unwrap();
        // A list with tickets needs --force, which deletes them too.
        assert!(remove_list(&s, "backlog", false, now()).is_err());
        remove_list(&s, "backlog", true, now()).unwrap();
        assert!(s.load_ticket("T-2").is_err());
    }

    #[test]
    fn rewrite_by_someone_else_preserves_the_original_once() {
        let (_d, s) = project();
        new_ticket(&s, "raw note", Some("todo"), "bilal");
        let edit = |body: &str, title: Option<&str>, who: &str| {
            let patch = TicketPatch {
                body: Some(body.into()),
                title: title.map(Into::into),
                ..Default::default()
            };
            update_ticket(&s, "T-1", patch, who, now()).unwrap()
        };
        // The creator editing their own note freezes nothing...
        assert!(edit("raw note, typo fixed", None, "bilal")
            .original
            .is_none());
        // ...an agent's rewrite freezes the creator's current wording...
        let o = edit("## Plan", Some("Proper ticket"), "claude")
            .original
            .unwrap();
        assert_eq!(
            (
                o.title.as_str(),
                o.body.as_str(),
                o.author.as_str(),
                o.rewritten_by.as_str()
            ),
            ("raw note", "raw note, typo fixed", "bilal", "claude")
        );
        // ...and later rewrites never touch it.
        assert_eq!(edit("third version", None, "someone").original, Some(o));
    }

    #[test]
    fn attachment_prefers_repo_reference_over_copy() {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "Tester"]);
        let s = Store::init(root, "Att", now()).unwrap();
        new_ticket(&s, "A", None, "t");
        std::fs::write(root.join("logo.png"), b"PNGDATA").unwrap();
        git(&["add", "logo.png"]);
        git(&["commit", "-q", "-m", "add logo"]);

        // Identical bytes -> reference the tracked repo file (no copy).
        let a = add_attachment(&s, "T-1", "logo.png", b"PNGDATA", "image/png", "t", now()).unwrap();
        assert_eq!(
            (a.source, a.path.as_str()),
            (AttachmentSource::Repo, "logo.png")
        );
        // Novel bytes -> copied into .wipe/media/.
        let b = add_attachment(&s, "T-1", "new.txt", b"hello", "text/plain", "t", now()).unwrap();
        assert_eq!(b.source, AttachmentSource::Media);
        assert!(b.path.starts_with(".wipe/media/") && root.join(&b.path).exists());
    }

    #[test]
    fn agents_belong_to_a_person_and_ids_stay_unique() {
        let (_d, s) = project();
        let claim = |id: &str, owner: &str| claim_agent(&s, id, None, Some(owner)).unwrap();
        // First use registers (or claims) the agent for that person.
        assert_eq!(claim("claude", "ada@x").1, Claim::Created);
        assert_eq!(
            claim("Claude", "ada@x").0.id,
            "claude",
            "ids are case-insensitive"
        );
        // Another person's agent of the same name is their own variant.
        let (bob, how) = claim("claude", "bob@x");
        assert_eq!((bob.id.as_str(), how), ("claude02", Claim::Variant));
        assert_eq!(bob.owner.as_deref(), Some("bob@x"));
        assert_eq!(claim("claude", "bob@x").0.id, "claude02", "and stays it");
        assert_eq!(agent_for(&s, "claude", Some("bob@x")), "claude02");
        assert_eq!(agent_for(&s, "claude", Some("ada@x")), "claude");
        // An unowned agent (from before 1.1) is claimed by its first user.
        upsert_identity(&s, "bot", "bot", Some(IdentityKind::Agent)).unwrap();
        assert_eq!(claim("bot", "bob@x").1, Claim::Existing);
        assert_eq!(claim("bot", "ada@x").0.id, "bot02");
        // A person's identity is never an agent's.
        upsert_identity(&s, "eve@x", "Eve", Some(IdentityKind::Human)).unwrap();
        assert!(claim_agent(&s, "eve@x", None, Some("bob@x")).is_err());

        // Duplicates met in a merge: agreeing entries fold, clashing ones are renamed.
        let agent = |id: &str, owner: Option<&str>| Identity {
            id: id.into(),
            display_name: id.into(),
            kind: IdentityKind::Agent,
            owner: owner.map(str::to_string),
        };
        let mut list = vec![
            agent("claude", Some("ada@x")),
            agent("claude", None),
            agent("Claude", Some("bob@x")),
        ];
        assert!(normalize_identities(&mut list));
        let ids: Vec<&str> = list.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["claude", "Claude02"]);
    }
}

//! Taking one change at a time: a ticket, list, label, thread or board setting
//! as it is in another state of the board (the incoming merge, a past commit),
//! carried into this one - everything else left as it is.
//!
//! A ticket is its file plus where its card sits; a list is its name, color,
//! limit, position and card order; a label is its definition. These work on
//! [`Files`] snapshots, so the same code updates the working copy and the base
//! overlay.

use serde::{Deserialize, Serialize};

use super::Files;
use crate::error::{Error, Result};
use crate::model::{Board, Definitions, List};

/// What to take: the `scope` and `id` of a [`super::Change`] (for a board-level
/// change, `fields` says which part: `name`, `description`, `priorities`,
/// `lists` (their order), or a file such as `identities`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    pub scope: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

/// The board files taking `e` reads and writes.
pub fn paths(e: &Entity) -> Vec<String> {
    match e.scope.as_str() {
        "ticket" => vec![format!("tickets/{}.json", e.id), "board.json".into()],
        "thread" => vec![format!("forum/{}.json", e.id), "board.json".into()],
        "list" => vec!["board.json".into()],
        "label" => vec!["definitions.json".into()],
        "board" => {
            let mut v = vec!["board.json".to_string(), "definitions.json".to_string()];
            for f in &e.fields {
                if !matches!(f.as_str(), "name" | "description" | "priorities" | "lists") {
                    v.push(format!("{f}.json"));
                }
            }
            v
        }
        _ => vec![e.id.clone()],
    }
}

fn parse<T: serde::de::DeserializeOwned>(f: &Files, path: &str) -> Result<T> {
    let bytes = f
        .get(path)
        .ok_or_else(|| Error::msg(format!("{path} is missing")))?;
    Ok(serde_json::from_slice(bytes)?)
}

fn put<T: Serialize>(f: &mut Files, path: &str, v: &T) -> Result<()> {
    let mut s = serde_json::to_string_pretty(v)?;
    s.push('\n');
    f.insert(path.to_string(), s.into_bytes());
    Ok(())
}

/// Copy (or remove) one file as it is in `source`.
fn file(target: &mut Files, source: &Files, path: &str) {
    match source.get(path) {
        Some(bytes) => {
            target.insert(path.to_string(), bytes.clone());
        }
        None => {
            target.remove(path);
        }
    }
}

/// Carry `e` from `source` into `target`. Both hold [`paths`]`(e)` (absent:
/// the file doesn't exist on that side).
pub fn apply(target: &mut Files, source: &Files, e: &Entity) -> Result<()> {
    match e.scope.as_str() {
        "ticket" => {
            let path = format!("tickets/{}.json", e.id);
            file(target, source, &path);
            let mut board: Board = parse(target, "board.json")?;
            let theirs: Board = parse(source, "board.json")?;
            let was = board.locate_card(&e.id);
            // Ids up to the source's are taken there.
            board.next_ticket = board.next_ticket.max(theirs.next_ticket);
            for l in &mut board.lists {
                l.cards.retain(|c| c != &e.id);
            }
            if target.contains_key(&path) {
                // Where the source has the card; else where it was; else the first list.
                let (list, at) = theirs
                    .locate_card(&e.id)
                    .filter(|(l, _)| board.list(l).is_some())
                    .or(was)
                    .or_else(|| board.lists.first().map(|l| (l.id.clone(), usize::MAX)))
                    .ok_or_else(|| Error::msg("the board has no lists"))?;
                let l = board.list_mut(&list).expect("located list");
                // After the card it follows in the source (else before the one it
                // precedes), so both sides end up in the same order; else by index.
                let src = theirs
                    .list(&list)
                    .map(|s| s.cards.clone())
                    .unwrap_or_default();
                let at = match src.iter().position(|c| c == &e.id) {
                    Some(i) => src[..i]
                        .iter()
                        .rev()
                        .find_map(|p| l.cards.iter().position(|c| c == p).map(|x| x + 1))
                        .or_else(|| {
                            src[i + 1..]
                                .iter()
                                .find_map(|n| l.cards.iter().position(|c| c == n))
                        })
                        .unwrap_or(at),
                    None => at,
                };
                let at = at.min(l.cards.len());
                l.cards.insert(at, e.id.clone());
            }
            put(target, "board.json", &board)
        }
        "thread" => {
            file(target, source, &format!("forum/{}.json", e.id));
            let mut board: Board = parse(target, "board.json")?;
            let theirs: Board = parse(source, "board.json")?;
            board.next_thread = board.next_thread.max(theirs.next_thread);
            put(target, "board.json", &board)
        }
        "list" => {
            let mut board: Board = parse(target, "board.json")?;
            let theirs: Board = parse(source, "board.json")?;
            match theirs.lists.iter().position(|l| l.id == e.id) {
                Some(at) => {
                    let src = &theirs.lists[at];
                    let mut list = match board.lists.iter().position(|l| l.id == e.id) {
                        Some(i) => board.lists.remove(i),
                        None => List {
                            id: src.id.clone(),
                            name: src.name.clone(),
                            color: None,
                            wip_limit: None,
                            cards: Vec::new(),
                        },
                    };
                    list.name = src.name.clone();
                    list.color = src.color.clone();
                    list.wip_limit = src.wip_limit;
                    // The source's card order, for the cards this list holds.
                    let mut cards: Vec<String> = src
                        .cards
                        .iter()
                        .filter(|c| list.cards.contains(c))
                        .cloned()
                        .collect();
                    cards.extend(
                        list.cards
                            .iter()
                            .filter(|c| !src.cards.contains(c))
                            .cloned(),
                    );
                    list.cards = cards;
                    let at = at.min(board.lists.len());
                    board.lists.insert(at, list);
                }
                None => {
                    if let Some(i) = board.lists.iter().position(|l| l.id == e.id) {
                        if !board.lists[i].cards.is_empty() {
                            return Err(Error::msg(format!(
                                "list `{}` still holds cards here - move or take them first",
                                e.id
                            )));
                        }
                        board.lists.remove(i);
                    }
                }
            }
            put(target, "board.json", &board)
        }
        "label" => {
            let mut defs: Definitions = parse(target, "definitions.json")?;
            let theirs: Definitions = parse(source, "definitions.json")?;
            let src = theirs.labels.iter().find(|l| l.name == e.id).cloned();
            match (defs.labels.iter().position(|l| l.name == e.id), src) {
                (Some(i), Some(l)) => defs.labels[i] = l,
                (None, Some(l)) => defs.labels.push(l),
                (Some(i), None) => {
                    defs.labels.remove(i);
                }
                (None, None) => {}
            }
            put(target, "definitions.json", &defs)
        }
        "board" => {
            for f in &e.fields {
                match f.as_str() {
                    "name" | "description" => {
                        let mut board: Board = parse(target, "board.json")?;
                        let theirs: Board = parse(source, "board.json")?;
                        if f == "name" {
                            board.name = theirs.name;
                        } else {
                            board.description = theirs.description;
                        }
                        put(target, "board.json", &board)?;
                    }
                    "priorities" => {
                        let mut defs: Definitions = parse(target, "definitions.json")?;
                        let theirs: Definitions = parse(source, "definitions.json")?;
                        defs.priorities = theirs.priorities;
                        put(target, "definitions.json", &defs)?;
                    }
                    "lists" => {
                        // The source's list order, for the lists both have.
                        let mut board: Board = parse(target, "board.json")?;
                        let theirs: Board = parse(source, "board.json")?;
                        let rank = |id: &str| {
                            theirs
                                .lists
                                .iter()
                                .position(|l| l.id == id)
                                .unwrap_or(usize::MAX)
                        };
                        board.lists.sort_by_key(|l| rank(&l.id));
                        put(target, "board.json", &board)?;
                    }
                    "ticket ids" => {
                        return Err(Error::msg(
                            "the ticket-id format can't be taken on its own - sync the whole board",
                        ))
                    }
                    other => file(target, source, &format!("{other}.json")),
                }
            }
            Ok(())
        }
        _ => {
            file(target, source, &e.id);
            Ok(())
        }
    }
}

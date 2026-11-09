//! `wipe forum ...` - post to and search the project's git-tracked forum.

use std::collections::HashSet;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use serde_json::{json, Value};

use wipe_core::forum::{self, NewPost, PostView, SearchQuery};
use wipe_core::model::Attachment;
use wipe_core::Store;

use crate::args::*;
use crate::commands::guess_mime;
use crate::identity;
use crate::input;
use crate::output::{dim, id_style, Out};
use crate::safe;

fn store() -> Result<Store> {
    Store::discover(".").map_err(Into::into)
}

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("model is serializable")
}

/// Read and stage each attachment path into an [`Attachment`] (dedup vs repo blobs
/// or copy into `.wipe/media/`), respecting the board's size limit.
fn stage_attachments(store: &Store, paths: &[PathBuf]) -> Result<Vec<Attachment>> {
    let limit = store.load_settings()?.max_attachment_mb * 1024 * 1024;
    let mut out = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow!("invalid file name: {}", path.display()))?
            .to_string();
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if bytes.len() as u64 > limit {
            bail!(
                "{name} is {:.1} MB, over the {} MB attachment limit",
                bytes.len() as f64 / 1_048_576.0,
                limit / 1024 / 1024
            );
        }
        out.push(wipe_core::ops::stage_media(
            store,
            &name,
            &bytes,
            guess_mime(&name),
        )?);
    }
    Ok(out)
}

/// Post characters `forum show` prints by default.
const POST_MAX: usize = 1500;
/// Threads `forum list` prints by default.
const LIST_LIMIT: usize = 25;

/// First non-empty line of a body, trimmed to a readable length.
fn snippet(body: &str) -> String {
    let clean = safe::clean(body);
    let line = clean
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > 100 {
        let s: String = line.chars().take(97).collect();
        format!("{s}...")
    } else {
        line.to_string()
    }
}

pub fn run(out: &Out, cmd: ForumCmd) -> Result<()> {
    let s = store()?;
    match cmd {
        ForumCmd::Post(a) => {
            let body = input::text(a.body, a.body_file.as_deref(), "the post body")?;
            let attachments = stage_attachments(&s, &a.attach)?;
            let author = identity::resolve(a.author)?;
            let spec = NewPost {
                summary: a.summary,
                body: body.unwrap_or_default(),
                labels: a.labels,
                refs: canonical_refs(&s, a.refs),
                attachments,
                reply_to: a.reply_to,
            };
            let id = forum::create_post(&s, spec, &author, Utc::now())?;
            let p = forum::get_post(&s, &id)?;
            let to = if p.parents.is_empty() {
                String::new()
            } else {
                format!(" (reply to {})", p.parents.join(", "))
            };
            out.ok(
                format!("posted {id} - {}{to}", safe::line(&p.summary)),
                json!({ "ok": true, "id": id, "summary": p.summary, "thread": p.thread_id,
                        "parents": p.parents, "author": author }),
            );
        }

        ForumCmd::Reply(a) => {
            let body = input::required(a.body, a.body_file.as_deref(), "the reply body", "--body")?;
            let attachments = stage_attachments(&s, &a.attach)?;
            let author = identity::resolve(a.author)?;
            let parent = a.id.clone();
            let spec = NewPost {
                summary: a.summary,
                body,
                labels: a.labels,
                refs: canonical_refs(&s, a.refs),
                attachments,
                reply_to: vec![parent.clone()],
            };
            let id = forum::create_post(&s, spec, &author, Utc::now())?;
            out.ok(
                format!("replied {id} (to {parent})"),
                json!({ "ok": true, "id": id, "parent": parent, "author": author }),
            );
        }

        ForumCmd::Show { id, depth, full } => {
            let all = forum::index(&s)?;
            let id = forum::resolve_post_id(&s, &id)?;
            let mut posts = forum::subtree(&all, &id);
            let top_depth = posts.iter().find(|p| p.id == id).map_or(0, |p| p.depth);
            // --depth bounds the JSON too, so the payload matches the text.
            if let Some(d) = depth {
                let keep = tree_within(&posts, &id, d);
                posts.retain(|p| keep.contains(&p.id));
            }
            let clipped = clean_posts(&mut posts, full);
            if out.json {
                let root = posts.iter().find(|p| p.id == id);
                out.json_value(&json!({
                    "id": id,
                    "thread": root.map(|p| p.thread_id.clone()),
                    "title": root.map(|p| safe::line(&p.thread_title)),
                    "posts": to_value(&posts),
                }));
            } else {
                render_tree(&posts, &id, top_depth);
                if clipped > 0 {
                    println!(
                        "{}",
                        dim(&format!(
                            "{clipped} long post(s) clipped - --full shows them whole"
                        ))
                    );
                }
            }
        }

        ForumCmd::List {
            label,
            author,
            limit,
        } => {
            let all = forum::index(&s)?;
            let mut roots: Vec<&PostView> = all
                .iter()
                .filter(|p| p.depth == 0)
                .filter(|r| label.as_ref().is_none_or(|l| r.labels.contains(l)))
                .filter(|r| {
                    author
                        .as_ref()
                        .is_none_or(|a| r.author.to_lowercase().contains(&a.to_lowercase()))
                })
                .collect();
            let total = roots.len();
            roots.truncate(match limit {
                Some(0) => usize::MAX,
                Some(n) => n,
                None => LIST_LIMIT,
            });
            let total_of = |tid: &str| all.iter().filter(|p| p.thread_id == tid).count();
            if out.json {
                let arr: Vec<Value> = roots
                    .iter()
                    .map(|r| {
                        json!({
                            "id": r.id,
                            "title": r.summary,
                            "author": r.author,
                            "labels": r.labels,
                            "posts": total_of(&r.thread_id),
                            "created": r.created,
                        })
                    })
                    .collect();
                out.json_value(&json!(arr));
            } else if roots.is_empty() {
                out.line("no threads yet - start one with `wipe forum post`");
            } else {
                for r in &roots {
                    let n = total_of(&r.id);
                    let labels = if r.labels.is_empty() {
                        String::new()
                    } else {
                        format!("[{}] ", r.labels.join(","))
                    };
                    println!(
                        "{}  {}  {}",
                        id_style(&r.id),
                        crate::view::title(&r.summary),
                        dim(&format!(
                            "({labels}{}{})",
                            safe::line(&r.author),
                            if n > 1 {
                                format!(", {n} posts")
                            } else {
                                String::new()
                            }
                        ))
                    );
                }
                if total > roots.len() {
                    println!(
                        "{}",
                        dim(&format!(
                            "… {} older (--limit 0, or wipe search <words> --forum)",
                            total - roots.len()
                        ))
                    );
                }
            }
        }

        ForumCmd::Search(a) => {
            // A blank/whitespace-only pattern means "no pattern" (match all),
            // matching the daemon's behavior.
            let pattern = match a.pattern.as_deref() {
                Some(p) if !p.trim().is_empty() => {
                    Some(forum::compile_pattern(p, !a.case_sensitive)?)
                }
                _ => None,
            };
            let q = SearchQuery {
                pattern,
                author: a.author,
                labels: a.labels,
                scope: a.scope,
                max_depth: a.depth,
                titles_only: a.titles,
                limit: (a.limit > 0).then_some(a.limit),
            };
            let hits = forum::search(&s, &q)?;
            if out.json {
                // Same guardrails as the text view: clean, clipped bodies.
                let mut v = to_value(&hits);
                for h in v.as_array_mut().into_iter().flatten() {
                    for key in ["body", "thread_title", "summary"] {
                        if let Some(text) = h.get(key).and_then(Value::as_str) {
                            let clean = if key == "body" {
                                safe::clip(text, POST_MAX).0
                            } else {
                                safe::line(text)
                            };
                            h[key] = json!(clean);
                        }
                    }
                }
                out.json_value(&v);
            } else if hits.is_empty() {
                out.line("no matching posts");
            } else {
                for h in &hits {
                    let labels = if h.labels.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", dim(&format!("[{}]", h.labels.join(","))))
                    };
                    println!(
                        "{}  {}{}  {}  {}",
                        id_style(&h.id),
                        dim(&h.author),
                        labels,
                        crate::view::title(&h.summary),
                        dim(&snippet(&h.body))
                    );
                }
                println!(
                    "{}",
                    dim(&format!(
                        "{} match(es) · `wipe forum show <id>` to open, `wipe forum reply <id>` to respond",
                        hits.len()
                    ))
                );
            }
        }

        ForumCmd::Edit {
            id,
            summary,
            body,
            body_file,
            author,
        } => {
            let body = input::text(body, body_file.as_deref(), "the new body")?;
            if body.is_none() && summary.is_none() && author.is_none() {
                bail!(
                    "nothing to change - pass --summary <TEXT>, --body <TEXT> (or --body-file) \
                     and/or --author <ID>"
                );
            }
            let now = Utc::now();
            let id = forum::resolve_post_id(&s, &id)?;
            if body.is_some() || summary.is_some() {
                forum::edit_post(&s, &id, body.as_deref(), summary.as_deref(), now)?;
            }
            if let Some(new_author) = &author {
                let old = forum::reattribute_post(&s, &id, new_author, now)?;
                out.ok(
                    format!("edited {id} (reattributed {old} -> {new_author})"),
                    json!({ "ok": true, "id": id, "from": old, "to": new_author }),
                );
            } else {
                out.ok(format!("edited {id}"), json!({ "ok": true, "id": id }));
            }
        }

        ForumCmd::Delete { id, yes } => {
            if !yes {
                bail!("refusing to delete {id} and its replies without --yes");
            }
            let id = forum::resolve_post_id(&s, &id)?;
            let threads = s.load_board()?.forum == wipe_core::model::ForumFormat::Threads;
            forum::delete_post(&s, &id, Utc::now())?;
            out.ok(
                if threads {
                    format!("deleted {id} and its replies")
                } else {
                    format!("deleted {id} (replies to it stay)")
                },
                json!({ "ok": true, "id": id }),
            );
        }

        ForumCmd::Digest(a) => {
            let text = digest(&s, &a.label, a.max_bytes)?;
            if out.json {
                out.json_value(&json!({ "label": a.label, "bytes": text.len(), "digest": text }));
            } else {
                print!("{text}");
            }
        }

        ForumCmd::Pin { id } | ForumCmd::Unpin { id } if id.contains('.') => {
            bail!(
                "only whole threads can be pinned - use the thread id (e.g. {})",
                id.split('.').next().unwrap_or(&id)
            );
        }
        ForumCmd::Pin { id } | ForumCmd::Unpin { id }
            if forum::get_post(&s, &id).is_ok_and(|p| p.depth > 0) =>
        {
            let p = forum::get_post(&s, &id)?;
            bail!(
                "only whole threads can be pinned - {} replies to {}; pin its thread, {}",
                p.id,
                p.parents.join(", "),
                p.thread_id
            );
        }
        ForumCmd::Pin { id } => {
            // The label must exist in the board's pool for the UI to color it.
            if !s
                .load_definitions()?
                .labels
                .iter()
                .any(|l| l.name == PIN_LABEL)
            {
                wipe_core::ops::create_label(
                    &s,
                    PIN_LABEL,
                    None,
                    Some("in `wipe forum digest`".into()),
                )?;
            }
            let changed = forum::set_root_label(&s, &id, PIN_LABEL, true, Utc::now())?;
            out.ok(
                if changed {
                    format!("pinned {id}")
                } else {
                    format!("{id} was already pinned")
                },
                json!({ "ok": true, "id": id, "pinned": true, "changed": changed }),
            );
        }
        ForumCmd::Unpin { id } => {
            let changed = forum::set_root_label(&s, &id, PIN_LABEL, false, Utc::now())?;
            out.ok(
                if changed {
                    format!("unpinned {id}")
                } else {
                    format!("{id} was not pinned")
                },
                json!({ "ok": true, "id": id, "pinned": false, "changed": changed }),
            );
        }

        ForumCmd::Watch(a) => watch(&s, a)?,
    }
    Ok(())
}

/// The label `wipe forum pin` applies and `wipe forum digest` collects.
const PIN_LABEL: &str = "pinned";

/// Build the Markdown digest of threads labeled `label`: per thread a heading,
/// the opening post, and one line per reply - oldest thread first, each thread's
/// share of `max_bytes` capped so one long thread cannot crowd out the rest.
fn digest(s: &Store, label: &str, max_bytes: usize) -> Result<String> {
    let threads = forum::labeled_threads(s, label)?;
    let mut out = format!("# Project notes (wipe forum, label `{label}`)\n\n");
    if threads.is_empty() {
        out.push_str(&format!(
            "_No threads labeled `{label}` yet - pin one with `wipe forum pin F-<id>`._\n"
        ));
        return Ok(out);
    }
    let footer = "\n_Full threads: `wipe forum show <id>`._\n";
    let budget = max_bytes.saturating_sub(out.len() + footer.len());
    let share = (budget / threads.len()).max(200);
    for (root, replies) in &threads {
        let mut sec = format!(
            "## {} - {}\n\n{}\n",
            root.id,
            root.summary,
            root.body.trim()
        );
        if !replies.is_empty() {
            sec.push('\n');
            for p in replies {
                sec.push_str(&format!(
                    "- {} ({}): {}\n",
                    p.id,
                    p.author,
                    safe::line(&p.summary)
                ));
            }
        }
        sec.push('\n');
        out.push_str(&clip(&sec, share, &root.id));
    }
    out.push_str(footer);
    // Hard cap: the per-thread floor can overshoot when there are many threads.
    if out.len() > max_bytes {
        let cut = clip(&out, max_bytes.saturating_sub(60), "");
        out = format!("{cut}\n_Digest truncated at {max_bytes} bytes - raise --max-bytes._\n");
    }
    Ok(out)
}

/// Clip `text` to at most `max` bytes on a char boundary, ending with a pointer
/// to the full thread when anything was cut.
fn clip(text: &str, max: usize, id: &str) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let more = if id.is_empty() {
        String::new()
    } else {
        format!(" (cut - `wipe forum show {id}`)")
    };
    format!("{}…{more}\n\n", text[..end].trim_end())
}

/// The ids of the posts at most `max_depth` replies below `top`, following
/// first parents (how [`render_tree`] nests them).
fn tree_within(posts: &[PostView], top: &str, max_depth: usize) -> HashSet<String> {
    let mut keep: HashSet<String> = HashSet::from([top.to_string()]);
    let mut frontier = vec![top.to_string()];
    for _ in 0..max_depth {
        let next: Vec<String> = posts
            .iter()
            .filter(|p| p.parents.first().is_some_and(|f| frontier.contains(f)))
            .map(|p| p.id.clone())
            .collect();
        keep.extend(next.iter().cloned());
        frontier = next;
    }
    keep
}

/// Ticket refs in their canonical spelling (`t2a` -> `T-02A`, an old `T-42`
/// -> its hex id), so posts never carry stale ids. Unknown refs stay as typed.
fn canonical_refs(s: &Store, refs: Vec<String>) -> Vec<String> {
    refs.into_iter()
        .map(|r| s.resolve_ticket_id(&r).unwrap_or(r))
        .collect()
}

/// Sanitize every body (and summary) and, unless `full`, clip long bodies.
/// Returns how many were clipped.
fn clean_posts(posts: &mut [PostView], full: bool) -> usize {
    let mut clipped = 0;
    for p in posts {
        p.summary = safe::line(&p.summary);
        if full {
            p.body = safe::clean(&p.body).into_owned();
        } else {
            let (body, cut) = safe::clip(&p.body, POST_MAX);
            p.body = if cut > 0 {
                clipped += 1;
                format!("{body}\n… {cut} more chars")
            } else {
                body
            };
        }
    }
    clipped
}

/// Render `top` and the posts under it as an indented tree: each post nests
/// under its first parent (other parents are named on its header line).
fn render_tree(posts: &[PostView], top: &str, top_depth: usize) {
    fn node(posts: &[PostView], p: &PostView, level: usize, seen: &mut HashSet<String>) {
        if !seen.insert(p.id.clone()) {
            return;
        }
        let pad = "  ".repeat(level);
        let labels = if p.labels.is_empty() {
            String::new()
        } else {
            format!(" {}", dim(&format!("[{}]", p.labels.join(","))))
        };
        let also = if p.parents.len() > 1 {
            format!(
                " {}",
                dim(&format!("(also replies to {})", p.parents[1..].join(", ")))
            )
        } else {
            String::new()
        };
        println!(
            "{pad}{}  {}  {}{labels}{also}",
            id_style(&p.id),
            crate::view::title(&p.summary),
            dim(&safe::line(&p.author)),
        );
        for line in p.body.lines() {
            println!("{pad}  {line}");
        }
        for c in posts
            .iter()
            .filter(|c| c.parents.first().is_some_and(|f| *f == p.id))
        {
            node(posts, c, level + 1, seen);
        }
    }
    let Some(root) = posts.iter().find(|p| p.id == top) else {
        return;
    };
    if top_depth > 0 {
        println!(
            "{}",
            dim(&format!(
                "in thread {} - {}",
                root.thread_id,
                safe::line(&root.thread_title)
            ))
        );
    }
    node(posts, root, 0, &mut HashSet::new());
}

/// Stream new matching posts as newline-delimited JSON until interrupted.
fn watch(store: &Store, a: ForumWatchArgs) -> Result<()> {
    let pattern = match a.pattern.as_deref() {
        Some(p) if !p.trim().is_empty() => Some(forum::compile_pattern(p, true)?),
        _ => None,
    };
    let q = SearchQuery {
        pattern,
        author: a.author,
        labels: a.labels,
        scope: a.scope,
        ..Default::default()
    };
    let interval = Duration::from_millis(a.interval.max(50));

    // One initial snapshot seeds both `seen` (every id) and the optional replay
    // (its matching subset, oldest-first). Using a single snapshot closes the
    // window where a post created between two separate reads would be recorded as
    // seen yet never emitted.
    let mut seen: HashSet<String> = {
        let initial = forum::index(store)?;
        if a.replay {
            let mut matched: Vec<&PostView> =
                initial.iter().filter(|p| forum::matches(p, &q)).collect();
            matched.sort_by_key(|p| p.created);
            for p in matched {
                // A closed pipe (consumer gone) ends the watch cleanly.
                if emit(p).is_err() {
                    return Ok(());
                }
            }
        }
        initial.into_iter().map(|p| p.id).collect()
    };

    loop {
        std::thread::sleep(interval);
        let all = match forum::index(store) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let mut fresh: Vec<&PostView> = all
            .iter()
            .filter(|p| !seen.contains(&p.id) && forum::matches(p, &q))
            .collect();
        fresh.sort_by_key(|p| p.created); // oldest-first
        for p in fresh {
            if emit(p).is_err() {
                return Ok(());
            }
        }
        // Rebuild `seen` from the current ids each poll: bounded memory, and safe
        // because dotted ids are never reused (a deleted id can never return).
        seen = all.into_iter().map(|p| p.id).collect();
    }
}

/// Write one NDJSON event to stdout and flush (so listeners get it immediately).
/// A write error means the consumer closed the pipe (e.g. `| head -1`, or a
/// harness that exited); propagate it so `watch` stops cleanly instead of
/// polling and re-indexing the forum forever as an orphan.
fn emit(p: &PostView) -> std::io::Result<()> {
    let event = json!({
        "id": p.id,
        "summary": p.summary,
        "parents": p.parents,
        "thread": p.thread_id,
        "title": p.thread_title,
        "author": p.author,
        "labels": p.labels,
        "depth": p.depth,
        "body": p.body,
        "created": p.created,
    });
    let mut lock = std::io::stdout().lock();
    writeln!(lock, "{event}")?;
    lock.flush()
}

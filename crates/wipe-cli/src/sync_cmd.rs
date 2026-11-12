//! `wipe commit`, `wipe sync`, `wipe migrate`, and the board-sync lines every
//! command appends: the board lives on the wipe branch (see
//! [`wipe_core::sync`]), and an agent learns about uncommitted or incoming board
//! changes from those lines rather than from git.

use anyhow::{bail, Result};
use owo_colors::{OwoColorize, Stream};
use serde_json::{json, Value};
use wipe_core::sync::{self, Change, Conflict, Mode, Resolutions, Side};
use wipe_core::Store;

use crate::args::{CommitArgs, LogArgs, MigrateArgs, ResolveArgs, RestoreArgs, SyncArgs};
use crate::identity;
use crate::output::{dim, Out};
use crate::safe;

/// Changes named in a one-line summary before "+N more".
const SUMMARY_MAX: usize = 3;

/// A merge stopped at conflicts; nothing was written. Carried as an error so
/// the exit code is non-zero and `--json` gets the conflicts as data.
#[derive(Debug)]
pub struct Conflicts {
    pub conflicts: Vec<Conflict>,
    pub doing: &'static str,
}

impl std::fmt::Display for Conflicts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.conflicts.len();
        writeln!(
            f,
            "{n} conflict{} between your board changes and the wipe branch - nothing was {}:",
            if n == 1 { "" } else { "s" },
            self.doing
        )?;
        for c in &self.conflicts {
            let what = if c.field.is_empty() {
                format!("{} (deleted on one side, changed on the other)", c.entity)
            } else {
                format!("{} {}", c.entity, c.field)
            };
            writeln!(f, "  {what}  [{}]", c.id)?;
            writeln!(f, "    yours:  {}", show(&c.ours))?;
            writeln!(f, "    branch: {}", show(&c.theirs))?;
        }
        write!(
            f,
            "settle them with --take ours|theirs (all), or one at a time: --resolve '<id>=ours'"
        )
    }
}

impl std::error::Error for Conflicts {}

fn show(v: &Value) -> String {
    match v {
        Value::Null => "(deleted)".into(),
        Value::String(s) => format!("\"{}\"", safe::clip(&safe::line(s), 70).0),
        other => safe::clip(&other.to_string(), 70).0,
    }
}

/// The conflict as JSON for `--json` errors.
pub fn conflicts_json(c: &Conflicts) -> Value {
    json!({ "ok": false, "error": format!("{} conflict(s) - nothing was {}", c.conflicts.len(), c.doing), "conflicts": c.conflicts })
}

fn resolutions(a: &ResolveArgs) -> Result<Resolutions> {
    let mut r = Resolutions::default();
    if let Some(t) = &a.take {
        r.default = Some(
            Side::parse(t)
                .ok_or_else(|| anyhow::anyhow!("--take expects `ours` or `theirs`, not `{t}`"))?,
        );
    }
    for spec in &a.resolve {
        let Some((id, side)) = spec.rsplit_once('=') else {
            bail!("--resolve expects <conflict-id>=ours|theirs, not `{spec}`");
        };
        let side = Side::parse(side)
            .ok_or_else(|| anyhow::anyhow!("--resolve: `{side}` is not ours or theirs"))?;
        r.picks.insert(id.trim().to_string(), side);
    }
    Ok(r)
}

fn store() -> Result<Store> {
    Ok(Store::discover(".")?)
}

fn lines(changes: &[Change]) -> Vec<String> {
    changes.iter().map(|c| safe::line(&c.line())).collect()
}

fn summary(changes: &[Change]) -> String {
    let mut parts: Vec<String> = changes
        .iter()
        .take(SUMMARY_MAX)
        .map(|c| {
            let l = c.line();
            // Drop the quoted title in the one-liner.
            safe::line(l.split(" \"").next().unwrap_or(&l))
        })
        .collect();
    if changes.len() > SUMMARY_MAX {
        parts.push(format!("+{} more", changes.len() - SUMMARY_MAX));
    }
    parts.join(", ")
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// `wipe commit`
pub fn commit(out: &Out, a: CommitArgs) -> Result<()> {
    let s = store()?;
    let who = identity::resolve(a.author)?;
    let res = resolutions(&a.resolve)?;
    let c =
        sync::commit(&s, a.message.as_deref(), a.target.as_deref(), &who, &res).map_err(|e| {
            anyhow::anyhow!("{e} - nothing was committed; your changes stay in the working copy")
        })?;
    if !c.conflicts.is_empty() {
        return Err(Conflicts {
            conflicts: c.conflicts,
            doing: "committed",
        }
        .into());
    }
    let renumbered: Vec<Value> = c
        .renumbered
        .iter()
        .map(|(o, n)| json!({ "from": o, "to": n }))
        .collect();
    let Some(short) = &c.short else {
        out.ok(
            if c.merged.is_empty() {
                "nothing to commit - the board matches the wipe branch".to_string()
            } else {
                format!(
                    "nothing of yours to commit; applied {} from the wipe branch",
                    plural(c.merged.len(), "incoming change")
                )
            },
            json!({ "ok": true, "commit": Value::Null, "merged": c.merged, "renumbered": renumbered }),
        );
        return Ok(());
    };
    let subject = c.message.lines().next().unwrap_or("").to_string();
    let to = match &c.remote {
        Some(r) => format!("{}/{}", r, sync::BRANCH),
        None => sync::BRANCH.to_string(),
    };
    out.ok(
        format!(
            "committed {short} to {to} as {who}: {}",
            safe::line(&subject)
        ),
        json!({
            "ok": true,
            "commit": c.commit,
            "short": short,
            "branch": sync::BRANCH,
            "remote": c.remote,
            "author": who,
            "message": c.message,
            "changes": c.changes,
            "merged": c.merged,
            "renumbered": renumbered,
        }),
    );
    if !out.json {
        for l in lines(&c.changes) {
            println!("  {} {l}", dim("+"));
        }
        if !c.merged.is_empty() {
            println!(
                "  {}",
                dim(&format!(
                    "applied {} from the wipe branch first: {}",
                    plural(c.merged.len(), "incoming change"),
                    summary(&c.merged)
                ))
            );
        }
        for (o, n) in &c.renumbered {
            println!(
                "  {}",
                dim(&format!(
                    "renumbered your {o} to {n} (the branch already used {o})"
                ))
            );
        }
        if a.message.is_none() {
            println!(
                "  {}",
                dim("tip: -m \"<what and why>\" makes the board history easier to read")
            );
        }
    }
    Ok(())
}

/// `wipe sync`
pub fn sync(out: &Out, a: SyncArgs) -> Result<()> {
    let s = store()?;
    if a.check {
        let st = if a.no_fetch {
            sync::status(&s)?
        } else {
            sync::refresh(&s)?
        };
        return report_status(out, &st);
    }
    let res = resolutions(&a.resolve)?;
    if !a.targets.is_empty() {
        if !a.no_fetch {
            sync::refresh(&s)?;
        }
        return take(out, &s, &sync::Source::Incoming, &a.targets, &res, false);
    }
    let r = sync::sync(&s, !a.no_fetch, &res)?;
    if !r.conflicts.is_empty() {
        return Err(Conflicts {
            conflicts: r.conflicts,
            doing: "applied",
        }
        .into());
    }
    let local = sync::status(&s).map(|st| st.local).unwrap_or_default();
    let renumbered: Vec<Value> = r
        .renumbered
        .iter()
        .map(|(o, n)| json!({ "from": o, "to": n }))
        .collect();
    let human = match (r.applied.len(), r.published) {
        (0, 0) => "the board is up to date with the wipe branch".to_string(),
        (0, p) => format!("published {}", plural(p, "board commit")),
        (n, 0) => format!("applied {}", plural(n, "incoming change")),
        (n, p) => format!(
            "applied {} and published {}",
            plural(n, "incoming change"),
            plural(p, "board commit")
        ),
    };
    out.ok(
        human,
        json!({ "ok": true, "applied": r.applied, "published": r.published, "renumbered": renumbered, "uncommitted": local.len() }),
    );
    if !out.json {
        for l in lines(&r.applied) {
            println!("  {} {l}", dim("↓"));
        }
        for (o, n) in &r.renumbered {
            println!(
                "  {}",
                dim(&format!(
                    "renumbered your uncommitted {o} to {n} (the branch already used {o})"
                ))
            );
        }
        if !local.is_empty() {
            println!(
                "  {}",
                dim(&format!(
                    "{} still uncommitted on top: wipe commit -m \"...\"",
                    plural(local.len(), "local change")
                ))
            );
        }
    }
    Ok(())
}

fn report_status(out: &Out, st: &sync::Status) -> Result<()> {
    if out.json {
        out.json_value(&json!({
            "mode": st.mode,
            "branch": st.branch,
            "remote": st.remote,
            "board_dir": st.board_dir,
            "base": st.base,
            "upstream": st.upstream,
            "local": st.local,
            "incoming": st.incoming,
            "commits": st.commits,
            "unpublished": st.unpublished,
            "rewritten": st.rewritten,
            "fetched": st.fetched,
        }));
        return Ok(());
    }
    match st.mode {
        Mode::None => {
            println!("not a git repository - the board is plain files, with nothing to sync");
            return Ok(());
        }
        Mode::Unmigrated => {
            println!("this board isn't on the wipe branch yet - `wipe migrate` moves it there");
            return Ok(());
        }
        Mode::Tracked => {
            println!(
                "the checked-out branch still tracks {}/ - `wipe migrate` untracks it here",
                st.board_dir
            );
            return Ok(());
        }
        Mode::Branch => {}
    }
    let place = match &st.remote {
        Some(r) => format!("{r}/{}", st.branch),
        None => format!("{} (no remote)", st.branch),
    };
    println!("board: {} on {place}", st.board_dir);
    if st.local.is_empty() {
        println!("  no uncommitted changes");
    } else {
        println!(
            "  {} (wipe commit -m \"...\"):",
            plural(st.local.len(), "uncommitted change")
        );
        for l in lines(&st.local) {
            println!("    {l}");
        }
    }
    if st.incoming.is_empty() {
        println!("  up to date with {place}");
    } else {
        println!(
            "  {} (wipe sync):",
            plural(st.incoming.len(), "incoming change")
        );
        for l in lines(&st.incoming) {
            println!("    {l}");
        }
        for c in st.commits.iter().take(5) {
            println!(
                "    {}",
                dim(&format!(
                    "{} {} - {}",
                    c.short,
                    safe::line(&c.subject),
                    c.author_name
                ))
            );
        }
    }
    if st.rewritten {
        println!("  ! the branch was rewritten since your last sync - `wipe sync` merges from the common ancestor");
    }
    if st.unpublished > 0 {
        println!(
            "  {} not published yet (wipe sync)",
            plural(st.unpublished, "commit")
        );
    }
    Ok(())
}

/// `wipe migrate`
pub fn migrate(out: &Out, a: MigrateArgs) -> Result<()> {
    let s = store()?;
    let who = identity::resolve(None)?;
    let st = sync::status(&s)?;
    if st.mode == Mode::Branch {
        out.ok(
            format!("{} is already on the wipe branch", st.board_dir),
            json!({ "ok": true, "already": true }),
        );
        return Ok(());
    }
    let m = sync::migrate(
        &s,
        &who,
        &sync::MigrateOpts {
            publish: !a.no_push,
            code_commit: !a.no_code_commit,
            resolutions: resolutions(&a.resolve)?,
        },
    )?;
    if !m.conflicts.is_empty() {
        return Err(Conflicts {
            conflicts: m.conflicts,
            doing: "migrated",
        }
        .into());
    }
    let short: String = m.commit.chars().take(7).collect();
    let human = if m.adopted {
        format!("joined the wipe branch ({short}); this branch's own board edits are now uncommitted changes")
    } else {
        format!(
            "moved the board onto the `{}` branch ({short})",
            sync::BRANCH
        )
    };
    out.ok(
        human,
        json!({
            "ok": true,
            "commit": m.commit,
            "adopted": m.adopted,
            "published": m.published,
            "publish_error": m.publish_error,
            "code_commit": m.code_commit,
            "staged": m.staged,
        }),
    );
    if !out.json {
        match (&m.published, &m.publish_error) {
            (Some(r), _) => println!("  published to {r}/{}", sync::BRANCH),
            (None, Some(e)) => println!("  ! not published: {e}"),
            (None, None) if !m.adopted => {
                println!("  {}", dim("not published yet - `wipe sync` publishes it"))
            }
            _ => {}
        }
        if let Some(c) = &m.code_commit {
            println!(
                "  untracked {}/ on this branch ({}) - push it so others' checkouts stop tracking the board",
                st.board_dir,
                &c[..c.len().min(7)]
            );
        } else if m.staged {
            println!(
                "  untracking {}/ is staged on this branch - commit it with your next commit",
                st.board_dir
            );
        }
        println!(
            "  {}",
            dim("from now on: board edits stay local until `wipe commit`; `wipe sync` brings in others'")
        );
    }
    Ok(())
}

/// The board-sync lines for a status overview: `(human lines, json)`.
pub fn overview(store: &Store) -> Option<(Vec<String>, Value)> {
    let st = sync::status(store).ok()?;
    sync::fetch_in_background(store);
    let v = json!({
        "mode": st.mode,
        "uncommitted": st.local.len(),
        "incoming": st.incoming.len(),
        "unpublished": st.unpublished,
        "local_changes": lines(&st.local),
        "incoming_changes": lines(&st.incoming),
    });
    Some((notice_lines(&st), v))
}

fn notice_lines(st: &sync::Status) -> Vec<String> {
    let mut out = Vec::new();
    match st.mode {
        Mode::None => {}
        Mode::Unmigrated => out.push(
            "this board predates the wipe branch - run `wipe migrate` once to move it there (then `wipe commit` / `wipe sync`)"
                .to_string(),
        ),
        Mode::Tracked => out.push(format!(
            "this checkout still tracks {}/ from before the wipe branch - run `wipe migrate` to untrack it here",
            st.board_dir
        )),
        Mode::Branch => {
            if !st.local.is_empty() {
                out.push(format!(
                    "{} not committed ({}) - when the work is done, record it with `wipe commit -m \"<what and why>\"`",
                    plural(st.local.len(), "board change"),
                    summary(&st.local)
                ));
            }
            if !st.incoming.is_empty() {
                out.push(format!(
                    "{} on the wipe branch ({}) - apply with `wipe sync`",
                    plural(st.incoming.len(), "incoming board change"),
                    summary(&st.incoming)
                ));
            }
            if st.unpublished > 0 {
                out.push(format!(
                    "{} not published to {} - `wipe sync` publishes",
                    plural(st.unpublished, "board commit"),
                    st.remote.as_deref().unwrap_or("the remote")
                ));
            }
        }
    }
    out
}

/// After a command: the board-sync lines on stderr, and a background fetch
/// when the last one is stale. Best-effort and silent on failure.
pub fn after_command() {
    if std::env::var_os("WIPE_NO_UPDATES").is_some_and(|v| !v.is_empty() && v != "0") {
        return;
    }
    let Ok(store) = Store::discover(".") else {
        return;
    };
    let Ok(st) = sync::status(&store) else { return };
    sync::fetch_in_background(&store);
    let tag = "[wipe]".if_supports_color(Stream::Stderr, |t| t.yellow().to_string());
    for l in notice_lines(&st) {
        eprintln!("{tag} {l}");
    }
}

/// A target as typed: `T-004`, `F-2`, `list:<id>`, `label:<name>`, `board:<part>`.
fn entity(raw: &str) -> Result<sync::Entity> {
    let raw = raw.trim();
    let e = |scope: &str, id: &str, fields: Vec<String>| sync::Entity {
        scope: scope.into(),
        id: id.into(),
        fields,
    };
    if let Some((kind, rest)) = raw.split_once(':') {
        return Ok(match kind {
            "list" => e("list", rest, vec![]),
            "label" => e("label", rest, vec![]),
            "board" => e("board", "", vec![rest.to_string()]),
            other => bail!("unknown target kind `{other}:` - use list:, label: or board:"),
        });
    }
    if let Some(digits) = wipe_core::id::ticket_ref_digits(raw) {
        return Ok(e("ticket", &format!("T-{digits}"), vec![]));
    }
    if let Some(n) = raw.to_ascii_uppercase().strip_prefix("F-") {
        return Ok(e("thread", &format!("F-{n}"), vec![]));
    }
    bail!("`{raw}` isn't a ticket (T-004), thread (F-2), list:<id>, label:<name> or board:<part>")
}

fn take(
    out: &Out,
    s: &Store,
    source: &sync::Source,
    targets: &[String],
    res: &Resolutions,
    discard_local: bool,
) -> Result<()> {
    let entities = targets
        .iter()
        .map(|t| entity(t))
        .collect::<Result<Vec<_>>>()?;
    let t = sync::take(s, source, &entities, res, discard_local)?;
    if !t.discards.is_empty() {
        bail!(
            "{} {} uncommitted changes that restoring would discard - nothing was changed. Commit them first (wipe commit -m \"...\"), or restore anyway with --discard-local",
            t.discards.join(", "),
            if t.discards.len() == 1 { "has" } else { "have" }
        );
    }
    if !t.conflicts.is_empty() {
        return Err(Conflicts {
            conflicts: t.conflicts,
            doing: "applied",
        }
        .into());
    }
    let names: Vec<String> = entities
        .iter()
        .map(|e| {
            if e.id.is_empty() {
                e.fields.join(", ")
            } else {
                e.id.clone()
            }
        })
        .collect();
    let from = match source {
        sync::Source::Incoming => "the wipe branch".to_string(),
        sync::Source::Commit(c) => c.chars().take(7).collect(),
    };
    out.ok(
        format!("took {} from {from}", names.join(", ")),
        json!({ "ok": true, "taken": t.taken, "synced": t.synced, "renumbered": t.renumbered.iter().map(|(o, n)| json!({ "from": o, "to": n })).collect::<Vec<_>>() }),
    );
    if !out.json {
        for (o, n) in &t.renumbered {
            println!(
                "  {}",
                dim(&format!(
                    "renumbered your uncommitted {o} to {n} (the branch already used {o})"
                ))
            );
        }
        if t.synced {
            println!(
                "  {}",
                dim("nothing else was incoming - the board is up to date with the wipe branch")
            );
        }
        if matches!(source, sync::Source::Commit(_)) {
            println!(
                "  {}",
                dim("it shows as an uncommitted change: wipe commit -m \"...\" to keep it")
            );
        }
    }
    Ok(())
}

/// `wipe restore`
pub fn restore(out: &Out, a: RestoreArgs) -> Result<()> {
    let s = store()?;
    take(
        out,
        &s,
        &sync::Source::Commit(a.from),
        &a.targets,
        &Resolutions::default(),
        a.discard_local,
    )
}

/// `wipe log`
pub fn log(out: &Out, a: LogArgs) -> Result<()> {
    let s = store()?;
    let entries = sync::log(&s, a.limit)?;
    let changes = |hash: &str| -> Vec<Change> {
        if a.changes {
            sync::at(&s, hash).map(|x| x.changes).unwrap_or_default()
        } else {
            Vec::new()
        }
    };
    if out.json {
        let v: Vec<Value> = entries
            .iter()
            .map(|e| {
                let mut v = json!(e);
                if a.changes {
                    v["changes"] = json!(changes(&e.commit.hash));
                }
                v
            })
            .collect();
        out.json_value(&json!({ "commits": v }));
        return Ok(());
    }
    for e in &entries {
        if e.place == "base" {
            println!("{}", dim("── your board is here ──"));
        }
        let tag = match e.place {
            "incoming" => " [incoming: wipe sync]",
            "unpublished" => " [not published]",
            _ => "",
        };
        println!(
            "{} {}  {}{}",
            e.commit.short,
            safe::line(&e.commit.subject),
            dim(&format!(
                "{} · {}",
                e.commit.author_name,
                e.commit.date[..e.commit.date.len().min(16)].replace('T', " ")
            )),
            tag
        );
        for l in lines(&changes(&e.commit.hash)) {
            println!("    {l}");
        }
    }
    if !entries.is_empty() {
        println!("{}", dim("take something back as it was after a commit: wipe restore <T-id|list:id|...> --from <commit>"));
    }
    Ok(())
}

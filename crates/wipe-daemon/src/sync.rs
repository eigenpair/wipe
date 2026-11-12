//! The board-sync API behind the UI's "incoming changes" and "commit" flows:
//! where the board stands against the wipe branch (see [`wipe_core::sync`]), a
//! preview of the board after applying incoming changes, and the explicit
//! apply / commit / migrate actions. Git work runs on blocking threads - fetch
//! and push go over the network.

use axum::extract::{Query, State};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use wipe_core::sync::{self, Resolutions};

use crate::api::{
    board_json, notify, resolve_actor, store_for, ApiError, ApiResult, AppState, ProjectQuery,
};

/// The remote is fetched at most this often, however often the UI asks.
const FETCH_MIN_SECS: i64 = 20;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> wipe_core::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::from(anyhow::anyhow!("sync task failed: {e}")))?
        .map_err(ApiError::from)
}

/// `GET /api/sync` - mode, uncommitted (`local`) and `incoming` changes, as of
/// the last fetch. Cheap: cached until the board or the branch changes.
pub async fn status(State(state): State<AppState>, Query(q): Query<ProjectQuery>) -> ApiResult {
    let store = store_for(&state, q.project)?;
    let st = blocking(move || sync::status(&store)).await?;
    Ok(Json(json!(st)))
}

/// Query for `POST /api/sync/fetch`.
#[derive(Debug, Deserialize)]
pub struct FetchQuery {
    project: Option<String>,
    /// Fetch even if the last fetch was moments ago (a manual refresh).
    #[serde(default)]
    force: bool,
}

/// `POST /api/sync/fetch` - fetch the remote branch (throttled unless `force`),
/// then report as `GET /api/sync`. Offline is not an error: the status says
/// `fetch_error`.
pub async fn fetch(State(state): State<AppState>, Query(q): Query<FetchQuery>) -> ApiResult {
    let store = store_for(&state, q.project)?;
    let force = q.force;
    let (st, err) = blocking(move || {
        let st = sync::status(&store)?;
        let fresh = st
            .fetched
            .is_some_and(|t| (Utc::now() - t).num_seconds() < FETCH_MIN_SECS);
        if (fresh && !force) || st.remote.is_none() {
            return Ok((st, None));
        }
        Ok(match sync::refresh(&store) {
            Ok(st) => (st, None),
            Err(e) => (sync::status(&store)?, Some(e.to_string())),
        })
    })
    .await?;
    let mut v = json!(st);
    if let Some(e) = err {
        v["fetch_error"] = json!(e);
    }
    Ok(Json(v))
}

/// Body for the merge-bearing actions.
#[derive(Debug, Default, Deserialize)]
pub struct SyncBody {
    #[serde(default)]
    resolutions: Resolutions,
    /// Commit message (commit only).
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    actor: Option<String>,
}

fn renumbered(pairs: &[(String, String)]) -> Value {
    json!(pairs
        .iter()
        .map(|(o, n)| json!({ "from": o, "to": n }))
        .collect::<Vec<_>>())
}

/// `POST /api/sync/preview` - the board after applying the incoming changes
/// (shaped like `GET /api/board`), what comes in, and any conflicts (shown
/// with their side until resolved). Writes nothing.
pub async fn preview(
    State(state): State<AppState>,
    Query(q): Query<ProjectQuery>,
    body: Option<Json<SyncBody>>,
) -> ApiResult {
    let store = store_for(&state, q.project)?;
    let res = body.map(|Json(b)| b.resolutions).unwrap_or_default();
    let p = blocking(move || sync::preview(&store, &res)).await?;
    Ok(Json(json!({
        "board": board_json(&p.board, &p.lists),
        "incoming": p.incoming,
        "conflicts": p.conflicts,
        "renumbered": renumbered(&p.renumbered),
    })))
}

/// `POST /api/sync/apply` - apply the incoming changes as previewed (no new
/// fetch), with `resolutions` for any conflicts. `{ok:false, conflicts}` when
/// some are unresolved; nothing is written then.
pub async fn apply(
    State(state): State<AppState>,
    Query(q): Query<ProjectQuery>,
    body: Option<Json<SyncBody>>,
) -> ApiResult {
    let store = store_for(&state, q.project)?;
    let res = body.map(|Json(b)| b.resolutions).unwrap_or_default();
    let r = blocking(move || sync::sync(&store, false, &res)).await?;
    if !r.conflicts.is_empty() {
        return Ok(Json(json!({ "ok": false, "conflicts": r.conflicts })));
    }
    notify(&state);
    Ok(Json(json!({
        "ok": true,
        "applied": r.applied,
        "published": r.published,
        "renumbered": renumbered(&r.renumbered),
    })))
}

/// `POST /api/sync/commit` - commit the uncommitted changes to the wipe branch
/// and publish them (incoming changes are merged first). `{ok:false,
/// conflicts}` when that merge needs resolutions; nothing is written then.
pub async fn commit(
    State(state): State<AppState>,
    Query(q): Query<ProjectQuery>,
    body: Option<Json<SyncBody>>,
) -> ApiResult {
    let store = store_for(&state, q.project)?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let who = resolve_actor(&state, &store, b.actor);
    let author = who.clone();
    let c =
        blocking(move || sync::commit(&store, b.message.as_deref(), None, &author, &b.resolutions))
            .await?;
    if !c.conflicts.is_empty() {
        return Ok(Json(json!({ "ok": false, "conflicts": c.conflicts })));
    }
    notify(&state);
    Ok(Json(json!({
        "ok": true,
        "commit": c.commit,
        "short": c.short,
        "message": c.message,
        "author": who,
        "remote": c.remote,
        "changes": c.changes,
        "merged": c.merged,
        "renumbered": renumbered(&c.renumbered),
    })))
}

/// `POST /api/sync/migrate` - move a board from before 1.0 onto the wipe branch
/// (publishing it and committing the untracking on the code branch).
pub async fn migrate(
    State(state): State<AppState>,
    Query(q): Query<ProjectQuery>,
    body: Option<Json<SyncBody>>,
) -> ApiResult {
    let store = store_for(&state, q.project)?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let who = resolve_actor(&state, &store, b.actor);
    let m = blocking(move || {
        sync::migrate(
            &store,
            &who,
            &sync::MigrateOpts {
                publish: true,
                code_commit: true,
                resolutions: b.resolutions,
            },
        )
    })
    .await?;
    if !m.conflicts.is_empty() {
        return Ok(Json(json!({ "ok": false, "conflicts": m.conflicts })));
    }
    notify(&state);
    Ok(Json(json!({ "ok": true, "migrated": m })))
}

/// `GET /api/sync/log` - the wipe branch's commits, newest first, each placed
/// relative to your board (`incoming`, `unpublished`, `base`, `past`).
pub async fn log(State(state): State<AppState>, Query(q): Query<ProjectQuery>) -> ApiResult {
    let store = store_for(&state, q.project)?;
    let entries = blocking(move || sync::log(&store, 200)).await?;
    Ok(Json(json!({ "commits": entries })))
}

/// Query for `GET /api/sync/at`.
#[derive(Debug, Deserialize)]
pub struct AtQuery {
    project: Option<String>,
    rev: String,
}

/// `GET /api/sync/at` - the board right after a commit (shaped like `GET
/// /api/board`) and what that commit changed.
pub async fn at(State(state): State<AppState>, Query(q): Query<AtQuery>) -> ApiResult {
    let store = store_for(&state, q.project)?;
    let rev = q.rev;
    let a = blocking(move || sync::at(&store, &rev)).await?;
    Ok(Json(json!({
        "board": board_json(&a.board, &a.lists),
        "changes": a.changes,
    })))
}

/// Body for `POST /api/sync/take`.
#[derive(Debug, Deserialize)]
pub struct TakeBody {
    /// `"incoming"`, or `{ "commit": "<sha>" }`.
    source: sync::Source,
    entities: Vec<sync::Entity>,
    #[serde(default)]
    resolutions: Resolutions,
    /// Replace tickets / posts with uncommitted changes when taking from a
    /// past commit (otherwise answered with `{ok:false, discards}`).
    #[serde(default)]
    discard_local: bool,
}

/// `POST /api/sync/take` - bring single changes into your board: as they are
/// incoming, or as they were after a past commit (a revert). `{ok:false,
/// conflicts}` when one of them needs a resolution; nothing is written then.
pub async fn take(
    State(state): State<AppState>,
    Query(q): Query<ProjectQuery>,
    Json(b): Json<TakeBody>,
) -> ApiResult {
    let store = store_for(&state, q.project)?;
    let t = blocking(move || {
        sync::take(
            &store,
            &b.source,
            &b.entities,
            &b.resolutions,
            b.discard_local,
        )
    })
    .await?;
    if !t.conflicts.is_empty() {
        return Ok(Json(json!({ "ok": false, "conflicts": t.conflicts })));
    }
    if !t.discards.is_empty() {
        return Ok(Json(json!({ "ok": false, "discards": t.discards })));
    }
    notify(&state);
    Ok(Json(json!({
        "ok": true,
        "taken": t.taken,
        "synced": t.synced,
        "renumbered": renumbered(&t.renumbered),
    })))
}

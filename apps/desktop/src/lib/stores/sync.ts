// The board's standing against the wipe branch, and the explicit flows on top
// of it: preview + apply incoming changes (all, or one at a time), commit local
// ones, and browse the branch's history, taking single changes back from it.
import { writable, derived, get } from 'svelte/store';
import { api } from '$lib/api';
import { currentProject, showPreview, endPreview, loadBoard, health } from '$lib/stores/board';
import type {
  SyncChange,
  SyncConflict,
  SyncEntity,
  SyncLogEntry,
  SyncResolutions,
  SyncSide,
  SyncStatus
} from '$lib/types';

export const syncStatus = writable<SyncStatus | null>(null);
/** `idle`, previewing `incoming` changes, composing a `commit`, or viewing a
 *  past commit from the `history`. */
export const syncMode = writable<'idle' | 'incoming' | 'commit' | 'history'>('idle');
/** The commit the history view shows. */
export const historyCommit = writable<SyncLogEntry | null>(null);
/** The branch's commits (for the history drawer). */
export const syncLog = writable<SyncLogEntry[]>([]);
/** Changes taken one at a time in the open flow (`scope:id`), shown as applied. */
export const taken = writable<Set<string>>(new Set());
/** Conflicts the current flow needs settled, and the picks so far. */
export const conflicts = writable<SyncConflict[]>([]);
export const picks = writable<Record<string, SyncSide>>({});
/** What the open incoming preview brings in (from the preview, which may differ
 *  slightly from the status's list when ids were renumbered). */
export const previewChanges = writable<SyncChange[]>([]);
export const renumbered = writable<{ from: string; to: string }[]>([]);
export const syncBusy = writable<boolean>(false);
/** The incoming preview is being (re)computed. */
export const syncLoading = writable<boolean>(false);
export const syncError = writable<string | null>(null);
/** A short confirmation shown in the bar after a commit / apply. */
export const syncNotice = writable<string | null>(null);

/** The diff kinds the UI colors, strongest first: one per ticket. */
export type Mark = 'created' | 'deleted' | 'moved' | 'edited' | 'commented' | 'checked';
const RANK: Mark[] = ['created', 'deleted', 'moved', 'edited', 'commented', 'checked'];

function markOf(kind: string): Mark {
  if (kind === 'renamed' || kind === 'reordered') return 'edited';
  if (kind === 'posted') return 'commented';
  return (RANK as string[]).includes(kind) ? (kind as Mark) : 'edited';
}

export interface TicketMark {
  /** The strongest change kind (drives the color). */
  mark: Mark;
  changes: SyncChange[];
}

function marksFrom(changes: SyncChange[]): Map<string, TicketMark> {
  const m = new Map<string, TicketMark>();
  for (const c of changes) {
    if (c.scope !== 'ticket') continue;
    const k = markOf(c.kind);
    const cur = m.get(c.id);
    if (!cur) m.set(c.id, { mark: k, changes: [c] });
    else {
      cur.changes.push(c);
      if (RANK.indexOf(k) < RANK.indexOf(cur.mark)) cur.mark = k;
    }
  }
  return m;
}

/** Uncommitted changes per ticket - always shown, subtly. */
export const localMarks = derived(syncStatus, ($s) =>
  $s?.mode === 'branch' ? marksFrom($s.local) : new Map<string, TicketMark>()
);
/** The previewed changes per ticket: incoming ones, or a past commit's. */
export const incomingMarks = derived(previewChanges, ($c) => marksFrom($c));

/** A short phrase for one change, for card chips and the change list. */
export function phrase(c: SyncChange): string {
  switch (c.kind) {
    case 'created':
      return c.to ? `New in ${c.to}` : 'New';
    case 'deleted':
      return 'Deleted';
    case 'moved':
      return `Moved from ${c.from ?? '?'}`;
    case 'renamed':
      return 'Renamed';
    case 'edited':
      return c.fields?.length ? `Edited ${c.fields.join(', ')}` : 'Edited';
    case 'commented':
      return c.count > 1 ? `+${c.count} comments` : '+1 comment';
    case 'checked':
      return c.count > 1 ? `${c.count} items checked` : c.detail?.startsWith('[x]') ? 'Checked an item' : 'Unchecked an item';
    case 'posted':
      return c.count > 1 ? `+${c.count} posts` : '+1 post';
    case 'reordered':
      return 'Reordered';
    default:
      return c.kind;
  }
}

function project(): string | undefined {
  return get(currentProject) ?? undefined;
}

function resolutions(): SyncResolutions {
  return { picks: get(picks) };
}

let statusSeq = 0;
/** Refresh the status (cheap). With `fetch`, ask the daemon to fetch the remote first. */
export async function refreshSync(opts: { fetch?: boolean; force?: boolean } = {}): Promise<void> {
  const proj = project();
  const seq = ++statusSeq;
  try {
    const s = opts.fetch ? await api.syncFetch(proj, opts.force) : await api.syncStatus(proj);
    if (seq !== statusSeq || project() !== proj) return;
    syncStatus.set(s);
  } catch {
    /* sync status is best-effort; the board works without it */
  }
}

let timer: ReturnType<typeof setInterval> | null = null;
let tick = 0;
/** Poll the status every 3s and fetch the remote about once a minute. */
export function startSyncPolling(): void {
  stopSyncPolling();
  void refreshSync({ fetch: true });
  timer = setInterval(() => {
    if (!get(health)) return;
    tick = (tick + 1) % 20;
    void refreshSync({ fetch: tick === 0 });
  }, 3000);
}

export function stopSyncPolling(): void {
  if (timer) clearInterval(timer);
  timer = null;
}

function flash(msg: string): void {
  syncNotice.set(msg);
  setTimeout(() => {
    if (get(syncNotice) === msg) syncNotice.set(null);
  }, 4000);
}

function reset(): void {
  conflicts.set([]);
  picks.set({});
  previewChanges.set([]);
  renumbered.set([]);
  syncError.set(null);
  taken.set(new Set());
  historyCommit.set(null);
}

/** Open the incoming-changes preview: the board shows the result, read-only. */
export async function openIncoming(): Promise<void> {
  reset();
  syncMode.set('incoming');
  await loadPreview();
}

async function loadPreview(): Promise<void> {
  syncLoading.set(true);
  try {
    const p = await api.syncPreview(project(), resolutions());
    if (get(syncMode) !== 'incoming') return;
    previewChanges.set(p.incoming);
    renumbered.set(p.renumbered);
    // A conflict settled by a pick no longer comes back from the server; keep
    // it listed so the pick stays visible and can be changed.
    const chosen = get(picks);
    const kept = get(conflicts).filter((c) => chosen[c.id] && !p.conflicts.some((n) => n.id === c.id));
    conflicts.set([...kept, ...p.conflicts]);
    showPreview(p.board);
  } catch (e) {
    syncError.set(e instanceof Error ? e.message : String(e));
  } finally {
    syncLoading.set(false);
  }
}

/** Pick a side for one conflict (re-rendering the preview with it). */
export async function pick(id: string, side: SyncSide): Promise<void> {
  picks.update((p) => ({ ...p, [id]: side }));
  if (get(syncMode) === 'incoming') await loadPreview();
}

/** Pick a side for every conflict. */
export async function pickAll(side: SyncSide): Promise<void> {
  const all: Record<string, SyncSide> = {};
  for (const c of get(conflicts)) all[c.id] = side;
  picks.set(all);
  if (get(syncMode) === 'incoming') await loadPreview();
}

/** Whether every conflict of the current flow has a pick. */
export const unresolved = derived([conflicts, picks], ([$c, $p]) => $c.filter((c) => !$p[c.id]).length);

/** Leave any flow and return to the live board. */
export async function closeSync(): Promise<void> {
  syncMode.set('idle');
  reset();
  await endPreview();
}

/** Apply the previewed incoming changes. */
export async function applyIncoming(): Promise<void> {
  syncBusy.set(true);
  syncError.set(null);
  try {
    const r = await api.syncApply(project(), resolutions());
    if (!r.ok) {
      conflicts.set(r.conflicts ?? []);
      return;
    }
    const n = r.applied?.length ?? 0;
    await closeSync();
    flash(n ? `Applied ${n} incoming change${n === 1 ? '' : 's'}` : 'Up to date');
    await refreshSync();
  } catch (e) {
    syncError.set(e instanceof Error ? e.message : String(e));
  } finally {
    syncBusy.set(false);
  }
}

/** Open the commit composer: local changes are emphasized on the board. */
export function openCommit(): void {
  reset();
  syncMode.set('commit');
}

/** Commit and publish the local changes (incoming ones are merged first). */
export async function commitLocal(message: string): Promise<boolean> {
  syncBusy.set(true);
  syncError.set(null);
  try {
    const r = await api.syncCommit(project(), message, resolutions());
    if (!r.ok) {
      conflicts.set(r.conflicts ?? []);
      return false;
    }
    syncMode.set('idle');
    reset();
    await loadBoard();
    flash(r.short ? `Committed ${r.short}` : 'Nothing to commit');
    await refreshSync();
    return true;
  } catch (e) {
    syncError.set(e instanceof Error ? e.message : String(e));
    return false;
  } finally {
    syncBusy.set(false);
  }
}

/** Move a board from before 1.0 onto the wipe branch. */
export async function migrateBoard(): Promise<void> {
  syncBusy.set(true);
  syncError.set(null);
  try {
    const r = await api.syncMigrate(project());
    if (!r.ok) {
      syncError.set(r.conflicts?.length ? 'The move needs conflicts settled - run `wipe migrate` in a terminal' : (r.error ?? 'failed'));
      return;
    }
    flash('Board moved to the wipe branch');
    await refreshSync();
  } catch (e) {
    syncError.set(e instanceof Error ? e.message : String(e));
  } finally {
    syncBusy.set(false);
  }
}

// --- single changes and history -----------------------------------------------

/** What to send to take a change: board-level reorders take the list order. */
export function entityOf(c: SyncChange): SyncEntity {
  if (c.scope === 'board') return { scope: 'board', id: '', fields: c.kind === 'reordered' ? ['lists'] : (c.fields ?? []) };
  return { scope: c.scope, id: c.id, fields: [] };
}

/** A change's identity within a flow (`ticket:T-003`). */
export function keyOf(c: SyncChange): string {
  return `${c.scope}:${c.id}`;
}

/** Whether a change can be taken on its own. */
export function takeable(c: SyncChange): boolean {
  return !(c.scope === 'board' && (c.fields ?? []).includes('ticket ids'));
}

/** Take one change into your board: as it comes in, or (history) as it was
 *  after the viewed commit. Returns false when it needs a conflict settled. */
export async function takeChange(c: SyncChange): Promise<boolean> {
  const mode = get(syncMode);
  const commit = get(historyCommit);
  const source = mode === 'history' && commit ? { commit: commit.hash } : 'incoming';
  syncBusy.set(true);
  syncError.set(null);
  try {
    // Every change to the same entity goes together.
    const r = await api.syncTake(project(), source, [entityOf(c)], resolutions());
    if (!r.ok) {
      const add = (r.conflicts ?? []).filter((n) => !get(conflicts).some((o) => o.id === n.id));
      conflicts.update((cs) => [...cs, ...add]);
      syncError.set('Settle the conflict below, then apply again.');
      return false;
    }
    taken.update((s) => new Set(s).add(keyOf(c)));
    if (mode === 'incoming') {
      if (r.synced) {
        await closeSync();
        flash('All incoming changes applied');
      } else {
        await loadPreview();
      }
    }
    if (mode === 'history') flash(`Took ${c.id || 'the change'} from ${commit?.short} - commit it to keep it`);
    await refreshSync();
    return true;
  } catch (e) {
    syncError.set(e instanceof Error ? e.message : String(e));
    return false;
  } finally {
    syncBusy.set(false);
  }
}

/** Take every change of the viewed commit that isn't taken yet. */
export async function takeAll(): Promise<void> {
  for (const c of get(previewChanges)) {
    if (!takeable(c) || get(taken).has(keyOf(c))) continue;
    if (!(await takeChange(c))) return;
  }
}

/** Load the branch's history. */
export async function loadLog(): Promise<void> {
  try {
    syncLog.set(await api.syncLog(project()));
  } catch (e) {
    syncError.set(e instanceof Error ? e.message : String(e));
  }
}

/** Show the board right after `entry`, with that commit's changes highlighted. */
export async function openHistory(entry: SyncLogEntry): Promise<void> {
  if (entry.place === 'incoming') {
    await openIncoming();
    return;
  }
  reset();
  syncMode.set('history');
  historyCommit.set(entry);
  syncLoading.set(true);
  try {
    const a = await api.syncAt(project(), entry.hash);
    if (get(historyCommit)?.hash !== entry.hash) return;
    previewChanges.set(a.changes);
    showPreview(a.board);
  } catch (e) {
    syncError.set(e instanceof Error ? e.message : String(e));
  } finally {
    syncLoading.set(false);
  }
}

// Watching and notifications for humans in the board UI.
//
// You watch the whole board, a list, a ticket, or a forum thread (or the whole
// forum). Every refresh already diffs the previous board snapshot against the
// new one; this turns those diffs into events (new card, moved, commented,
// assigned, labeled, checklist/criteria done, edited) and the forum poll into
// "new reply" events, keeps the ones on things you watch that someone ELSE did,
// and shows them:
//   - as native system notifications (the browser's Notification API) while the
//     tab is hidden or unfocused - clicking one focuses the board and opens the
//     ticket or thread;
//   - as small in-app toasts while you're looking at the board.
// Being mentioned (`@you`, in a ticket, comment or post) notifies you whether
// you watch it or not - a mention is like an assignment.
// Watches are per board and per browser (localStorage). Native notifications
// need a secure origin (https, or http://localhost); elsewhere - e.g. a phone on
// the LAN URL - toasts are used.
import { writable, get } from 'svelte/store';
import type { Board, ForumPost, Ticket } from '$lib/types';

export type Watches = {
  board: boolean;
  forum: boolean;
  lists: string[];
  tickets: string[];
  threads: string[];
};

const EMPTY: Watches = { board: false, forum: false, lists: [], tickets: [], threads: [] };

function key(project: string | null): string {
  return `wipe:watch:${project ?? ''}`;
}

function load(project: string | null): Watches {
  try {
    const raw = localStorage.getItem(key(project));
    return raw ? { ...EMPTY, ...JSON.parse(raw) } : { ...EMPTY };
  } catch {
    return { ...EMPTY };
  }
}

/** The current board's watches. */
export const watches = writable<Watches>({ ...EMPTY });
let watchProject: string | null = null;

/** Switch to (and load) the watches of `project`. */
export function useWatchesOf(project: string | null): void {
  watchProject = project;
  watches.set(load(project));
}

function save(w: Watches): void {
  watches.set(w);
  try {
    localStorage.setItem(key(watchProject), JSON.stringify(w));
  } catch {
    /* best-effort */
  }
}

type Kind = 'lists' | 'tickets' | 'threads';
export function isWatching(w: Watches, kind: Kind, id: string): boolean {
  return w[kind].includes(id);
}
/** Toggle a watch; turning one on also asks for notification permission. */
export function toggleWatch(kind: Kind | 'board' | 'forum', id = ''): void {
  const w = { ...get(watches) };
  if (kind === 'board' || kind === 'forum') w[kind] = !w[kind];
  else w[kind] = w[kind].includes(id) ? w[kind].filter((x) => x !== id) : [...w[kind], id];
  save(w);
  void requestPermission();
}

// --- permission -----------------------------------------------------------------

export type Permission = NotificationPermission | 'unsupported';
export const permission = writable<Permission>(currentPermission());

function currentPermission(): Permission {
  if (typeof window === 'undefined' || !('Notification' in window) || !window.isSecureContext)
    return 'unsupported';
  return Notification.permission;
}

/** Ask the browser for permission (no-op when already decided or unsupported). */
export async function requestPermission(): Promise<Permission> {
  const p = currentPermission();
  if (p === 'default') {
    try {
      const r = await Notification.requestPermission();
      permission.set(r);
      return r;
    } catch {
      /* some browsers throw outside a user gesture */
    }
  }
  permission.set(p);
  return p;
}

// --- toasts ------------------------------------------------------------------------

export type Toast = {
  id: number;
  title: string;
  body: string;
  kind: EventKind;
  target: Target;
};
export const toasts = writable<Toast[]>([]);
let toastSeq = 0;

export function dismissToast(id: number): void {
  toasts.update((t) => t.filter((x) => x.id !== id));
}

// --- events ------------------------------------------------------------------------

export type EventKind =
  | 'created'
  | 'moved'
  | 'comment'
  | 'assigned'
  | 'label'
  | 'done'
  | 'edited'
  | 'reply';

/** What clicking a notification opens: a ticket, or a forum post (`thread`:
 *  a thread's opening post). */
export type Target = { ticket?: string; thread?: string; post?: string };

/** Clicking a notification/toast asks the page to open this. */
export const openRequest = writable<Target | null>(null);

type Event = { kind: EventKind; title: string; body: string; target: Target; tag: string };

const ICON: Record<EventKind, string> = {
  created: '✦',
  moved: '→',
  comment: '💬',
  assigned: '👤',
  label: '#',
  done: '✓',
  edited: '✎',
  reply: '↩'
};

function who(actor: string): string {
  return actor.replace(/\s*<[^>]+>$/, '') || actor;
}

function snippet(s: string, n = 140): string {
  const one = s.replace(/\s+/g, ' ').trim();
  return one.length > n ? `${one.slice(0, n - 1)}…` : one;
}

/** Turn one ticket's change into events (newest activity/comments beyond `prev`). */
function ticketEvents(t: Ticket, prev: Ticket | undefined, listName: string): Event[] {
  const head = `${t.id} · ${t.title}`;
  const target = { ticket: t.id };
  const out: Event[] = [];
  if (!prev) {
    const by = t.activity.find((a) => a.kind === 'created')?.actor ?? '';
    out.push({
      kind: 'created',
      title: `New card in ${listName}`,
      body: `${head}${by ? ` — by ${who(by)}` : ''}`,
      target,
      tag: t.id
    });
    return out;
  }
  for (const c of t.comments.slice(prev.comments.length)) {
    out.push({
      kind: 'comment',
      title: `${who(c.author)} commented on ${t.id}`,
      body: `${t.title}\n“${snippet(c.body)}”`,
      target,
      tag: `${t.id}:c`
    });
  }
  for (const a of t.activity.slice(prev.activity.length)) {
    const actor = who(a.actor);
    const kind: EventKind | null =
      a.kind === 'moved'
        ? 'moved'
        : a.kind === 'assigned'
          ? 'assigned'
          : a.kind === 'label-added'
            ? 'label'
            : a.kind === 'edited' || a.kind === 'renamed'
              ? 'edited'
              : null;
    if (!kind) continue;
    const what =
      kind === 'moved'
        ? `moved it to ${a.detail}`
        : kind === 'assigned'
          ? `assigned ${who(a.detail ?? '')}`
          : kind === 'label'
            ? `labeled it “${a.detail}”`
            : a.kind === 'renamed'
              ? 'renamed it'
              : 'edited the description';
    out.push({ kind, title: `${actor} ${what}`, body: head, target, tag: `${t.id}:${kind}` });
  }
  const doneNow = (xs: { done: boolean }[]) => xs.length > 0 && xs.every((x) => x.done);
  if (doneNow(t.acceptance) && !doneNow(prev.acceptance)) {
    out.push({
      kind: 'done',
      title: `All acceptance criteria met on ${t.id}`,
      body: t.title,
      target,
      tag: `${t.id}:ac`
    });
  } else if (doneNow(t.checklist) && !doneNow(prev.checklist)) {
    out.push({
      kind: 'done',
      title: `Checklist completed on ${t.id}`,
      body: t.title,
      target,
      tag: `${t.id}:ck`
    });
  }
  return out;
}

/** Mentions of `meId` that `t` gained over `prev` (new comments, or a title /
 *  description that now mentions you). */
function mentionEvents(t: Ticket, prev: Ticket | undefined, meId: string): Event[] {
  const out: Event[] = [];
  const target = { ticket: t.id };
  const mentioned = (s: string) => mentionsOf(s).some((m) => sameId(m, meId));
  for (const c of t.comments.slice(prev?.comments.length ?? 0)) {
    if (sameId(c.author, meId) || !mentioned(c.body)) continue;
    out.push({
      kind: 'assigned',
      title: `${who(c.author)} mentioned you on ${t.id}`,
      body: `${t.title}\n“${snippet(c.body)}”`,
      target,
      tag: `${t.id}:m`
    });
  }
  const now = mentioned(`${t.title}\n${t.body}`);
  const before = prev ? mentioned(`${prev.title}\n${prev.body}`) : false;
  if (now && !before) {
    const by = t.activity.at(-1)?.actor ?? '';
    if (!sameId(by, meId))
      out.push({
        kind: 'assigned',
        title: `${by ? who(by) : 'Someone'} mentioned you in ${t.id}`,
        body: t.title,
        target,
        tag: `${t.id}:m`
      });
  }
  return out;
}

/** Called with every new live board snapshot (same project as `prev`). `skip`
 *  holds ids the local user just changed - never notify about your own edits.
 *  `meId` is the local person's identity, for mentions. */
export function notifyBoardChanges(
  prev: Board,
  next: Board,
  skip: Set<string>,
  meId: string | null = null
): void {
  const w = get(watches);
  const watching = w.board || w.lists.length > 0 || w.tickets.length > 0;
  if (!watching && !meId) return;
  const before = new Map<string, Ticket>();
  const beforeList = new Map<string, string>();
  for (const l of prev.lists)
    for (const t of l.tickets) {
      before.set(t.id, t);
      beforeList.set(t.id, l.list);
    }
  const events: Event[] = [];
  for (const l of next.lists) {
    for (const t of l.tickets) {
      const p = before.get(t.id);
      if (p === t || skip.has(t.id)) continue;
      if (p && p.updated === t.updated) continue;
      const mentions = meId ? mentionEvents(t, p, meId) : [];
      if (mentions.length) {
        events.push(...mentions);
        continue;
      }
      // A card counts for a watched list while in it, and when it leaves it.
      const from = beforeList.get(t.id);
      const watched =
        w.board ||
        w.lists.includes(l.list) ||
        (from !== undefined && w.lists.includes(from)) ||
        w.tickets.includes(t.id);
      if (!watched) continue;
      events.push(...ticketEvents(t, p, l.name ?? l.list));
    }
  }
  emit(events);
}

const seenPosts = new Set<string>();
let forumBaselined: string | null = null;

/** Called with every forum load: notifies about new posts on watched threads,
 *  and about any new post that mentions you (`@you`), watched or not - a
 *  mention is like an assignment. */
export function notifyForum(project: string | null, posts: ForumPost[], meId: string | null): void {
  const first = forumBaselined !== project;
  if (first) {
    seenPosts.clear();
    forumBaselined = project;
  }
  const w = get(watches);
  const events: Event[] = [];
  const fresh = posts.filter((p) => !seenPosts.has(p.id));
  for (const p of posts) seenPosts.add(p.id);
  if (first) return;
  for (const p of fresh.reverse()) {
    if (meId && sameId(p.author, meId)) continue;
    if (meId && mentionsOf(p.body).some((m) => sameId(m, meId))) {
      events.push({
        kind: 'assigned',
        title: `${who(p.author)} mentioned you in ${p.id}`,
        body: `${p.summary}\n“${snippet(p.body)}”`,
        target: { post: p.id },
        tag: `${p.id}:m`
      });
      continue;
    }
    if (!(w.forum || w.threads.includes(p.thread_id))) continue;
    events.push({
      kind: 'reply',
      title:
        p.depth === 0
          ? `New thread by ${who(p.author)}`
          : `${who(p.author)} replied in ${p.thread_id}`,
      body: `${p.id} · ${p.summary}`,
      target: { post: p.id },
      tag: p.thread_id
    });
  }
  emit(events);
}

/** Whether an author / mention (`Name <email>`, `email`, `claude`) is `id`. */
export function sameId(a: string, id: string): boolean {
  const key = (x: string) => (x.match(/<([^>]+)>/)?.[1] ?? x).trim().toLowerCase();
  return key(a) === key(id);
}

/** The identities a text mentions (`@claude`, `@ada@example.com`). */
export function mentionsOf(text: string): string[] {
  const out: string[] = [];
  const re = /(^|[^\w@./+-])@([A-Za-z0-9][\w.+-]*(?:@[\w-]+(?:\.[\w-]+)+)?)/g;
  for (const m of text.matchAll(re)) {
    const id = m[2].replace(/[.+-]+$/, '');
    if (/^[TF]-[0-9A-F]+$/.test(id)) continue;
    out.push(id);
  }
  return out;
}

function emit(events: Event[]): void {
  if (!events.length) return;
  // A burst (an agent sweeping the board) collapses into one summary.
  const batch =
    events.length > 4
      ? [
          {
            kind: 'edited' as EventKind,
            title: `${events.length} updates on watched items`,
            body: events
              .slice(0, 4)
              .map((e) => e.title)
              .join('\n'),
            target: events[0].target,
            tag: 'wipe-batch'
          }
        ]
      : events;
  const foreground = typeof document !== 'undefined' && !document.hidden && document.hasFocus();
  const native = get(permission) === 'granted' && !foreground;
  for (const e of batch) {
    if (native) {
      try {
        const n = new Notification(`${ICON[e.kind]} ${e.title}`, {
          body: e.body,
          tag: e.tag,
          icon: '/favicon.svg'
        });
        n.onclick = () => {
          window.focus();
          openRequest.set(e.target);
          n.close();
        };
        continue;
      } catch {
        /* fall through to a toast */
      }
    }
    const id = ++toastSeq;
    toasts.update((t) => [...t.slice(-3), { id, title: e.title, body: e.body, kind: e.kind, target: e.target }]);
    setTimeout(() => dismissToast(id), 7000);
  }
}

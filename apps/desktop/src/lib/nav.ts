// Page links: the URL says what is open, so the browser's back/forward walk
// every jump - between tickets, posts and views - natively.
//
//   /             the board
//   /?at=T-023    the board, scrolled to T-023 (highlighted)
//   /T-023        ticket T-023 open (over the board)
//   /forum        the forum
//   /F-01A        the forum, at post F-01A in its thread (highlighted)
import { goto } from '$app/navigation';
import { base } from '$app/paths';
import type { RefKind } from '$lib/refs';

export type Route = {
  view: 'board' | 'forum';
  /** The open ticket (its popup). */
  ticket?: string;
  /** The forum post in focus. */
  post?: string;
  /** A board card to scroll to and highlight. */
  at?: string;
};

const TICKET = /^T-[0-9A-Fa-f]+$/;
const POST = /^F-[0-9A-Fa-f]+(?:\.\d+)*$/;

export function parseRoute(url: URL): Route {
  let path = decodeURIComponent(url.pathname);
  if (base && path.startsWith(base)) path = path.slice(base.length);
  const seg = path.replace(/^\/+|\/+$/g, '');
  const at = url.searchParams.get('at') ?? undefined;
  if (TICKET.test(seg)) return { view: 'board', ticket: seg.toUpperCase(), at };
  if (POST.test(seg)) return { view: 'forum', post: seg.replace(/^f/, 'F') };
  if (seg === 'forum') return { view: 'forum' };
  return { view: 'board', at };
}

export function pathOf(r: Route): string {
  if (r.ticket) return `${base}/${r.ticket}`;
  if (r.post) return `${base}/${r.post}`;
  if (r.view === 'forum') return `${base}/forum`;
  return r.at ? `${base}/?at=${encodeURIComponent(r.at)}` : `${base}/`;
}

/** Go somewhere (a new history entry unless `replace`). */
export function go(r: Route, replace = false): Promise<void> {
  return goto(pathOf(r), { noScroll: true, keepFocus: true, replaceState: replace });
}

export const openTicket = (id: string) => go({ view: 'board', ticket: id });
export const openPost = (id: string) => go({ view: 'forum', post: id });

/** Follow a reference from where it was clicked:
 *  - a ticket inside an open ticket switches the popup to it; on the board it
 *    scrolls the card into view and highlights it; elsewhere it opens it;
 *  - a post opens its thread at that post (scrolling there if it is open). */
export function follow(kind: RefKind, id: string, from: Route): Promise<void> | void {
  if (kind === 'ticket') {
    if (!from.ticket && from.view === 'board') return go({ view: 'board', at: id });
    return openTicket(id);
  }
  if (kind === 'post') return openPost(id);
}

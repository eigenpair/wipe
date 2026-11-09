// Threads from the flat forum: every post is its own entity, and a post replies
// to the posts its body opens with tags for (the daemon resolves those into
// `parents` and the thread's opening post into `thread_id`).
import type { ForumPost, ForumThreadSummary } from './types';

/** The thread rows for the sidebar: one per opening post, newest activity first. */
export function threadsOf(posts: ForumPost[]): ForumThreadSummary[] {
  const byThread = new Map<string, ForumPost[]>();
  for (const p of posts) {
    const list = byThread.get(p.thread_id);
    if (list) list.push(p);
    else byThread.set(p.thread_id, [p]);
  }
  const out: ForumThreadSummary[] = [];
  for (const p of posts) {
    if (p.depth !== 0) continue;
    const all = byThread.get(p.id) ?? [p];
    let last = p;
    for (const q of all) if (q.created > last.created) last = q;
    out.push({
      id: p.id,
      title: p.summary,
      author: p.author,
      labels: p.labels,
      posts: all.length,
      created: p.created,
      updated: last.created,
      last_author: last.author,
      snippet: oneLine(splitReplyTags(p.body).rest, 140),
      participants: [...new Set(all.map((q) => q.author))]
    });
  }
  out.sort((a, b) => (a.updated < b.updated ? 1 : a.updated > b.updated ? -1 : 0));
  return out;
}

/** A thread's posts as a tree: each post under its first parent, oldest first. */
export function childrenOf(posts: ForumPost[]): Map<string, ForumPost[]> {
  const kids = new Map<string, ForumPost[]>();
  const sorted = [...posts].sort((a, b) => (a.created < b.created ? -1 : 1));
  for (const p of sorted) {
    const parent = p.parents[0];
    if (!parent) continue;
    const list = kids.get(parent);
    if (list) list.push(p);
    else kids.set(parent, [p]);
  }
  return kids;
}

const TAG = /^@([TF]-[0-9A-F]+(?:\.\d+)*|[A-Za-z0-9][\w.+-]*(?:@[\w-]+(?:\.[\w-]+)+)?)/;

/** Split the tags a body opens with (`@F-01A @claude: ...`) from the rest. */
export function splitReplyTags(body: string): { tags: string[]; rest: string } {
  const tags: string[] = [];
  let rest = body.replace(/^\s+/, '');
  for (;;) {
    const m = rest.match(TAG);
    if (!m) break;
    const tail = rest.slice(m[0].length);
    if (tail && !/^[\s,:;]/.test(tail)) break;
    tags.push(m[1].replace(/[.+-]+$/, ''));
    rest = tail.replace(/^[,:;]?\s*/, '');
  }
  return { tags, rest };
}

/** Whitespace collapsed, cut to `n` characters. */
export function oneLine(s: string, n: number): string {
  const one = s.replace(/\s+/g, ' ').trim();
  return one.length > n ? `${one.slice(0, n - 1)}…` : one;
}

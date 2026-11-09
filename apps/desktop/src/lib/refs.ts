// `@` references: one syntax for everything on a board, highlighted anywhere
// text is shown (Markdown bodies, comments, titles, checklist items, posts).
//
//   @T-02A          a ticket
//   @F-01C          a forum post (`@F-3.1` on boards with the old thread forum)
//   @claude         an identity (a mention) - also `@ada@example.com`
//
// They render as `.wref` chips carrying `data-ref` / `data-kind`; RefPreview
// gives every chip its hover card and click behaviour through one delegated
// listener, so a page full of references costs nothing extra.
import { marked, type TokenizerAndRendererExtension } from 'marked';
import DOMPurify from 'dompurify';
import { writable, get } from 'svelte/store';
import { board, identities, me, postIndex } from '$lib/stores/board';

export type RefKind = 'ticket' | 'post' | 'identity';

const ID_RE = /^@([TF])-([0-9A-F]+)((?:\.\d+)*)/;
const NAME_RE = /^@([A-Za-z0-9][\w.+-]*(?:@[\w-]+(?:\.[\w-]+)+)?)/;

/** Parse one tag at the start of `src` (which starts with `@`). */
export function tagAt(src: string): { raw: string; kind: RefKind; id: string } | null {
  const m = src.match(ID_RE);
  if (m) {
    const after = src.charAt(m[0].length);
    if (!/[A-Za-z0-9_]/.test(after)) {
      if (m[1] === 'T' && !m[3]) return { raw: m[0], kind: 'ticket', id: `T-${m[2]}` };
      if (m[1] === 'F') return { raw: m[0], kind: 'post', id: `F-${m[2]}${m[3]}` };
    }
  }
  const n = src.match(NAME_RE);
  if (!n) return null;
  const id = n[1].replace(/[.+-]+$/, '');
  return id ? { raw: `@${id}`, kind: 'identity', id } : null;
}

/** Whether a tag may start right after `prev` (not inside a word or address). */
function boundary(prev: string): boolean {
  return prev === '' || !/[\w@./+-]/.test(prev);
}

export type Segment = { text: string } | { kind: RefKind; id: string; raw: string };

/** Split plain text into text and reference segments. */
export function segments(text: string): Segment[] {
  const out: Segment[] = [];
  let last = 0;
  for (let i = text.indexOf('@'); i >= 0; i = text.indexOf('@', i + 1)) {
    if (!boundary(i ? text[i - 1] : '')) continue;
    const t = tagAt(text.slice(i));
    if (!t) continue;
    if (i > last) out.push({ text: text.slice(last, i) });
    out.push(t);
    last = i + t.raw.length;
    i = last - 1;
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}

// --- resolving what a reference points at (for display) ---------------------

/** Bumped whenever what references resolve to changes (a ticket or post
 *  appears, an identity is renamed), so rendered Markdown refreshes. */
export const refsVersion = writable(0);

let ticketIds = new Set<string>();
let names = new Map<string, string>();
let meKey = '';
let sig = '';

function refresh() {
  const b = get(board);
  ticketIds = new Set(b?.lists.flatMap((l) => l.tickets.map((t) => t.id)) ?? []);
  names = new Map(get(identities).map((i) => [i.id.toLowerCase(), i.display_name]));
  meKey = (get(me)?.id ?? '').toLowerCase();
  const posts = get(postIndex);
  const next = [
    [...ticketIds].join(','),
    [...names].map(([k, v]) => `${k}=${v}`).join(','),
    meKey,
    posts.size
  ].join('|');
  if (next !== sig) {
    sig = next;
    refsVersion.update((n) => n + 1);
  }
}
if (typeof window !== 'undefined') {
  board.subscribe(refresh);
  identities.subscribe(refresh);
  me.subscribe(refresh);
  postIndex.subscribe(refresh);
}

const esc = (s: string) =>
  s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]!);

/** Whether a reference resolves to something on this board. */
export function exists(kind: RefKind, id: string): boolean {
  if (kind === 'ticket') return ticketIds.has(id);
  if (kind === 'post') return get(postIndex).has(id);
  return true;
}

/** The name an identity mention shows (`@Ada`). */
export function nameOf(id: string): string {
  return names.get(id.toLowerCase()) ?? id;
}

/** Whether an identity mention is you. */
export function isMe(id: string): boolean {
  return !!meKey && id.toLowerCase() === meKey;
}

/** The HTML of one reference chip. */
export function refHtml(kind: RefKind, id: string): string {
  const cls = ['wref', `wref-${kind}`];
  if (kind !== 'identity' && !exists(kind, id)) cls.push('wref-missing');
  if (kind === 'identity' && isMe(id)) cls.push('wref-me');
  const label = kind === 'identity' ? `@${nameOf(id)}` : id;
  const href = kind === 'identity' ? '' : ` href="/${esc(id)}"`;
  return `<a class="${cls.join(' ')}" data-ref="${esc(id)}" data-kind="${kind}"${href}>${esc(label)}</a>`;
}

// --- Markdown ----------------------------------------------------------------

const wref: TokenizerAndRendererExtension = {
  name: 'wref',
  level: 'inline',
  start(src: string) {
    for (let i = src.indexOf('@'); i >= 0; i = src.indexOf('@', i + 1)) {
      if (boundary(i ? src[i - 1] : '') && tagAt(src.slice(i))) return i;
    }
    return undefined;
  },
  tokenizer(src: string) {
    if (src[0] !== '@') return undefined;
    const t = tagAt(src);
    if (!t) return undefined;
    return { type: 'wref', raw: t.raw, kind: t.kind, id: t.id };
  },
  renderer(token) {
    return refHtml(token.kind as RefKind, token.id as string);
  }
};

marked.use({ extensions: [wref], gfm: true, breaks: true });

const PURIFY = { ADD_ATTR: ['target'] };

/** A Markdown body as sanitized HTML, references as chips. */
export function renderMarkdown(src: string): string {
  if (!src) return '';
  const html = marked.parse(src, { async: false }) as string;
  return typeof window === 'undefined' ? html : DOMPurify.sanitize(html, PURIFY);
}

/** One line of inline Markdown (titles, checklist items, summaries): bold,
 *  italic, underline (`<u>`), code, links and references - no blocks. */
export function renderInline(src: string): string {
  if (!src) return '';
  const html = marked.parseInline(src, { async: false }) as string;
  return typeof window === 'undefined' ? html : DOMPurify.sanitize(html, PURIFY);
}

/** Inline Markdown reduced to plain text (for tooltips, notifications). */
export function plainInline(src: string): string {
  return src
    .replace(/<\/?u>/g, '')
    .replace(/(\*\*|__|\*|_|~~|`)(.+?)\1/g, '$2')
    .replace(/\[([^\]]+)\]\([^)]+\)/g, '$1');
}

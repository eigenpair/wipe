// `@` suggestions while typing: identities first, then tickets and posts by
// what they say (titles and summaries above bodies) - or, once you type `T` or
// `F` and a digit, by id alone (the dash is put in for you). The suggestions
// show in one small menu at the caret (MentionMenu), driven by the keyboard.
import { writable, get } from 'svelte/store';
import { allTickets, identities, forumPosts, me } from '$lib/stores/board';
import type { RefKind } from '$lib/refs';

export type Suggestion = {
  kind: RefKind;
  id: string;
  label: string;
  hint?: string;
};

export type MenuState = {
  x: number;
  y: number;
  items: Suggestion[];
  index: number;
};

/** The open menu (null: closed). */
export const mentionMenu = writable<MenuState | null>(null);

let pickHandler: ((s: Suggestion) => void) | null = null;

/** Choose an item of the open menu (mouse). */
export function pickMention(s: Suggestion) {
  pickHandler?.(s);
}

const LIMIT = 8;

function score(text: string, q: string): number {
  const t = text.toLowerCase();
  if (!q) return 1;
  if (t.startsWith(q)) return 3;
  if (new RegExp(`\\b${q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`).test(t)) return 2;
  return t.includes(q) ? 1 : 0;
}

/** What `@query` suggests. */
export function suggest(query: string): Suggestion[] {
  const q = query.toLowerCase();
  const tickets = get(allTickets);
  const posts = get(forumPosts);
  // Id mode: `T5`, `t-01`, `F2a`.
  const idm = query.match(/^([TtFf])-?([0-9A-Fa-f]*)$/);
  if (idm && (idm[2] || query.includes('-'))) {
    const kind = idm[1].toUpperCase();
    const digits = idm[2].toUpperCase();
    const n = digits ? parseInt(digits, 16) : NaN;
    const rank = (id: string) => {
      const d = id.slice(2).split('.')[0];
      if (!digits) return 1;
      if (parseInt(d, 16) === n) return 4;
      if (d.replace(/^0+/, '').startsWith(digits.replace(/^0+/, '') || '0')) return 3;
      return d.includes(digits) ? 2 : 0;
    };
    const pool: Suggestion[] =
      kind === 'T'
        ? tickets.map((t) => ({ kind: 'ticket', id: t.id, label: t.title }))
        : posts.map((p) => ({ kind: 'post', id: p.id, label: p.summary }));
    return pool
      .map((s) => ({ s, r: rank(s.id) }))
      .filter((x) => x.r > 0)
      .sort((a, b) => b.r - a.r || (a.s.id < b.s.id ? 1 : -1))
      .slice(0, LIMIT)
      .map((x) => x.s);
  }

  const meId = get(me)?.id;
  const people: Suggestion[] = get(identities)
    .map((i) => ({
      s: {
        kind: 'identity' as const,
        id: i.id,
        label: i.display_name,
        hint: i.kind === 'agent' ? 'agent' : i.id === meId ? 'you' : undefined
      },
      r: Math.max(score(i.display_name, q), score(i.id, q))
    }))
    .filter((x) => x.r > 0)
    .sort((a, b) => b.r - a.r || a.s.label.localeCompare(b.s.label))
    .slice(0, q ? 4 : 5)
    .map((x) => x.s);

  // Content: titles and summaries count three times a body hit.
  const content: { s: Suggestion; r: number; t: string }[] = [];
  if (q.length >= 2) {
    for (const t of tickets) {
      const r = score(t.title, q) * 3 + ((t.body ?? '').toLowerCase().includes(q) ? 1 : 0);
      if (r) content.push({ s: { kind: 'ticket', id: t.id, label: t.title }, r, t: t.updated });
    }
    for (const p of posts) {
      const r = score(p.summary, q) * 3 + (p.body.toLowerCase().includes(q) ? 1 : 0);
      if (r) content.push({ s: { kind: 'post', id: p.id, label: p.summary }, r, t: p.created });
    }
  } else if (!q) {
    // Nothing typed yet: the latest tickets and posts.
    for (const t of tickets) content.push({ s: { kind: 'ticket', id: t.id, label: t.title }, r: 0, t: t.updated });
    for (const p of posts.slice(0, 20))
      content.push({ s: { kind: 'post', id: p.id, label: p.summary }, r: 0, t: p.created });
  }
  content.sort((a, b) => b.r - a.r || (a.t < b.t ? 1 : -1));
  return [...people, ...content.map((c) => c.s)].slice(0, LIMIT);
}

// --- caret position --------------------------------------------------------

const MIRROR = [
  'boxSizing', 'width', 'height', 'overflowX', 'overflowY', 'borderTopWidth', 'borderRightWidth',
  'borderBottomWidth', 'borderLeftWidth', 'paddingTop', 'paddingRight', 'paddingBottom',
  'paddingLeft', 'fontStyle', 'fontVariant', 'fontWeight', 'fontStretch', 'fontSize',
  'lineHeight', 'fontFamily', 'textAlign', 'textTransform', 'textIndent', 'letterSpacing',
  'wordSpacing', 'tabSize', 'whiteSpace', 'wordBreak', 'overflowWrap'
] as const;

/** Screen position of the caret at `pos` in a field (a hidden mirror element
 *  lays the text out the same way). */
function caretPoint(el: HTMLInputElement | HTMLTextAreaElement, pos: number) {
  const cs = getComputedStyle(el);
  const div = document.createElement('div');
  for (const p of MIRROR) div.style[p] = cs[p];
  div.style.position = 'fixed';
  div.style.visibility = 'hidden';
  div.style.top = '0';
  div.style.left = '-9999px';
  div.style.whiteSpace = el instanceof HTMLTextAreaElement ? 'pre-wrap' : 'pre';
  div.textContent = el.value.slice(0, pos);
  const mark = document.createElement('span');
  mark.textContent = '​';
  div.appendChild(mark);
  document.body.appendChild(div);
  const r = el.getBoundingClientRect();
  const lh = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.3;
  const x = r.left + mark.offsetLeft - el.scrollLeft;
  const y = r.top + mark.offsetTop - el.scrollTop + lh;
  div.remove();
  return { x: Math.min(x, window.innerWidth - 300), y };
}

// --- the action --------------------------------------------------------------

/** The `@query` being typed at the caret, if any: where its `@` is. */
function queryAt(v: string, caret: number): { at: number; query: string } | null {
  const before = v.slice(0, caret);
  const m = before.match(/(^|[^\w@./+-])@([\w.+-]*(?:@[\w.-]*)?)$/);
  if (!m) return null;
  return { at: caret - m[2].length - 1, query: m[2] };
}

/** Svelte action: `@` suggestions in an input or textarea. */
export function mention(node: HTMLInputElement | HTMLTextAreaElement) {
  let at = -1;
  let open = false;

  function close() {
    if (!open) return;
    open = false;
    at = -1;
    mentionMenu.set(null);
    pickHandler = null;
  }

  function update() {
    const caret = node.selectionStart ?? 0;
    if (caret !== node.selectionEnd) return close();
    const q = queryAt(node.value, caret);
    if (!q) return close();
    const items = suggest(q.query);
    if (!items.length) return close();
    at = q.at;
    open = true;
    const p = caretPoint(node, q.at);
    const prev = get(mentionMenu);
    mentionMenu.set({ x: p.x, y: p.y + 4, items, index: prev && prev.items === items ? prev.index : 0 });
    pickHandler = pick;
  }

  function pick(s: Suggestion) {
    const caret = node.selectionStart ?? 0;
    if (at < 0) return;
    node.focus();
    node.setSelectionRange(at, caret);
    const text = `@${s.id} `;
    if (!document.execCommand?.('insertText', false, text)) {
      node.setRangeText(text, at, caret, 'end');
      node.dispatchEvent(new Event('input', { bubbles: true }));
    }
    close();
  }

  function onInput(e: Event) {
    // `@T5` becomes `@T-5` as you type: ids always carry their dash.
    const caret = node.selectionStart ?? 0;
    const v = node.value;
    const m = v.slice(0, caret).match(/(^|[^\w@./+-])@([TtFf])(\d)$/);
    if (m && (e as InputEvent).inputType?.startsWith('insert')) {
      node.setSelectionRange(caret - 1, caret);
      if (!document.execCommand?.('insertText', false, `-${m[3]}`)) {
        node.setRangeText(`-${m[3]}`, caret - 1, caret, 'end');
        node.dispatchEvent(new Event('input', { bubbles: true }));
      }
      return;
    }
    update();
  }

  function onKey(e: KeyboardEvent) {
    const m = get(mentionMenu);
    if (!open || !m) return;
    const move = (d: number) => {
      e.preventDefault();
      e.stopImmediatePropagation();
      mentionMenu.set({ ...m, index: (m.index + d + m.items.length) % m.items.length });
    };
    if (e.key === 'ArrowDown') move(1);
    else if (e.key === 'ArrowUp') move(-1);
    else if ((e.key === 'Enter' || e.key === 'Tab') && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      e.stopImmediatePropagation();
      pick(m.items[m.index]);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopImmediatePropagation();
      close();
    }
  }

  const onBlur = () => setTimeout(close, 120);
  const onCaret = () => open && update();

  node.addEventListener('input', onInput);
  // Capture: runs before the field's own Enter handling (send, list continue).
  node.addEventListener('keydown', onKey as EventListener, true);
  node.addEventListener('click', onCaret);
  node.addEventListener('keyup', (e: Event) => {
    const k = (e as KeyboardEvent).key;
    if (k === 'ArrowLeft' || k === 'ArrowRight' || k === 'Home' || k === 'End') onCaret();
  });
  node.addEventListener('blur', onBlur);
  return {
    destroy() {
      close();
      node.removeEventListener('input', onInput);
      node.removeEventListener('keydown', onKey as EventListener, true);
      node.removeEventListener('click', onCaret);
      node.removeEventListener('blur', onBlur);
    }
  };
}

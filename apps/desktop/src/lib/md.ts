// Markdown editing for every text field: Ctrl/Cmd+B / I / U (and E for code,
// K for a link) on inputs and textareas, plus the helpers the formatting bar
// uses. Edits go through `insertText`, so the browser's undo still works.

type Field = HTMLInputElement | HTMLTextAreaElement;

/** Replace the selection with `text` and select [selStart, selEnd) after it
 *  (offsets relative to where `text` starts), keeping native undo. */
function insert(el: Field, text: string, selStart?: number, selEnd?: number) {
  const from = el.selectionStart ?? 0;
  el.focus();
  // execCommand keeps the edit on the undo stack; setRangeText is the fallback.
  const ok = document.execCommand?.('insertText', false, text);
  if (!ok) {
    el.setRangeText(text, from, el.selectionEnd ?? from, 'end');
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }
  if (selStart !== undefined) el.setSelectionRange(from + selStart, from + (selEnd ?? selStart));
}

/** Wrap the selection in `before`/`after` - or unwrap it if it already is. */
export function wrap(el: Field, before: string, after = before, placeholder = '') {
  const s = el.selectionStart ?? 0;
  const e = el.selectionEnd ?? s;
  const v = el.value;
  const sel = v.slice(s, e);
  // Toggle off: the markers sit just outside or just inside the selection.
  if (v.slice(s - before.length, s) === before && v.slice(e, e + after.length) === after) {
    el.setSelectionRange(s - before.length, e + after.length);
    insert(el, sel, 0, sel.length);
    return;
  }
  if (sel.startsWith(before) && sel.endsWith(after) && sel.length >= before.length + after.length) {
    const inner = sel.slice(before.length, sel.length - after.length);
    insert(el, inner, 0, inner.length);
    return;
  }
  const inner = sel || placeholder;
  insert(el, `${before}${inner}${after}`, before.length, before.length + inner.length);
}

/** Put `prefix` in front of every selected line (or a numbered `1.`, `2.`). */
export function prefixLines(el: HTMLTextAreaElement, prefix: string | 'numbered') {
  const v = el.value;
  const s = el.selectionStart ?? 0;
  const e = el.selectionEnd ?? s;
  const start = v.lastIndexOf('\n', s - 1) + 1;
  const endNl = v.indexOf('\n', e);
  const end = endNl < 0 ? v.length : endNl;
  const lines = v.slice(start, end).split('\n');
  const has = (l: string) =>
    prefix === 'numbered' ? /^\d+\.\s/.test(l) : l.startsWith(prefix as string);
  const all = lines.every(has);
  const next = lines.map((l, i) => {
    if (all) return prefix === 'numbered' ? l.replace(/^\d+\.\s/, '') : l.slice((prefix as string).length);
    return prefix === 'numbered' ? `${i + 1}. ${l}` : `${prefix}${l}`;
  });
  el.setSelectionRange(start, end);
  const text = next.join('\n');
  insert(el, text, 0, text.length);
}

/** Insert a block (code fence, table) on lines of its own, selecting `select`. */
export function insertBlock(el: HTMLTextAreaElement, block: string, select?: [number, number]) {
  const v = el.value;
  const s = el.selectionStart ?? 0;
  const before = s > 0 && v[s - 1] !== '\n' ? '\n' : '';
  const after = v[el.selectionEnd ?? s] && v[el.selectionEnd ?? s] !== '\n' ? '\n' : '';
  const text = `${before}${block}${after}`;
  if (select) insert(el, text, before.length + select[0], before.length + select[1]);
  else insert(el, text);
}

export function codeBlock(el: HTMLTextAreaElement) {
  const sel = el.value.slice(el.selectionStart ?? 0, el.selectionEnd ?? 0);
  if (sel.includes('\n') || !sel) {
    const body = sel || 'code';
    insertBlock(el, `\`\`\`\n${body}\n\`\`\``, [4, 4 + body.length]);
  } else wrap(el, '`');
}

export function table(el: HTMLTextAreaElement) {
  insertBlock(el, '| Column | Column |\n| --- | --- |\n| Cell | Cell |', [2, 8]);
}

export function link(el: Field) {
  const s = el.selectionStart ?? 0;
  const e = el.selectionEnd ?? s;
  const sel = el.value.slice(s, e);
  if (/^https?:\/\/\S+$/.test(sel)) insert(el, `[link](${sel})`, 1, 5);
  else {
    const text = sel || 'text';
    insert(el, `[${text}](url)`, text.length + 3, text.length + 6);
  }
}

/** Svelte action: Markdown shortcuts on an input or textarea. Ctrl/Cmd + B
 *  bold, I italic, U underline, E inline code, K link (+ Shift+X strike). */
export function mdkeys(node: Field) {
  function onKey(e: KeyboardEvent) {
    if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
    const k = e.key.toLowerCase();
    const act: Record<string, () => void> = {
      b: () => wrap(node, '**'),
      i: () => wrap(node, '*'),
      u: () => wrap(node, '<u>', '</u>'),
      e: () => wrap(node, '`'),
      k: () => link(node)
    };
    if (e.shiftKey && k === 'x') {
      e.preventDefault();
      wrap(node, '~~');
      return;
    }
    if (e.shiftKey || !act[k]) return;
    e.preventDefault();
    act[k]();
  }
  node.addEventListener('keydown', onKey as EventListener);
  return { destroy: () => node.removeEventListener('keydown', onKey as EventListener) };
}

/** Continue a list on Enter in a textarea (`- `, `1. `, `- [ ] `); Enter on an
 *  empty item ends the list. Svelte action. */
export function mdlists(node: HTMLTextAreaElement) {
  function onKey(e: KeyboardEvent) {
    if (e.key !== 'Enter' || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey || e.isComposing) return;
    if (e.defaultPrevented) return;
    const v = node.value;
    const s = node.selectionStart ?? 0;
    if (s !== node.selectionEnd) return;
    const start = v.lastIndexOf('\n', s - 1) + 1;
    const line = v.slice(start, s);
    const m = line.match(/^(\s*)([-*+] \[[ xX]\] |[-*+] |(\d+)\. )(.*)$/);
    if (!m) return;
    e.preventDefault();
    if (!m[4].trim()) {
      // An empty item ends the list.
      node.setSelectionRange(start, s);
      insert(node, '');
      return;
    }
    const marker = m[3] ? `${Number(m[3]) + 1}. ` : m[2].replace(/\[[xX]\]/, '[ ]');
    insert(node, `\n${m[1]}${marker}`);
  }
  node.addEventListener('keydown', onKey as EventListener);
  return { destroy: () => node.removeEventListener('keydown', onKey as EventListener) };
}

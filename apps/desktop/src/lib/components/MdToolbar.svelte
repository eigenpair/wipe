<script lang="ts">
  // A minimal Markdown formatting bar for larger text fields (descriptions,
  // posts, comments): inline styles, lists, quote, code, table, and `@` to start
  // a reference. Acts on the bound textarea and keeps its undo history.
  import {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Code,
    Link,
    List,
    ListOrdered,
    ListChecks,
    Quote,
    SquareCode,
    Table,
    AtSign
  } from 'lucide-svelte';
  import { wrap, link, prefixLines, codeBlock, table } from '$lib/md';

  let { target }: { target: HTMLTextAreaElement | null } = $props();

  const mac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
  const mod = mac ? '⌘' : 'Ctrl+';

  function act(fn: (el: HTMLTextAreaElement) => void) {
    return (e: MouseEvent) => {
      e.preventDefault();
      if (target) fn(target);
    };
  }
  const tools = [
    { icon: Bold, title: `Bold (${mod}B)`, run: (el: HTMLTextAreaElement) => wrap(el, '**', '**', 'bold') },
    { icon: Italic, title: `Italic (${mod}I)`, run: (el: HTMLTextAreaElement) => wrap(el, '*', '*', 'italic') },
    { icon: Underline, title: `Underline (${mod}U)`, run: (el: HTMLTextAreaElement) => wrap(el, '<u>', '</u>', 'underlined') },
    { icon: Strikethrough, title: `Strikethrough (${mod}Shift+X)`, run: (el: HTMLTextAreaElement) => wrap(el, '~~', '~~', 'struck') },
    { icon: Code, title: `Inline code (${mod}E)`, run: (el: HTMLTextAreaElement) => wrap(el, '`', '`', 'code') },
    { icon: Link, title: `Link (${mod}K)`, run: link },
    null,
    { icon: List, title: 'Bulleted list', run: (el: HTMLTextAreaElement) => prefixLines(el, '- ') },
    { icon: ListOrdered, title: 'Numbered list', run: (el: HTMLTextAreaElement) => prefixLines(el, 'numbered') },
    { icon: ListChecks, title: 'Task list', run: (el: HTMLTextAreaElement) => prefixLines(el, '- [ ] ') },
    { icon: Quote, title: 'Quote', run: (el: HTMLTextAreaElement) => prefixLines(el, '> ') },
    null,
    { icon: SquareCode, title: 'Code block', run: codeBlock },
    { icon: Table, title: 'Table', run: table },
    {
      icon: AtSign,
      title: 'Reference a ticket, post or person (@)',
      run: (el: HTMLTextAreaElement) => {
        const s = el.selectionStart ?? 0;
        const pad = s > 0 && !/\s/.test(el.value[s - 1]) ? ' @' : '@';
        el.focus();
        if (!document.execCommand?.('insertText', false, pad)) {
          el.setRangeText(pad, s, el.selectionEnd ?? s, 'end');
          el.dispatchEvent(new Event('input', { bubbles: true }));
        }
      }
    }
  ];
</script>

<div class="mdbar" role="toolbar" aria-label="Formatting">
  {#each tools as t, i (i)}
    {#if t}
      <button type="button" class="tb" title={t.title} aria-label={t.title} onmousedown={act(t.run)}>
        <t.icon size={14} />
      </button>
    {:else}
      <span class="sep"></span>
    {/if}
  {/each}
</div>

<style>
  .mdbar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 1px;
    padding: 2px;
  }
  .tb {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 24px;
    border: none;
    border-radius: var(--wp-r-sm);
    background: none;
    color: var(--wp-text-subtle);
    cursor: pointer;
    transition:
      background var(--wp-fast) var(--wp-ease),
      color var(--wp-fast) var(--wp-ease);
  }
  .tb:hover {
    color: var(--wp-text);
    background: var(--wp-elevated);
  }
  .sep {
    width: 1px;
    height: 14px;
    margin: 0 4px;
    background: var(--wp-border);
  }
</style>

<script lang="ts">
  // wipe's own release notes (`wipe changelog`), newest first: the running
  // version is marked, older ones fold away behind their headings.
  import { fade, scale } from 'svelte/transition';
  import { X } from 'lucide-svelte';
  import { marked } from 'marked';
  import DOMPurify from 'dompurify';
  import { api } from '$lib/api';
  import type { Changelog } from '$lib/types';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  let data = $state<Changelog | null>(null);
  let error = $state('');
  let query = $state('');
  const dur = 140;

  $effect(() => {
    if (open && !data) {
      api
        .changelog()
        .then((d) => (data = d))
        .catch((e) => (error = String(e?.message ?? e)));
    }
  });

  function html(md: string): string {
    const out = marked.parse(md, { async: false }) as string;
    return typeof window === 'undefined' ? out : DOMPurify.sanitize(out);
  }

  let shown = $derived.by(() => {
    const rs = data?.releases ?? [];
    const q = query.trim().toLowerCase();
    if (!q) return rs;
    return rs.filter((r) => r.version.includes(q) || r.body.toLowerCase().includes(q));
  });

  /** Open by default: unreleased notes and the running version. */
  function expanded(version: string, i: number): boolean {
    return !!query.trim() || i === 0 || version === data?.current;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && open) open = false;
  }
</script>

<svelte:window onkeydown={onKey} />

{#if open}
  <div class="scrim" transition:fade={{ duration: dur }} onclick={() => (open = false)} role="presentation"></div>
  <div class="wrap">
    <div class="modal" transition:scale={{ duration: dur, start: 0.97, opacity: 0 }} role="dialog" aria-modal="true" aria-label="What's new in wipe">
      <header class="head">
        <div class="title">
          <h3>What's new</h3>
          {#if data}<span class="sub">you're on v{data.current} · <code>wipe changelog</code></span>{/if}
        </div>
        <input class="find" type="search" placeholder="Filter…" bind:value={query} aria-label="Filter the changelog" />
        <button class="close" aria-label="Close" onclick={() => (open = false)}><X size={18} /></button>
      </header>

      <div class="body wp-scroll">
        {#if error}
          <p class="empty">Couldn't load the changelog: {error}</p>
        {:else if !data}
          <p class="empty">Loading…</p>
        {:else if !shown.length}
          <p class="empty">Nothing matches "{query}".</p>
        {:else}
          {#each shown as r, i (r.version)}
            <details class="rel" open={expanded(r.version, i)}>
              <summary>
                <span class="ver">{r.version}</span>
                {#if r.version === data.current}<span class="tag cur">this version</span>{/if}
                {#if !r.date}<span class="tag">unreleased</span>{/if}
                <span class="date">{r.date ?? ''}</span>
              </summary>
              <!-- eslint-disable-next-line svelte/no-at-html-tags -->
              <div class="md">{@html html(r.body)}</div>
            </details>
          {/each}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    z-index: 90;
  }
  .wrap {
    position: fixed;
    inset: 0;
    z-index: 91;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 6vh 16px;
    pointer-events: none;
  }
  .modal {
    pointer-events: auto;
    width: min(680px, 100%);
    max-height: 88vh;
    display: flex;
    flex-direction: column;
    background: var(--wp-card);
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-lg);
    box-shadow: var(--wp-shadow-lift);
    overflow: hidden;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 14px 16px 14px 20px;
    border-bottom: 1px solid var(--wp-border);
  }
  .title {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .title h3 {
    font-family: var(--wp-font-display);
    font-size: 16px;
    font-weight: 600;
  }
  .sub {
    font-size: 12px;
    color: var(--wp-text-subtle);
  }
  .sub code {
    font-family: var(--wp-font-mono);
    font-size: 11px;
  }
  .find {
    width: 150px;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--wp-r-sm);
    border: 1px solid var(--wp-border);
    background: var(--wp-surface);
    color: var(--wp-text);
    font-size: 12.5px;
  }
  .close {
    display: inline-flex;
    padding: 6px;
    border: none;
    background: none;
    color: var(--wp-text-muted);
    cursor: pointer;
    border-radius: var(--wp-r-sm);
  }
  .close:hover {
    background: var(--wp-elevated);
    color: var(--wp-text);
  }
  .body {
    overflow-y: auto;
    padding: 6px 20px 20px;
  }
  .empty {
    padding: 24px 0;
    color: var(--wp-text-muted);
    font-size: 13px;
  }
  .rel {
    border-bottom: 1px solid var(--wp-border);
  }
  .rel:last-child {
    border-bottom: none;
  }
  summary {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 12px 0;
    cursor: pointer;
    list-style: none;
  }
  summary::-webkit-details-marker {
    display: none;
  }
  summary::before {
    content: '▸';
    color: var(--wp-text-subtle);
    font-size: 11px;
    transition: transform var(--wp-fast) var(--wp-ease);
  }
  details[open] > summary::before {
    transform: rotate(90deg);
  }
  .ver {
    font-family: var(--wp-font-mono);
    font-weight: 600;
    font-size: 14px;
  }
  .tag {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: var(--wp-r-pill);
    border: 1px solid var(--wp-border);
    color: var(--wp-text-muted);
  }
  .tag.cur {
    border-color: color-mix(in srgb, var(--wp-accent) 50%, var(--wp-border));
    color: var(--wp-accent);
  }
  .date {
    margin-left: auto;
    font-size: 12px;
    color: var(--wp-text-subtle);
  }
  .md {
    padding: 0 0 14px 18px;
    font-size: 13.5px;
    line-height: 1.55;
    color: var(--wp-text);
    word-break: break-word;
  }
  .md :global(h3) {
    font-size: 13px;
    font-weight: 600;
    margin: 14px 0 6px;
    color: var(--wp-text);
  }
  .md :global(ul) {
    padding-left: 18px;
    margin: 6px 0;
  }
  .md :global(li) {
    margin: 4px 0;
  }
  .md :global(p) {
    margin: 6px 0;
  }
  .md :global(code) {
    font-family: var(--wp-font-mono);
    font-size: 0.88em;
    background: var(--wp-surface);
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-sm);
    padding: 0 4px;
  }
  .md :global(pre) {
    background: var(--wp-surface);
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-sm);
    padding: 8px 10px;
    overflow-x: auto;
  }
  .md :global(pre code) {
    border: none;
    padding: 0;
  }
  .md :global(a) {
    color: var(--wp-accent);
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  @media (max-width: 520px) {
    .find {
      width: 100px;
    }
  }
</style>

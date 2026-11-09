<script lang="ts">
  // The one hover card and click handler for every `@` reference chip on the
  // page (`.wref`, see lib/refs.ts): delegated listeners, a single floating
  // card, nothing per chip - so long threads full of references stay cheap.
  import { onMount } from 'svelte';
  import { scale } from 'svelte/transition';
  import { page } from '$app/state';
  import { CornerDownRight, MessageSquare, CheckSquare } from 'lucide-svelte';
  import Avatar from './Avatar.svelte';
  import { board, identities, postIndex, definitions } from '$lib/stores/board';
  import { follow, parseRoute } from '$lib/nav';
  import { renderInline, type RefKind } from '$lib/refs';
  import { splitReplyTags, oneLine } from '$lib/forum';
  import { formatDate, labelColorFor } from '$lib/utils';

  type Shown = { kind: RefKind; id: string; x: number; y: number; above: boolean };
  let shown = $state<Shown | null>(null);
  let showTimer: ReturnType<typeof setTimeout> | null = null;
  let hideTimer: ReturnType<typeof setTimeout> | null = null;
  let anchor: Element | null = null;

  const W = 320;

  function place(el: Element): Shown {
    const r = el.getBoundingClientRect();
    const kind = (el.getAttribute('data-kind') ?? 'ticket') as RefKind;
    const id = el.getAttribute('data-ref') ?? '';
    const x = Math.max(8, Math.min(window.innerWidth - W - 8, r.left + r.width / 2 - W / 2));
    // Below the chip unless that runs off the screen.
    const above = r.bottom + 220 > window.innerHeight && r.top > 240;
    return { kind, id, x, y: above ? window.innerHeight - r.top + 6 : r.bottom + 6, above };
  }

  function clearTimers() {
    if (showTimer) clearTimeout(showTimer);
    if (hideTimer) clearTimeout(hideTimer);
    showTimer = hideTimer = null;
  }

  function onOver(e: MouseEvent) {
    const el = (e.target as Element | null)?.closest?.('.wref[data-ref]');
    if (!el) return;
    if (el === anchor) {
      if (hideTimer) clearTimeout(hideTimer);
      hideTimer = null;
      return;
    }
    clearTimers();
    anchor = el;
    // Already showing one: move to the next chip at once; else a short intent delay.
    if (shown) shown = place(el);
    else showTimer = setTimeout(() => anchor === el && (shown = place(el)), 260);
  }

  function onOut(e: MouseEvent) {
    if (!anchor) return;
    const to = e.relatedTarget as Node | null;
    if (to && anchor.contains(to)) return;
    if (showTimer) clearTimeout(showTimer);
    showTimer = null;
    hideTimer = setTimeout(() => {
      shown = null;
      anchor = null;
    }, 90);
  }

  function onClick(e: MouseEvent) {
    const el = (e.target as Element | null)?.closest?.('.wref[data-ref]');
    if (!el) return;
    // Modified clicks keep the browser's own behaviour (open in a new tab).
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    clearTimers();
    shown = null;
    anchor = null;
    const kind = el.getAttribute('data-kind') as RefKind;
    const id = el.getAttribute('data-ref') ?? '';
    void follow(kind, id, parseRoute(page.url));
  }

  onMount(() => {
    document.addEventListener('mouseover', onOver);
    document.addEventListener('mouseout', onOut);
    // Capture: a chip inside a clickable card/button acts as the link, not the card.
    document.addEventListener('click', onClick, true);
    const hide = () => {
      clearTimers();
      shown = null;
      anchor = null;
    };
    window.addEventListener('scroll', hide, true);
    window.addEventListener('blur', hide);
    return () => {
      document.removeEventListener('mouseover', onOver);
      document.removeEventListener('mouseout', onOut);
      document.removeEventListener('click', onClick, true);
      window.removeEventListener('scroll', hide, true);
      window.removeEventListener('blur', hide);
    };
  });

  let ticketInfo = $derived.by(() => {
    if (shown?.kind !== 'ticket' || !$board) return null;
    for (const l of $board.lists) {
      const t = l.tickets.find((x) => x.id === shown!.id);
      if (t) return { t, list: l.name ?? l.list };
    }
    return null;
  });
  let post = $derived(shown?.kind === 'post' ? $postIndex.get(shown.id) : undefined);
  let person = $derived(
    shown?.kind === 'identity'
      ? $identities.find((i) => i.id.toLowerCase() === shown!.id.toLowerCase())
      : undefined
  );
  function identityFor(a: string) {
    const key = (a.match(/<([^>]+)>/)?.[1] ?? a).toLowerCase();
    return $identities.find((i) => i.id.toLowerCase() === key);
  }
  function nameFor(a: string) {
    return identityFor(a)?.display_name ?? a.replace(/\s*<[^>]+>$/, '');
  }
</script>

{#if shown}
  <div
    class="rp"
    class:above={shown.above}
    style="left:{shown.x}px; {shown.above ? 'bottom' : 'top'}:{shown.y}px; width:{W}px"
    transition:scale={{ start: 0.96, duration: 120, opacity: 0 }}
    role="tooltip"
  >
    {#if shown.kind === 'ticket'}
      {#if ticketInfo}
        {@const t = ticketInfo.t}
        <div class="row head">
          <span class="rid">{t.id}</span>
          <span class="tag">{ticketInfo.list}</span>
          {#if t.priority}<span class="meta">{t.priority}</span>{/if}
        </div>
        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
        <div class="title">{@html renderInline(t.title)}</div>
        {#if t.labels.length}
          <div class="row chips">
            {#each t.labels.slice(0, 4) as l (l)}
              <span class="chip" style="--c:{labelColorFor(l, $definitions.labels)}">{l}</span>
            {/each}
          </div>
        {/if}
        {#if t.body}<div class="snip">{oneLine(t.body, 180)}</div>{/if}
        <div class="row foot">
          {#each t.assignees.slice(0, 4) as a (a)}
            <Avatar id={a} identity={identityFor(a)} size={18} />
          {/each}
          {#if t.checklist.length}
            <span class="meta"
              ><CheckSquare size={11} />{t.checklist.filter((c) => c.done).length}/{t.checklist
                .length}</span
            >
          {/if}
          {#if t.comments.length}
            <span class="meta"><MessageSquare size={11} />{t.comments.length}</span>
          {/if}
          <span class="meta right">updated {formatDate(t.updated)}</span>
        </div>
      {:else}
        <div class="missing">{shown.id} isn't on this board.</div>
      {/if}
    {:else if shown.kind === 'post'}
      {#if post}
        <div class="row head">
          <span class="rid post">{post.id}</span>
          <Avatar id={identityFor(post.author)?.id ?? post.author} identity={identityFor(post.author)} size={18} />
          <span class="who">{nameFor(post.author)}</span>
          <span class="meta right">{formatDate(post.created)}</span>
        </div>
        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
        <div class="title">{@html renderInline(post.summary)}</div>
        {#if splitReplyTags(post.body).rest}
          <div class="snip">{oneLine(splitReplyTags(post.body).rest, 200)}</div>
        {/if}
        <div class="row foot">
          {#if post.depth > 0}
            <span class="meta"><CornerDownRight size={11} />in {post.thread_id} · {oneLine(post.thread_title, 40)}</span>
          {:else}
            <span class="meta">opens a thread</span>
          {/if}
          {#if post.replies}<span class="meta right"><MessageSquare size={11} />{post.replies}</span>{/if}
        </div>
      {:else}
        <div class="missing">{shown.id} isn't in this forum.</div>
      {/if}
    {:else}
      <div class="row head">
        <Avatar id={person?.id ?? shown.id} identity={person} size={26} />
        <div class="col">
          <span class="who big">{person?.display_name ?? shown.id}</span>
          <span class="meta">
            {#if !person}not on this board{:else if person.kind === 'agent'}agent{#if person.owner}
                · {nameFor(person.owner)}'s{/if}{:else}person · {person.id}{/if}
          </span>
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .rp {
    position: fixed;
    z-index: 300;
    pointer-events: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-radius: var(--wp-r-md);
    border: 1px solid var(--wp-border-strong);
    background: color-mix(in srgb, var(--wp-elevated) 94%, transparent);
    backdrop-filter: blur(10px);
    box-shadow: var(--wp-shadow-lift);
    transform-origin: top center;
    font-size: 12.5px;
    color: var(--wp-text);
  }
  .rp.above {
    transform-origin: bottom center;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .col {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .rid {
    font-family: var(--wp-font-mono);
    font-size: 11px;
    color: var(--wp-accent);
  }
  .rid.post {
    color: var(--wp-focus);
  }
  .tag {
    font-family: var(--wp-font-display);
    font-size: 9.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 1px 7px;
    border-radius: var(--wp-r-pill);
    background: color-mix(in srgb, var(--wp-accent) 14%, transparent);
    color: var(--wp-accent);
  }
  .title {
    font-family: var(--wp-font-display);
    font-size: 14px;
    font-weight: 600;
    line-height: 1.3;
  }
  .snip {
    color: var(--wp-text-muted);
    line-height: 1.45;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .chips {
    flex-wrap: wrap;
  }
  .chip {
    font-size: 10px;
    padding: 0 7px;
    height: 17px;
    display: inline-flex;
    align-items: center;
    border-radius: var(--wp-r-pill);
    color: color-mix(in srgb, var(--c) 82%, var(--wp-text));
    background: color-mix(in srgb, var(--c) 15%, transparent);
  }
  .foot {
    margin-top: 2px;
  }
  .meta {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 11px;
    color: var(--wp-text-subtle);
  }
  .right {
    margin-left: auto;
  }
  .who {
    font-weight: 600;
    font-size: 12px;
  }
  .who.big {
    font-size: 13.5px;
  }
  .missing {
    color: var(--wp-text-muted);
  }
</style>

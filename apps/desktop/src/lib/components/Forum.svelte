<script lang="ts">
  // The forum. Every post is its own entity (`F-01A`) headed by a one-sentence
  // summary; a post replies to the posts its text opens with tags for. The
  // sidebar lists the posts that open a thread; the thread view nests each
  // reply under the post it answers. What is open is the URL (`/F-01A`), so
  // reference jumps, back and forward all land on the right post.
  import { onMount, tick } from 'svelte';
  import { get } from 'svelte/store';
  import { MessageSquarePlus, Search, X, SlidersHorizontal, MessageSquare, Hash } from 'lucide-svelte';
  import ForumPost from './ForumPost.svelte';
  import PostComposer from './PostComposer.svelte';
  import Avatar from './Avatar.svelte';
  import Markdown from './Markdown.svelte';
  import WatchToggle from './WatchToggle.svelte';
  import { api } from '$lib/api';
  import {
    currentProject,
    identities,
    definitions,
    forumPosts,
    forumFormat,
    postIndex,
    loadForum,
    me
  } from '$lib/stores/board';
  import { go, openPost } from '$lib/nav';
  import { threadsOf, childrenOf, splitReplyTags } from '$lib/forum';
  import { labelColor } from '$lib/utils';
  import type { ForumPost as Post, ForumThreadSummary } from '$lib/types';

  let { focus = null }: { focus?: string | null } = $props();

  let error = $state<string | null>(null);

  // --- sidebar width (resizable + remembered) ------------------------------
  const SIDEBAR_KEY = 'wipe.forum.sidebarWidth';
  let sidebarWidth = $state(300);
  onMount(() => {
    try {
      const v = parseInt(localStorage.getItem(SIDEBAR_KEY) ?? '', 10);
      if (Number.isFinite(v)) sidebarWidth = Math.max(240, Math.min(600, v));
    } catch {
      /* storage unavailable */
    }
    void loadForum();
  });
  function startResize(e: PointerEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startW = sidebarWidth;
    const onMove = (ev: PointerEvent) => {
      sidebarWidth = Math.max(240, Math.min(600, startW + (ev.clientX - startX)));
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      try {
        localStorage.setItem(SIDEBAR_KEY, String(Math.round(sidebarWidth)));
      } catch {
        /* ignore */
      }
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }

  // --- what is open ------------------------------------------------------------
  let threads = $derived(threadsOf($forumPosts));
  let focusPost = $derived(focus ? $postIndex.get(focus) : undefined);
  let selectedId = $derived(focusPost?.thread_id ?? null);
  let root = $derived(selectedId ? $postIndex.get(selectedId) : undefined);
  let threadPosts = $derived(selectedId ? $forumPosts.filter((p) => p.thread_id === selectedId) : []);
  let kids = $derived(childrenOf(threadPosts));
  let openSummary = $derived(threads.find((t) => t.id === selectedId));

  // Land on the post in focus: scroll it into view and highlight it.
  let landed: string | null = null;
  $effect(() => {
    const id = focusPost?.id;
    if (!id || id === landed) return;
    landed = id;
    void tick().then(() => {
      const row = document.querySelector<HTMLElement>(`[data-post-id="${CSS.escape(id)}"]`);
      const target = row?.querySelector<HTMLElement>('.bubble') ?? row;
      if (!row || !target) return;
      if (id === selectedId) feedEl?.scrollTo({ top: 0, behavior: 'smooth' });
      else row.scrollIntoView({ behavior: 'smooth', block: 'center' });
      target.classList.remove('wp-flash');
      void target.offsetWidth;
      target.classList.add('wp-flash');
      setTimeout(() => target.classList.remove('wp-flash'), 1700);
    });
  });

  // --- composers -------------------------------------------------------------
  let composingThread = $state(false);
  let ntSummary = $state('');
  let ntBody = $state('');
  let ntLabels = $state<string[]>([]);
  // The reply box: its body opens with the tags of the posts it answers - the
  // thread's opening post by default, the post whose Reply was clicked after.
  let rSummary = $state('');
  let rBody = $state('');
  let rBodyEl = $state<HTMLTextAreaElement | null>(null);
  let feedEl = $state<HTMLDivElement | null>(null);
  let composedFor: string | null = null;
  $effect(() => {
    if (!selectedId || selectedId === composedFor) return;
    composedFor = selectedId;
    rSummary = '';
    rBody = `@${selectedId} `;
  });
  let replyTargets = $derived(splitReplyTags(rBody).tags.filter((t) => /^F-/.test(t)));

  function proj() {
    return get(currentProject) ?? undefined;
  }

  async function createThread(p: { summary: string; body: string; labels: string[]; attachments: import('$lib/types').Attachment[] }) {
    const post = await api.forumCreate(
      { summary: p.summary, body: p.body, labels: p.labels, attachments: p.attachments },
      proj()
    );
    composingThread = false;
    await loadForum();
    await openPost(post.id);
  }

  async function sendReply(p: { summary: string; body: string; attachments: import('$lib/types').Attachment[] }) {
    const post = await api.forumCreate(
      { summary: p.summary, body: p.body, attachments: p.attachments },
      proj()
    );
    await loadForum();
    // The box stays aimed at the thread.
    rBody = selectedId ? `@${selectedId} ` : '';
    await go({ view: 'forum', post: post.id }, true);
  }

  /** Aim the reply box at `post`: its tag replaces the ones the body opens with. */
  async function startReply(post: Post) {
    const { rest } = splitReplyTags(rBody);
    rBody = `@${post.id} ${rest}`;
    await tick();
    rBodyEl?.focus();
    rBodyEl?.setSelectionRange(rBody.length, rBody.length);
    rBodyEl?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }

  async function saveEdit(post: Post, summary: string, body: string) {
    try {
      await api.forumEdit(post.id, { summary, body }, proj());
      await loadForum();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      throw e;
    }
  }

  async function del(post: Post) {
    try {
      await api.forumDelete(post.id, proj());
      await loadForum();
      if (post.id === selectedId) await go({ view: 'forum' });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  let copied = $state<string | null>(null);
  async function copyLink(post: Post) {
    const url = `${location.origin}/${post.id}`;
    try {
      await navigator.clipboard.writeText(url);
      copied = post.id;
      setTimeout(() => (copied = null), 1400);
    } catch {
      error = `Link: ${url}`;
    }
  }

  // --- search + filters ------------------------------------------------------
  let query = $state('');
  let filtersOpen = $state(false);
  let fIdentities = $state<string[]>([]); // identity ids that must have engaged
  let fOpOnly = $state(false); // ...only as the original poster
  let fLabels = $state<string[]>([]);
  let activeFilters = $derived(fIdentities.length + fLabels.length + (fOpOnly ? 1 : 0));

  // Threads with a post matching every search word (summary, body, author, id).
  let searchThreadIds = $derived.by(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return null;
    const hit = new Set<string>();
    for (const p of $forumPosts) {
      const hay = `${p.id} ${p.legacy_id ?? ''} ${p.summary} ${p.body} ${p.author}`.toLowerCase();
      if (words.every((w) => hay.includes(w))) hit.add(p.thread_id);
    }
    return hit;
  });

  function identityFor(id: string) {
    const key = (id.match(/<([^>]+)>/)?.[1] ?? id).toLowerCase();
    return $identities.find((i) => i.id.toLowerCase() === key);
  }
  function displayName(id: string) {
    return identityFor(id)?.display_name ?? id.replace(/\s*<[^>]+>$/, '');
  }
  /** Compact "last activity" label: 5s → "now", then m / h / d, then a date. */
  function relativeTime(iso?: string): string {
    if (!iso) return '';
    const then = new Date(iso).getTime();
    if (Number.isNaN(then)) return '';
    const s = Math.max(0, (Date.now() - then) / 1000);
    if (s < 45) return 'now';
    if (s < 3600) return `${Math.round(s / 60)}m`;
    if (s < 86400) return `${Math.round(s / 3600)}h`;
    if (s < 7 * 86400) return `${Math.round(s / 86400)}d`;
    return new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  }
  function labelColorFor(name: string) {
    const def = $definitions.labels.find((l) => l.name === name);
    return labelColor(name, def?.color);
  }
  /** Whether an author string (`Name <email>` or a bare id) refers to identity `id`. */
  function sameIdentity(author: string, id: string): boolean {
    const key = (author.match(/<([^>]+)>/)?.[1] ?? author).toLowerCase();
    return key === id.toLowerCase();
  }
  function toggleFIdentity(id: string) {
    fIdentities = fIdentities.includes(id) ? fIdentities.filter((x) => x !== id) : [...fIdentities, id];
  }
  function toggleFLabel(name: string) {
    fLabels = fLabels.includes(name) ? fLabels.filter((x) => x !== name) : [...fLabels, name];
  }
  function clearFilters() {
    fIdentities = [];
    fLabels = [];
    fOpOnly = false;
  }

  let filtered = $derived(
    threads.filter((t) => {
      if (searchThreadIds && !searchThreadIds.has(t.id)) return false;
      if (fLabels.length && !fLabels.some((l) => t.labels?.includes(l))) return false;
      if (fIdentities.length) {
        if (fOpOnly) {
          if (!fIdentities.some((id) => sameIdentity(t.author, id))) return false;
        } else {
          const parts = t.participants ?? [t.author, t.last_author];
          if (!fIdentities.some((id) => parts.some((p) => sameIdentity(p, id)))) return false;
        }
      }
      return true;
    })
  );

  /** Time-bucket a thread by how long ago it was last active. */
  function bucketOf(iso?: string): string {
    if (!iso) return 'Earlier';
    const then = new Date(iso).getTime();
    if (Number.isNaN(then)) return 'Earlier';
    const days = (Date.now() - then) / 86400000;
    if (days < 1) return 'Today';
    if (days < 7) return 'This week';
    if (days < 30) return 'This month';
    return new Date(iso).toLocaleDateString(undefined, { month: 'long', year: 'numeric' });
  }

  type Row = { kind: 'divider'; label: string } | { kind: 'thread'; t: ForumThreadSummary };
  let sidebarItems = $derived.by<Row[]>(() => {
    const out: Row[] = [];
    let last: string | null = null;
    for (const t of filtered) {
      const b = bucketOf(t.updated ?? t.created);
      if (b !== last) {
        out.push({ kind: 'divider', label: b });
        last = b;
      }
      out.push({ kind: 'thread', t });
    }
    return out;
  });

  // A project switch closes the composer and the filters.
  $effect(() => {
    void $currentProject;
    composingThread = false;
    query = '';
    clearFilters();
  });
</script>

<div class="forum" class:reading={!!selectedId} style="--sw:{sidebarWidth}px">
  <aside class="side wp-scroll">
    <div class="side-head">
      <span class="h">Forum</span>
      <button class="new" onclick={() => (composingThread = !composingThread)}>
        <MessageSquarePlus size={15} /> New
      </button>
    </div>

    {#if composingThread}
      <PostComposer
        bind:summary={ntSummary}
        bind:body={ntBody}
        bind:labels={ntLabels}
        withLabels
        compact
        submitLabel="Post thread"
        summaryPlaceholder="Thread summary - the point, in one sentence"
        onsubmit={createThread}
        oncancel={() => (composingThread = false)}
      />
    {/if}

    <div class="searchrow">
      <div class="searchbar">
        <Search size={14} />
        <input class="sin" placeholder="Search posts…" bind:value={query} />
        {#if query}
          <button class="sclear" aria-label="Clear search" onclick={() => (query = '')}><X size={12} /></button>
        {/if}
      </div>
      <button
        class="filterbtn"
        class:on={filtersOpen || activeFilters > 0}
        title="Filters"
        onclick={() => (filtersOpen = !filtersOpen)}
      >
        <SlidersHorizontal size={14} />
        {#if activeFilters > 0}<span class="fcount">{activeFilters}</span>{/if}
      </button>
    </div>

    {#if filtersOpen}
      <div class="filters">
        <div class="frow">
          <span class="flabel">Engaged</span>
          <label class="oponly"><input type="checkbox" bind:checked={fOpOnly} /> only as OP</label>
        </div>
        <div class="fpool">
          {#each $identities as id (id.id)}
            <button class="ichip" class:on={fIdentities.includes(id.id)} onclick={() => toggleFIdentity(id.id)}>
              <Avatar id={id.id} identity={id} size={15} />
              <span class="iname">{id.display_name || id.id}</span>
            </button>
          {/each}
        </div>
        {#if $definitions.labels.length}
          <span class="flabel">Labels</span>
          <div class="fpool">
            {#each $definitions.labels as label (label.name)}
              <button
                class="pchip"
                class:on={fLabels.includes(label.name)}
                style="--c:{labelColor(label.name, label.color)}"
                onclick={() => toggleFLabel(label.name)}>{label.name}</button
              >
            {/each}
          </div>
        {/if}
        {#if activeFilters > 0}
          <button class="clearf" onclick={clearFilters}>Clear filters</button>
        {/if}
      </div>
    {/if}

    {#if sidebarItems.length === 0}
      <div class="empty">
        {threads.length === 0 ? 'No threads yet. Start one with New.' : 'No threads match.'}
      </div>
    {:else}
      {#each sidebarItems as item (item.kind === 'divider' ? 'd:' + item.label : item.t.id)}
        {#if item.kind === 'divider'}
          <div class="seg">{item.label}</div>
        {:else}
          {@const t = item.t}
          <a
            class="threadrow"
            class:active={t.id === selectedId}
            href="/{t.id}"
            onclick={(e) => {
              if (e.metaKey || e.ctrlKey || e.shiftKey) return;
              e.preventDefault();
              void openPost(t.id);
            }}
          >
            <div class="tr-top">
              <span class="tr-title"><Markdown source={t.title} inline /></span>
              <span class="tr-count" title="{t.posts} post{t.posts === 1 ? '' : 's'}">
                <MessageSquare size={11} />{t.posts}
              </span>
            </div>
            {#if t.snippet}<div class="tr-snip">{t.snippet}</div>{/if}
            <div class="tr-sub">
              <span class="tr-id">{t.id}</span>
              {#if t.labels?.length}
                <span class="tr-labels">
                  {#each t.labels.slice(0, 2) as l (l)}
                    <span class="tr-chip" style="--c:{labelColorFor(l)}">{l}</span>
                  {/each}
                  {#if t.labels.length > 2}<span class="tr-more">+{t.labels.length - 2}</span>{/if}
                </span>
              {/if}
              <span class="tr-right">
                <span class="tr-meta">{displayName(t.last_author ?? t.author)}</span>
                <span class="tr-time" title="last activity">{relativeTime(t.updated ?? t.created)}</span>
              </span>
            </div>
          </a>
        {/if}
      {/each}
    {/if}
    {#if $forumFormat === 'threads'}
      <p class="legacy">
        <Hash size={11} /> This board still has the old thread forum - <b>New ids</b> (top right) gives every
        post its own id.
      </p>
    {/if}
  </aside>

  <div class="resizer" role="separator" aria-orientation="vertical" tabindex="-1" onpointerdown={startResize}></div>

  <section class="thread">
    {#if error}<div class="err">{error} <button class="rx" onclick={() => (error = null)}><X size={11} /></button></div>{/if}
    {#if root}
      <button class="back" onclick={() => go({ view: 'forum' })}>&larr; All threads</button>
      <header class="th-head">
        <h2><Markdown source={root.summary} inline /></h2>
        <div class="th-meta">
          <span class="thread-id">{root.id}</span>
          {#if root.legacy_id}<span class="thread-id" title="Its id before the forum moved to one file per post">(was {root.legacy_id})</span>{/if}
          <WatchToggle kind="threads" id={root.id} label={false} />
          <span class="dot">·</span>
          <span>started by {displayName(root.author)}</span>
          {#if openSummary}
            <span class="dot">·</span>
            <span>{openSummary.posts} post{openSummary.posts === 1 ? '' : 's'}</span>
            <span class="dot">·</span>
            <span>active {relativeTime(openSummary.updated)}</span>
          {/if}
          {#if copied}<span class="copied">link copied</span>{/if}
        </div>
      </header>
      <div class="feed wp-scroll" bind:this={feedEl}>
        <ForumPost
          post={root}
          {kids}
          meId={$me?.id ?? null}
          onreply={startReply}
          onsave={saveEdit}
          ondelete={del}
          oncopy={copyLink}
        />
      </div>
      <div class="replybar">
        <div class="replying">
          {#if replyTargets.length}
            replying to
            {#each replyTargets as id (id)}<span class="rt">{id}</span>{/each}
            <span class="rhint">- the tags the text opens with decide what it answers</span>
          {:else}
            <span class="rhint">no post tag at the start: this becomes a new thread</span>
          {/if}
        </div>
        <PostComposer
          bind:summary={rSummary}
          bind:body={rBody}
          bind:bodyEl={rBodyEl}
          compact
          submitLabel="Reply"
          summaryPlaceholder="Summary of your reply - one sentence"
          onsubmit={sendReply}
        />
      </div>
    {:else if focus && $forumPosts.length}
      <div class="pick">{focus} isn't in this forum.</div>
    {:else}
      <div class="pick">Select a thread, or start a new one.</div>
    {/if}
  </section>
</div>

<style>
  .forum {
    position: relative;
    display: grid;
    grid-template-columns: var(--sw, 300px) minmax(0, 1fr);
    gap: 14px;
    height: 100%;
    min-height: 0;
  }
  .back {
    display: none;
  }
  /* Phones: one pane at a time - the thread list, or the open thread with a
     way back to the list. */
  @media (max-width: 700px) {
    .forum {
      grid-template-columns: minmax(0, 1fr);
      padding: 0 12px;
    }
    .resizer {
      display: none;
    }
    .side {
      border-right: none;
      padding-right: 0;
    }
    .forum.reading .side,
    .forum:not(.reading) .thread {
      display: none;
    }
    .back {
      display: inline-flex;
      align-self: flex-start;
      margin-bottom: 8px;
      padding: 6px 10px;
      border-radius: var(--wp-r-sm);
      border: 1px solid var(--wp-border);
      background: var(--wp-card);
      color: var(--wp-text-muted);
      font-size: 13px;
      cursor: pointer;
    }
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow-y: auto;
    padding-right: 6px;
    border-right: 1px solid var(--wp-border);
  }
  /* Drag handle sitting on the sidebar's right border to resize it. */
  .resizer {
    position: absolute;
    top: 0;
    bottom: 0;
    left: var(--sw, 300px);
    width: 10px;
    transform: translateX(-5px);
    cursor: col-resize;
    z-index: 6;
    background: none;
  }
  .resizer::after {
    content: '';
    position: absolute;
    inset: 0 4px;
    border-radius: var(--wp-r-pill);
    background: transparent;
    transition: background var(--wp-fast) var(--wp-ease);
  }
  .resizer:hover::after {
    background: color-mix(in srgb, var(--wp-accent) 45%, transparent);
  }
  .side-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .side-head .h {
    font-family: var(--wp-font-display);
    font-size: 13px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--wp-text-muted);
  }
  .new {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 10px;
    border-radius: var(--wp-r-sm);
    font-size: 12px;
    cursor: pointer;
    border: 1px solid var(--wp-border);
    background: var(--wp-card);
    color: var(--wp-text-muted);
  }
  .new:hover {
    color: var(--wp-text);
    background: var(--wp-elevated);
  }
  .pchip {
    display: inline-flex;
    align-items: center;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--wp-r-pill);
    border: 1px solid color-mix(in srgb, var(--c) 55%, transparent);
    background: none;
    color: var(--wp-text-muted);
    font-size: 11px;
    cursor: pointer;
    transition: all var(--wp-fast) var(--wp-ease);
  }
  .pchip::before {
    content: '';
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--c);
    margin-right: 6px;
  }
  .pchip.on {
    background: color-mix(in srgb, var(--c) 16%, transparent);
    border-color: var(--c);
    color: var(--wp-text);
  }
  .searchrow {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .searchbar {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    height: 32px;
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-pill);
    background: var(--wp-surface);
    color: var(--wp-text-subtle);
  }
  .sin {
    border: none;
    background: none;
    color: var(--wp-text);
    font-size: 13px;
    width: 100%;
    min-width: 0;
    outline: none;
  }
  .sclear {
    display: inline-flex;
    border: none;
    background: none;
    color: var(--wp-text-subtle);
    cursor: pointer;
    padding: 2px;
  }
  .sclear:hover {
    color: var(--wp-text);
  }
  .filterbtn {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 32px;
    width: 34px;
    flex: none;
    border-radius: var(--wp-r-sm);
    border: 1px solid var(--wp-border);
    background: var(--wp-surface);
    color: var(--wp-text-subtle);
    cursor: pointer;
    transition: all var(--wp-fast) var(--wp-ease);
  }
  .filterbtn:hover,
  .filterbtn.on {
    color: var(--wp-text);
    border-color: var(--wp-border-strong);
    background: var(--wp-elevated);
  }
  .fcount {
    position: absolute;
    top: -5px;
    right: -5px;
    min-width: 15px;
    height: 15px;
    padding: 0 4px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--wp-r-pill);
    background: var(--wp-accent);
    color: var(--wp-on-accent);
    font-size: 9px;
    font-weight: 700;
  }
  .filters {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-md);
    background: var(--wp-surface);
  }
  .frow {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .flabel {
    font-family: var(--wp-font-display);
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--wp-text-muted);
  }
  .oponly {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--wp-text-subtle);
    cursor: pointer;
  }
  .fpool {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .ichip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 100%;
    height: 24px;
    padding: 0 9px 0 4px;
    border-radius: var(--wp-r-pill);
    border: 1px solid var(--wp-border);
    background: var(--wp-card);
    color: var(--wp-text-muted);
    font-size: 11px;
    cursor: pointer;
    transition: all var(--wp-fast) var(--wp-ease);
  }
  .ichip .iname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ichip.on {
    border-color: var(--wp-accent);
    background: color-mix(in srgb, var(--wp-accent) 14%, transparent);
    color: var(--wp-text);
  }
  .clearf {
    align-self: flex-start;
    border: none;
    background: none;
    color: var(--wp-text-subtle);
    font-size: 11px;
    cursor: pointer;
    padding: 2px 0;
  }
  .clearf:hover {
    color: var(--wp-accent);
  }
  /* Minimalistic activity-bucket divider. */
  .seg {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 8px 2px 2px;
    font-family: var(--wp-font-display);
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--wp-text-subtle);
  }
  .seg::after {
    content: '';
    flex: 1;
    height: 1px;
    background: var(--wp-border);
  }
  .seg:first-child {
    margin-top: 0;
  }
  .threadrow {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 4px;
    text-align: left;
    padding: 8px 10px 8px 12px;
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-md);
    background: var(--wp-card);
    cursor: pointer;
    color: var(--wp-text);
    transition:
      background var(--wp-fast) var(--wp-ease),
      border-color var(--wp-fast) var(--wp-ease);
  }
  .threadrow:hover {
    background: var(--wp-elevated);
    border-color: var(--wp-border-strong);
  }
  /* Active thread: an accent spine on the left + tinted surface. */
  .threadrow.active {
    border-color: color-mix(in srgb, var(--wp-accent) 55%, transparent);
    background: color-mix(in srgb, var(--wp-accent) 9%, var(--wp-card));
  }
  .threadrow.active::before {
    content: '';
    position: absolute;
    left: 0;
    top: 8px;
    bottom: 8px;
    width: 3px;
    border-radius: 0 var(--wp-r-pill) var(--wp-r-pill) 0;
    background: var(--wp-accent);
  }
  .tr-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .tr-title {
    flex: 1;
    min-width: 0;
    font-family: var(--wp-font-display);
    font-size: 13.5px;
    font-weight: 600;
    line-height: 1.3;
    letter-spacing: -0.005em;
    /* single line - the sidebar stays dense so more threads fit vertically */
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tr-sub {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .tr-right {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
    min-width: 0;
  }
  .tr-meta {
    font-size: 11px;
    color: var(--wp-text-subtle);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tr-time {
    font-family: var(--wp-font-mono);
    font-size: 10.5px;
    color: var(--wp-text-subtle);
    flex: none;
  }
  /* Post counter: distinct hue + icon so it reads as a "conversation size" badge. */
  .tr-count {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-family: var(--wp-font-mono);
    font-size: 11px;
    height: 18px;
    padding: 0 7px;
    border-radius: var(--wp-r-pill);
    background: color-mix(in srgb, var(--wp-focus) 15%, transparent);
    color: var(--wp-focus);
    flex: none;
  }
  .tr-snip {
    font-size: 12px;
    line-height: 1.4;
    color: var(--wp-text-muted);
    /* exactly one line, truncated with an ellipsis sized to the sidebar width */
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tr-labels {
    display: flex;
    flex-wrap: nowrap;
    gap: 4px;
    min-width: 0;
    overflow: hidden;
  }
  .tr-more {
    font-size: 10px;
    color: var(--wp-text-subtle);
    align-self: center;
  }
  .tr-chip {
    display: inline-flex;
    align-items: center;
    height: 17px;
    padding: 0 7px 0 6px;
    border-radius: var(--wp-r-pill);
    font-size: 10px;
    font-weight: 500;
    color: color-mix(in srgb, var(--c) 82%, var(--wp-text));
    background: color-mix(in srgb, var(--c) 15%, transparent);
    border: 1px solid color-mix(in srgb, var(--c) 35%, transparent);
    white-space: nowrap;
  }
  .empty,
  .pick {
    color: var(--wp-text-subtle);
    font-size: 13px;
    padding: 12px 6px;
  }
  .pick {
    display: flex;
    height: 100%;
    align-items: center;
    justify-content: center;
  }
  .thread {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .th-head {
    padding-bottom: 12px;
    border-bottom: 1px solid var(--wp-border);
  }
  .th-head h2 {
    font-family: var(--wp-font-display);
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.01em;
    line-height: 1.25;
  }
  .th-meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
    font-size: 11.5px;
    color: var(--wp-text-subtle);
  }
  .th-meta .thread-id {
    font-family: var(--wp-font-mono);
    color: var(--wp-text-muted);
  }
  .th-meta .dot {
    opacity: 0.5;
  }
  .feed {
    flex: 1;
    overflow-y: auto;
    padding: 14px 2px;
  }
  .replybar {
    border-top: 1px solid var(--wp-border);
    padding-top: 10px;
  }
  .replying {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--wp-accent);
    margin-bottom: 6px;
  }
  .rx {
    display: inline-flex;
    border: none;
    background: none;
    color: var(--wp-text-subtle);
    cursor: pointer;
  }
  .err {
    font-size: 12px;
    color: var(--wp-error);
    padding: 4px 0;
  }
  a.threadrow {
    text-decoration: none;
  }
  .tr-id {
    font-family: var(--wp-font-mono);
    font-size: 10.5px;
    color: var(--wp-text-subtle);
    flex: none;
  }
  .tr-title :global(.wref) {
    font-size: 0.8em;
  }
  .legacy {
    display: flex;
    gap: 5px;
    align-items: baseline;
    margin: 8px 2px 4px;
    font-size: 11px;
    line-height: 1.45;
    color: var(--wp-text-subtle);
  }
  .copied {
    font-size: 11px;
    color: var(--wp-accent);
  }
  .rt {
    font-family: var(--wp-font-mono);
    font-size: 10.5px;
    padding: 0 5px;
    border-radius: 4px;
    color: var(--wp-focus);
    background: color-mix(in srgb, var(--wp-focus) 13%, transparent);
  }
  .rhint {
    color: var(--wp-text-subtle);
  }
  .replying {
    flex-wrap: wrap;
    color: var(--wp-text-muted);
  }
  .feed :global(.bubble.wp-flash) {
    animation: wp-flash 1.6s var(--wp-ease);
  }
</style>

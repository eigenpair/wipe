<script lang="ts">
  import { fly } from 'svelte/transition';
  import { X, RefreshCw, ArrowDown, ArrowUp, CircleDot } from 'lucide-svelte';
  import {
    syncStatus,
    syncLog,
    syncMode,
    historyCommit,
    loadLog,
    openHistory,
    openIncoming,
    openCommit,
    closeSync,
    refreshSync
  } from '$lib/stores/sync';
  import { formatDate } from '$lib/utils';
  import type { SyncLogEntry } from '$lib/types';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  let st = $derived($syncStatus);
  let where = $derived(st ? (st.remote ? `${st.remote}/${st.branch}` : st.branch) : 'wipe');
  let local = $derived(st?.mode === 'branch' ? st.local.length : 0);
  let incoming = $derived($syncLog.filter((c) => c.place === 'incoming'));
  let rest = $derived($syncLog.filter((c) => c.place !== 'incoming'));

  // Reload when opened, and when the branch or your base moves.
  let lastKey = '';
  $effect(() => {
    const key = `${open}|${st?.upstream}|${st?.base}|${st?.unpublished}`;
    if (open && key !== lastKey) void loadLog();
    lastKey = key;
  });

  function close() {
    open = false;
    if ($syncMode === 'history') void closeSync();
  }

  function pick(c: SyncLogEntry) {
    if ($historyCommit?.hash === c.hash) return;
    void openHistory(c);
  }

  function who(c: SyncLogEntry): string {
    return c.author_name;
  }
</script>

{#if open}
  <aside class="drawer" transition:fly={{ x: 24, duration: 160 }} aria-label="Board history">
    <header>
      <div class="title">
        <span>Board history</span>
        <span class="branch">{where}</span>
      </div>
      <button class="icon" title="Fetch and refresh" onclick={() => refreshSync({ fetch: true, force: true }).then(loadLog)}>
        <RefreshCw size={14} />
      </button>
      <button class="icon" title="Close" onclick={close}><X size={15} /></button>
    </header>

    <div class="body wp-scroll">
      {#if st?.mode === 'none'}
        <p class="empty">
          {#if st.vcs === 'plastic'}
            This board lives in a Unity Version Control (Plastic) workspace: its history is your
            Plastic changesets that include .wipe/. The wipe branch history needs git.
          {:else if st.vcs}
            This board's history is in your {st.vcs} commits that include .wipe/. The wipe branch
            history needs git.
          {:else}
            This board isn't under version control, so it has no history. In a git repo it would
            live on the wipe branch.
          {/if}
        </p>
      {:else if st?.mode !== 'branch'}
        <p class="empty">History follows the wipe branch - this board isn't on it yet.</p>
      {:else}
        <ol class="timeline">
          {#if local}
            <li class="node local" class:on={$syncMode === 'commit'}>
              <button onclick={() => openCommit()}>
                <span class="dot"><ArrowUp size={10} /></span>
                <span class="text">
                  <span class="subj">{local} uncommitted change{local === 1 ? '' : 's'}</span>
                  <span class="meta">yours, not on {where} yet - commit</span>
                </span>
              </button>
            </li>
          {/if}

          {#if incoming.length}
            <li class="node incoming-head" class:on={$syncMode === 'incoming'}>
              <button onclick={() => openIncoming()}>
                <span class="dot"><ArrowDown size={10} /></span>
                <span class="text">
                  <span class="subj">{incoming.length} incoming commit{incoming.length === 1 ? '' : 's'}</span>
                  <span class="meta">preview and apply - all, or one change at a time</span>
                </span>
              </button>
            </li>
            {#each incoming as c (c.hash)}
              <li class="node incoming sub">
                <button onclick={() => openIncoming()}>
                  <span class="dot"></span>
                  <span class="text">
                    <span class="subj">{c.subject}</span>
                    <span class="meta"><span class="hash">{c.short}</span> {who(c)} · {formatDate(c.date)}</span>
                  </span>
                </button>
              </li>
            {/each}
          {/if}

          {#each rest as c (c.hash)}
            {#if c.place === 'base'}
              <li class="here"><CircleDot size={12} /> your board is here</li>
            {/if}
            <li class="node {c.place}" class:on={$historyCommit?.hash === c.hash}>
              <button onclick={() => pick(c)} title="See the board right after this commit">
                <span class="dot"></span>
                <span class="text">
                  <span class="subj">{c.subject}</span>
                  <span class="meta">
                    <span class="hash">{c.short}</span>
                    {who(c)} · {formatDate(c.date)}
                    {#if c.place === 'unpublished'}<span class="tag">not published</span>{/if}
                  </span>
                </span>
              </button>
            </li>
          {/each}
        </ol>
        {#if $syncMode === 'history'}
          <p class="hint">
            The board shows how it was right after the selected commit, its changes highlighted. Hover a
            change to apply it to your board - it then shows as yours to commit.
          </p>
        {/if}
      {/if}
    </div>
  </aside>
{/if}

<style>
  .drawer {
    position: fixed;
    top: 53px;
    right: 12px;
    bottom: 12px;
    width: min(340px, calc(100vw - 24px));
    z-index: 35;
    display: flex;
    flex-direction: column;
    background: var(--wp-elevated);
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-lg);
    box-shadow: var(--wp-shadow-lift);
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px 10px 10px 14px;
    border-bottom: 1px solid var(--wp-border);
  }
  .title {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
    font-size: 13px;
    font-weight: 600;
  }
  .branch {
    font-family: var(--wp-font-mono);
    font-size: 11px;
    font-weight: 400;
    color: var(--wp-text-subtle);
  }
  .icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: var(--wp-r-sm);
    background: none;
    color: var(--wp-text-muted);
    cursor: pointer;
  }
  .icon:hover {
    background: var(--wp-surface);
    color: var(--wp-text);
  }
  .body {
    flex: 1;
    overflow: auto;
    padding: 10px 8px 14px;
  }
  .timeline {
    list-style: none;
    margin: 0;
    padding: 0;
    position: relative;
  }
  /* The branch: one line through every commit's dot. */
  .timeline::before {
    content: '';
    position: absolute;
    left: 15px;
    top: 10px;
    bottom: 10px;
    width: 2px;
    border-radius: 1px;
    background: var(--wp-border-strong);
  }
  .node button {
    position: relative;
    display: flex;
    gap: 10px;
    width: 100%;
    padding: 7px 8px 7px 8px;
    border: none;
    border-radius: var(--wp-r-md);
    background: none;
    color: var(--wp-text);
    text-align: left;
    cursor: pointer;
  }
  .node button:hover {
    background: var(--wp-surface);
  }
  .node.on button {
    background: color-mix(in srgb, var(--wp-accent) 12%, transparent);
  }
  .dot {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    margin-top: 1px;
    border-radius: 50%;
    background: var(--wp-elevated);
    border: 2px solid var(--wp-text-subtle);
    color: var(--wp-on-accent);
    z-index: 1;
  }
  .node.sub .dot {
    width: 10px;
    height: 10px;
    margin: 4px 3px 0;
  }
  .node.local .dot {
    border-style: dashed;
    border-color: var(--wp-text-muted);
    color: var(--wp-text-muted);
  }
  .node.incoming-head .dot,
  .node.incoming .dot {
    background: var(--wp-accent);
    border-color: var(--wp-accent);
  }
  .node.unpublished .dot {
    border-color: var(--wp-d-edited);
  }
  .node.base .dot,
  .node.on .dot {
    border-color: var(--wp-accent);
  }
  .text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .subj {
    font-size: 13px;
    line-height: 1.35;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .meta {
    font-size: 11.5px;
    color: var(--wp-text-subtle);
  }
  .hash {
    font-family: var(--wp-font-mono);
    color: var(--wp-text-muted);
  }
  .tag {
    margin-left: 4px;
    padding: 0 6px;
    border-radius: var(--wp-r-pill);
    background: color-mix(in srgb, var(--wp-d-edited) 16%, transparent);
    color: var(--wp-text);
  }
  .here {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 6px 0 2px 10px;
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--wp-accent);
    z-index: 1;
  }
  .here :global(svg) {
    background: var(--wp-elevated);
    border-radius: 50%;
  }
  .empty,
  .hint {
    margin: 10px 8px 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--wp-text-subtle);
  }
</style>

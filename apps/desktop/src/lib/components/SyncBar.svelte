<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { ArrowDown, ArrowUp, Check, X, GitBranch, ChevronDown, ChevronUp, RefreshCw, TriangleAlert } from 'lucide-svelte';
  import { currentProject, health } from '$lib/stores/board';
  import {
    syncStatus,
    syncMode,
    conflicts,
    picks,
    previewChanges,
    renumbered,
    syncBusy,
    syncLoading,
    syncError,
    syncNotice,
    unresolved,
    phrase,
    refreshSync,
    startSyncPolling,
    stopSyncPolling,
    openIncoming,
    openCommit,
    closeSync,
    applyIncoming,
    commitLocal,
    pick,
    pickAll,
    migrateBoard,
    historyCommit,
    taken,
    takeChange,
    takeAll,
    takeable,
    keyOf
  } from '$lib/stores/sync';
  import { formatDate } from '$lib/utils';
  import type { SyncChange, SyncConflict } from '$lib/types';

  let { onopen }: { onopen: (ticketId: string) => void } = $props();

  let message = $state('');
  let panelOpen = $state(true);
  let msgEl = $state<HTMLInputElement | null>(null);

  onMount(() => startSyncPolling());
  onDestroy(() => stopSyncPolling());
  // A different board: its own status, and no flow carried over.
  let lastProject: string | null = null;
  $effect(() => {
    const p = $currentProject;
    if (lastProject !== null && p !== lastProject) {
      void closeSync();
      void refreshSync({ fetch: true });
    }
    lastProject = p;
  });

  let st = $derived($syncStatus);
  let local = $derived(st?.mode === 'branch' ? st.local.length : 0);
  let incoming = $derived(st?.mode === 'branch' ? st.incoming.length : 0);
  let where = $derived(st ? (st.remote ? `${st.remote}/${st.branch}` : st.branch) : 'wipe');
  let authors = $derived([...new Set((st?.commits ?? []).map((c) => c.author_name))].slice(0, 3));

  // The changes the open flow is about.
  let shown = $derived<SyncChange[]>(
    $syncMode === 'incoming' || $syncMode === 'history'
      ? $previewChanges
      : $syncMode === 'commit'
        ? (st?.local ?? [])
        : []
  );
  // Changes can be applied one by one while previewing incoming or past states.
  let picking = $derived($syncMode === 'incoming' || $syncMode === 'history');
  let left = $derived($previewChanges.filter((c) => takeable(c) && !$taken.has(keyOf(c))).length);

  // Group changes for the panel: tickets, then forum, lists, labels, board.
  const GROUPS: [string, string][] = [
    ['ticket', 'Tickets'],
    ['thread', 'Forum'],
    ['list', 'Lists'],
    ['label', 'Labels'],
    ['board', 'Board'],
    ['file', 'Files']
  ];
  let grouped = $derived(
    GROUPS.map(([scope, name]) => ({ name, items: shown.filter((c) => c.scope === scope) })).filter(
      (g) => g.items.length
    )
  );

  function markClass(c: SyncChange): string {
    switch (c.kind) {
      case 'created':
        return 'created';
      case 'deleted':
        return 'deleted';
      case 'moved':
        return 'moved';
      case 'commented':
      case 'posted':
        return 'commented';
      case 'checked':
        return 'checked';
      default:
        return 'edited';
    }
  }

  function show(v: unknown): string {
    if (v === null || v === undefined) return '(deleted)';
    const s = typeof v === 'string' ? v : JSON.stringify(v);
    return s.length > 80 ? s.slice(0, 79) + '…' : s;
  }

  function conflictLabel(c: SyncConflict): string {
    if (!c.field) return `${c.entity} was deleted on one side and changed on the other`;
    if (c.field === 'list') return `${c.entity} moved to two different lists`;
    return `${c.entity} · ${c.field.replace(/\[([^\]]+)\]/g, ' $1').replace(/\//g, ' › ')}`;
  }

  async function startCommit() {
    openCommit();
    panelOpen = true;
    message = '';
    setTimeout(() => msgEl?.focus(), 30);
  }

  async function startIncoming() {
    panelOpen = true;
    await openIncoming();
  }

  async function doCommit() {
    if (await commitLocal(message)) message = '';
  }

  // Escape closes the open flow - unless a dialog (a ticket opened from the
  // preview) is open: that Escape is the dialog's. Checked while capturing, so
  // the dialog is still in the page when we look.
  function onKey(e: KeyboardEvent) {
    if (e.key !== 'Escape' || $syncMode === 'idle') return;
    if (document.querySelector('[aria-modal="true"]')) return;
    void closeSync();
  }

  function vcsLabel(v: string): string {
    return ({ plastic: 'Unity VCS', mercurial: 'Mercurial', svn: 'SVN', fossil: 'Fossil', jujutsu: 'Jujutsu' } as Record<string, string>)[v] ?? v;
  }
  function plainNote(v: string): string {
    return `${vcsLabel(v)} workspace: the board is plain files in .wipe/ - check them in with your code to share it. Commit, sync and history (the wipe branch) need git.`;
  }

  function fetchedAgo(): string {
    if (!st?.fetched) return 'not fetched yet';
    return `fetched ${formatDate(st.fetched)}`;
  }
</script>

<svelte:window onkeydowncapture={onKey} />

{#if $health && st && st.mode !== 'none'}
  <div class="sync" class:active={$syncMode !== 'idle'}>
    {#if st.mode === 'unmigrated'}
      <button
        class="pill warn"
        disabled={$syncBusy}
        title="This board predates the wipe branch. Move it there: the branch becomes the board's single source of truth, and .wipe/ stops being tracked with your code."
        onclick={() => {
          if (confirm(`Move this board onto the \`wipe\` branch?\n\nIt is published to the remote, and .wipe/ is untracked on the current code branch (one small commit). Same as \`wipe migrate\`.`))
            void migrateBoard();
        }}
      >
        <GitBranch size={14} /> Move board to the wipe branch
      </button>
    {:else if st.mode === 'tracked'}
      <span class="pill warn static" title="Run `wipe migrate` in this checkout to untrack .wipe/ here.">
        <TriangleAlert size={14} /> This branch still tracks .wipe/ - run <code>wipe migrate</code>
      </span>
    {:else if $syncMode === 'incoming'}
      <button class="round" aria-label="Close preview" title="Close (Esc)" onclick={() => closeSync()}>
        <X size={15} />
      </button>
      <span class="what">
        <ArrowDown size={14} />
        <b>{$syncLoading && !$previewChanges.length ? incoming : $previewChanges.length}</b> incoming
        {#if authors.length}<span class="who">· {authors.join(', ')}</span>{/if}
      </span>
      <button
        class="pill primary"
        disabled={$syncBusy || $syncLoading || $unresolved > 0}
        title={$unresolved ? 'Settle the conflicts first' : 'Pull these changes into your board'}
        onclick={() => applyIncoming()}
      >
        <Check size={14} />
        {$syncLoading ? 'Loading preview…' : $syncBusy ? 'Applying…' : $unresolved ? `${$unresolved} conflict${$unresolved === 1 ? '' : 's'} to settle` : `Apply ${$previewChanges.length} change${$previewChanges.length === 1 ? '' : 's'}`}
      </button>
    {:else if $syncMode === 'history' && $historyCommit}
      <button class="round" aria-label="Close history view" title="Back to your board (Esc)" onclick={() => closeSync()}>
        <X size={15} />
      </button>
      <span class="what hist" title={$historyCommit.subject}>
        <span class="hhash">{$historyCommit.short}</span>
        <span class="hsubj">{$historyCommit.subject}</span>
        <span class="who">· {$historyCommit.author_name}</span>
      </span>
      <button
        class="pill primary"
        disabled={$syncBusy || $syncLoading || left === 0}
        title="Bring every change of this commit into your board, as it was after it (uncommitted until you commit)"
        onclick={() => takeAll()}
      >
        <Check size={14} />
        {$syncLoading ? 'Loading…' : $syncBusy ? 'Applying…' : left ? `Apply ${left === $previewChanges.length ? 'all ' : ''}${left}` : 'All applied'}
      </button>
    {:else if $syncMode === 'commit'}
      <button class="round" aria-label="Cancel commit" title="Cancel (Esc)" onclick={() => closeSync()}>
        <X size={15} />
      </button>
      <input
        bind:this={msgEl}
        class="msg"
        placeholder="What changed and why (optional)"
        bind:value={message}
        onkeydown={(e) => {
          if (e.key === 'Enter' && !$syncBusy && $unresolved === 0) void doCommit();
        }}
      />
      <button
        class="pill primary"
        disabled={$syncBusy || $unresolved > 0 || local === 0}
        title="Record these changes on {where} and publish them"
        onclick={doCommit}
      >
        <ArrowUp size={14} />
        {$syncBusy ? 'Committing…' : $unresolved ? `${$unresolved} conflict${$unresolved === 1 ? '' : 's'} to settle` : `Commit ${local} & publish`}
      </button>
    {:else}
      {#if incoming}
        <button class="pill primary incoming" onclick={startIncoming} title="Changes your team committed to {where}">
          <span class="pulse"></span>
          <ArrowDown size={14} /> See {incoming} incoming change{incoming === 1 ? '' : 's'}
        </button>
      {/if}
      {#if local}
        <button class="pill" onclick={startCommit} title="Your board changes that aren't on {where} yet">
          <ArrowUp size={14} /> Commit {local} change{local === 1 ? '' : 's'}
        </button>
      {/if}
      {#if !incoming && !local}
        {#if $syncNotice}
          <span class="quiet ok"><Check size={13} /> {$syncNotice}</span>
        {:else}
          <button
            class="quiet"
            title="Synced with {where} · {fetchedAgo()}{st.unpublished ? ` · ${st.unpublished} commit(s) not published` : ''} - click to check now"
            onclick={() => refreshSync({ fetch: true, force: true })}
          >
            {#if st.unpublished}<TriangleAlert size={13} /> {st.unpublished} unpublished{:else}<Check size={13} /> Synced{/if}
            <RefreshCw size={12} class="again" />
          </button>
        {/if}
      {/if}
    {/if}

    {#if $syncMode !== 'idle'}
      <div class="panel" class:closed={!panelOpen}>
        <button class="ptoggle" onclick={() => (panelOpen = !panelOpen)}>
          {#if $syncMode === 'history' && $historyCommit}
            Changed in {$historyCommit.short} · {formatDate($historyCommit.date)}
          {:else if $syncMode === 'incoming'}
            Incoming from {where}
          {:else}
            Your uncommitted changes
          {/if}
          {#if panelOpen}<ChevronUp size={14} />{:else}<ChevronDown size={14} />{/if}
        </button>
        {#if panelOpen}
          <div class="pbody wp-scroll">
            {#if $syncError}<div class="err">{$syncError}</div>{/if}

            {#if $syncMode === 'commit' && incoming}
              <div class="note row">
                <ArrowDown size={13} />
                {incoming} incoming change{incoming === 1 ? '' : 's'} on {where} will be merged into your board first.
              </div>
            {/if}

            {#if $conflicts.length}
              <section class="conflicts">
                <header>
                  <TriangleAlert size={14} />
                  <span>{$conflicts.length} conflict{$conflicts.length === 1 ? '' : 's'} - both sides changed the same thing</span>
                  <span class="all">
                    <button onclick={() => pickAll('ours')}>Keep all mine</button>
                    <button onclick={() => pickAll('theirs')}>Take all theirs</button>
                  </span>
                </header>
                {#each $conflicts as c (c.id)}
                  <div class="conflict">
                    <div class="clabel">{conflictLabel(c)}</div>
                    <div class="sides">
                      <button class:on={$picks[c.id] === 'ours'} onclick={() => pick(c.id, 'ours')}>
                        <span class="side">Yours</span>
                        <span class="val">{show(c.ours)}</span>
                      </button>
                      <button class:on={$picks[c.id] === 'theirs'} onclick={() => pick(c.id, 'theirs')}>
                        <span class="side">Branch</span>
                        <span class="val">{show(c.theirs)}</span>
                      </button>
                    </div>
                  </div>
                {/each}
              </section>
            {/if}

            {#if $renumbered.length}
              <div class="note">
                {#each $renumbered as r (r.from)}
                  <span>Your new <b>{r.from}</b> becomes <b>{r.to}</b> - the branch already used that id.</span>
                {/each}
              </div>
            {/if}

            {#each grouped as g (g.name)}
              <section class="group">
                <h4>{g.name}</h4>
                {#each g.items as c, i (c.scope + c.id + c.kind + i)}
                  <div class="change" class:done={$taken.has(keyOf(c))}>
                    <button
                      class="cmain"
                      disabled={c.scope !== 'ticket' || c.kind === 'deleted'}
                      onclick={() => onopen(c.id)}
                    >
                      <span class="dot {markClass(c)}"></span>
                      {#if c.id && c.scope !== 'board'}<span class="cid">{c.id}</span>{/if}
                      <span class="ctitle">{c.title || (c.scope === 'board' ? (c.fields ?? []).join(', ') : '')}</span>
                      <span class="cwhat {markClass(c)}">{c.scope === 'board' ? 'updated' : phrase(c)}</span>
                      {#if c.by?.length}<span class="cby">{c.by.map((b) => b.replace(/\s*<[^>]*>/, '')).join(', ')}</span>{/if}
                    </button>
                    {#if picking && takeable(c)}
                      {#if $taken.has(keyOf(c))}
                        <span class="applied"><Check size={12} /> Applied</span>
                      {:else}
                        <button
                          class="apply"
                          disabled={$syncBusy}
                          title={$syncMode === 'history'
                            ? 'Bring this into your board as it was after this commit'
                            : 'Bring just this change into your board'}
                          onclick={() => takeChange(c)}>Apply</button
                        >
                      {/if}
                    {/if}
                  </div>
                {/each}
              </section>
            {/each}

            {#if $syncMode === 'incoming' && st.commits.length}
              <section class="group commits">
                <h4>Commits</h4>
                {#each st.commits.slice(0, 8) as c (c.hash)}
                  <div class="commit">
                    <span class="hash">{c.short}</span>
                    <span class="subj">{c.subject}</span>
                    <span class="cby">{c.author_name} · {formatDate(c.date)}</span>
                  </div>
                {/each}
              </section>
            {/if}

            {#if !shown.length && !$conflicts.length && !$syncBusy && !$syncLoading}
              <div class="empty">Nothing here any more - it may already be synced.</div>
            {/if}
          </div>
        {/if}
      </div>
    {/if}
  </div>
{:else if $health && st && st.mode === 'none' && st.vcs}
  <div class="sync">
    <span class="pill static plain" title={plainNote(st.vcs)}>
      <GitBranch size={13} /> Board travels with your {vcsLabel(st.vcs)} {st.vcs === 'plastic' || st.vcs === 'svn' ? 'checkins' : 'commits'}
    </span>
  </div>
{/if}

<style>
  .sync {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px;
    border-radius: var(--wp-r-pill);
    border: 1px solid var(--wp-border-strong);
    background: var(--wp-card);
    color: var(--wp-text);
    font-size: 12.5px;
    font-weight: 500;
    white-space: nowrap;
    cursor: pointer;
    transition: all var(--wp-fast) var(--wp-ease);
  }
  .pill:hover:not(:disabled) {
    background: var(--wp-elevated);
    border-color: var(--wp-text-subtle);
  }
  .pill:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .pill.primary {
    background: var(--wp-accent);
    border-color: var(--wp-accent);
    color: var(--wp-on-accent);
    box-shadow: 0 2px 10px color-mix(in srgb, var(--wp-accent) 35%, transparent);
  }
  .pill.primary:hover:not(:disabled) {
    background: var(--wp-accent-hover);
    border-color: var(--wp-accent-hover);
  }
  .pill.warn {
    border-color: color-mix(in srgb, var(--wp-d-edited) 60%, var(--wp-border));
    color: var(--wp-text);
    background: color-mix(in srgb, var(--wp-d-edited) 12%, var(--wp-card));
  }
  .pill.static {
    cursor: default;
  }
  .pill.plain {
    height: 26px;
    border-color: var(--wp-border);
    background: transparent;
    color: var(--wp-text-subtle);
    font-weight: 400;
  }
  .pill code {
    font-family: var(--wp-font-mono);
    font-size: 11.5px;
  }
  .pulse {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--wp-on-accent);
    animation: pulse 1.6s var(--wp-ease) infinite;
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.35;
      transform: scale(0.7);
    }
  }
  .round {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    border: 1px solid var(--wp-border);
    background: var(--wp-card);
    color: var(--wp-text-muted);
    cursor: pointer;
  }
  .round:hover {
    color: var(--wp-text);
    background: var(--wp-elevated);
  }
  .what {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 13px;
    color: var(--wp-text);
    white-space: nowrap;
  }
  .what .who {
    color: var(--wp-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 220px;
  }
  .msg {
    width: min(360px, 34vw);
    height: 30px;
    padding: 0 12px;
    border-radius: var(--wp-r-pill);
    border: 1px solid var(--wp-border-strong);
    background: var(--wp-card);
    color: var(--wp-text);
    font: inherit;
    font-size: 13px;
  }
  .msg:focus {
    outline: none;
    border-color: var(--wp-accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--wp-accent) 20%, transparent);
  }
  .quiet {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 8px;
    border: none;
    border-radius: var(--wp-r-pill);
    background: none;
    color: var(--wp-text-subtle);
    font-size: 12px;
    cursor: pointer;
  }
  .quiet:hover {
    color: var(--wp-text-muted);
    background: var(--wp-surface);
  }
  .quiet :global(.again) {
    opacity: 0;
    transition: opacity var(--wp-fast) var(--wp-ease);
  }
  .quiet:hover :global(.again) {
    opacity: 1;
  }
  .quiet.ok {
    color: var(--wp-d-created);
    cursor: default;
  }

  /* The dropdown panel: what the open flow is about. */
  .panel {
    position: absolute;
    top: calc(100% + 10px);
    left: 50%;
    transform: translateX(-50%);
    width: min(560px, 92vw);
    z-index: 40;
    background: var(--wp-elevated);
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-lg);
    box-shadow: var(--wp-shadow-lift);
    overflow: hidden;
    animation: drop var(--wp-base) var(--wp-ease);
  }
  @keyframes drop {
    from {
      opacity: 0;
      transform: translate(-50%, -6px);
    }
  }
  .ptoggle {
    display: flex;
    width: 100%;
    align-items: center;
    justify-content: space-between;
    padding: 9px 14px;
    border: none;
    background: var(--wp-surface);
    color: var(--wp-text-muted);
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    cursor: pointer;
  }
  .panel.closed .ptoggle {
    background: var(--wp-elevated);
  }
  .pbody {
    max-height: min(56vh, 520px);
    overflow: auto;
    padding: 8px 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .err {
    padding: 8px 10px;
    border-radius: var(--wp-r-md);
    background: color-mix(in srgb, var(--wp-error) 12%, transparent);
    color: var(--wp-error);
    font-size: 12.5px;
  }
  .note {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    border-radius: var(--wp-r-md);
    background: color-mix(in srgb, var(--wp-d-moved) 10%, transparent);
    color: var(--wp-text-muted);
    font-size: 12.5px;
  }
  .note.row {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  .conflicts {
    border: 1px solid color-mix(in srgb, var(--wp-d-edited) 55%, var(--wp-border));
    border-radius: var(--wp-r-md);
    background: color-mix(in srgb, var(--wp-d-edited) 7%, transparent);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .conflicts header {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--wp-text);
  }
  .conflicts .all {
    margin-left: auto;
    display: inline-flex;
    gap: 4px;
  }
  .conflicts .all button {
    border: 1px solid var(--wp-border);
    background: var(--wp-card);
    color: var(--wp-text-muted);
    border-radius: var(--wp-r-sm);
    font-size: 11.5px;
    padding: 2px 8px;
    cursor: pointer;
  }
  .conflicts .all button:hover {
    color: var(--wp-text);
  }
  .clabel {
    font-family: var(--wp-font-mono);
    font-size: 11.5px;
    color: var(--wp-text-muted);
    margin-bottom: 4px;
  }
  .sides {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }
  .sides button {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    text-align: left;
    padding: 6px 9px;
    border-radius: var(--wp-r-md);
    border: 1px solid var(--wp-border);
    background: var(--wp-card);
    color: var(--wp-text);
    cursor: pointer;
    min-width: 0;
  }
  .sides button.on {
    border-color: var(--wp-accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--wp-accent) 25%, transparent);
  }
  .sides .side {
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--wp-text-subtle);
  }
  .sides .val {
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .group h4 {
    margin: 4px 6px 2px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--wp-text-subtle);
  }
  .change {
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: var(--wp-r-sm);
  }
  .change:hover {
    background: var(--wp-surface);
  }
  .cmain {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: none;
    background: none;
    color: var(--wp-text);
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  .cmain:disabled {
    cursor: default;
  }
  .change.done .cmain {
    opacity: 0.55;
  }
  /* Apply one change: revealed on the row's hover. */
  .apply {
    flex: none;
    margin-right: 6px;
    padding: 2px 9px;
    border-radius: var(--wp-r-pill);
    border: 1px solid var(--wp-accent);
    background: var(--wp-accent);
    color: var(--wp-on-accent);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--wp-fast) var(--wp-ease);
  }
  .change:hover .apply,
  .apply:focus-visible {
    opacity: 1;
  }
  .applied {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    margin-right: 8px;
    font-size: 11.5px;
    color: var(--wp-d-created);
  }
  .what.hist {
    min-width: 0;
    max-width: min(420px, 40vw);
    overflow: hidden;
  }
  .hhash {
    font-family: var(--wp-font-mono);
    font-size: 12px;
    color: var(--wp-accent);
  }
  .hsubj {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  @media (hover: none) {
    .apply {
      opacity: 1;
    }
  }
  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--wp-d-edited);
  }
  .cid {
    flex: none;
    font-family: var(--wp-font-mono);
    font-size: 11.5px;
    color: var(--wp-text-muted);
  }
  .ctitle {
    min-width: 0;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cwhat {
    flex: none;
    font-size: 11.5px;
    padding: 1px 7px;
    border-radius: var(--wp-r-pill);
    background: color-mix(in srgb, var(--wp-d-edited) 14%, transparent);
    color: var(--wp-text);
  }
  .cby {
    flex: none;
    font-size: 11.5px;
    color: var(--wp-text-subtle);
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot.created,
  .cwhat.created {
    --c: var(--wp-d-created);
  }
  .dot.deleted,
  .cwhat.deleted {
    --c: var(--wp-d-deleted);
  }
  .dot.moved,
  .cwhat.moved {
    --c: var(--wp-d-moved);
  }
  .dot.edited,
  .cwhat.edited {
    --c: var(--wp-d-edited);
  }
  .dot.commented,
  .cwhat.commented {
    --c: var(--wp-d-commented);
  }
  .dot.checked,
  .cwhat.checked {
    --c: var(--wp-d-checked);
  }
  .dot {
    background: var(--c);
  }
  .cwhat {
    background: color-mix(in srgb, var(--c) 15%, transparent);
  }
  .commit {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 4px 8px;
    font-size: 12.5px;
  }
  .hash {
    font-family: var(--wp-font-mono);
    font-size: 11.5px;
    color: var(--wp-accent);
  }
  .subj {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty {
    padding: 14px;
    text-align: center;
    color: var(--wp-text-subtle);
    font-size: 12.5px;
  }
  @media (max-width: 700px) {
    .msg {
      width: 44vw;
    }
    .what .who {
      display: none;
    }
    .pill {
      padding: 0 10px;
    }
  }
</style>

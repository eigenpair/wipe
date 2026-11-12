<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    RefreshCw,
    Settings,
    GitCommitVertical,
    WifiOff,
    LayoutGrid,
    MessagesSquare,
    Hash,
    Search
  } from 'lucide-svelte';
  import { page } from '$app/state';
  import { tick } from 'svelte';
  import {
    board,
    boardError,
    health,
    healthError,
    currentProject,
    readOnly,
    loading,
    forumUnread,
    loadForum,
    bootstrap,
    checkHealth,
    loadBoard,
    loadProjects,
    reloadProject,
    setForumView,
    stopLiveUpdates
  } from '$lib/stores/board';
  import { api, getApiBase } from '$lib/api';
  import { parseRoute, go, openTicket as openTicketRoute, openPost, type Route } from '$lib/nav';
  import RefPreview from '$lib/components/RefPreview.svelte';
  import MentionMenu from '$lib/components/MentionMenu.svelte';
  import ConvertDialog from '$lib/components/ConvertDialog.svelte';
  import SearchBar from '$lib/components/SearchBar.svelte';
  import QuickJump from '$lib/components/QuickJump.svelte';
  import NotifyMenu from '$lib/components/NotifyMenu.svelte';
  import Toasts from '$lib/components/Toasts.svelte';
  import { openRequest } from '$lib/stores/notify';
  import Board from '$lib/components/Board.svelte';
  import Forum from '$lib/components/Forum.svelte';
  import ProjectSwitcher from '$lib/components/ProjectSwitcher.svelte';
  import Lightbox from '$lib/components/Lightbox.svelte';
  import TicketModal from '$lib/components/TicketModal.svelte';
  import NewTicketDialog from '$lib/components/NewTicketDialog.svelte';
  import HistoryDrawer from '$lib/components/HistoryDrawer.svelte';
  import ChangelogDialog from '$lib/components/ChangelogDialog.svelte';
  import BoardSettings from '$lib/components/BoardSettings.svelte';
  import ThemeToggle from '$lib/components/ThemeToggle.svelte';
  import SyncBar from '$lib/components/SyncBar.svelte';
  import { refreshSync } from '$lib/stores/sync';
  import Button from '$lib/components/ui/Button.svelte';
  import type { Ticket } from '$lib/types';

  // What is open lives in the URL (lib/nav.ts): back/forward walk every jump.
  let route = $derived(parseRoute(page.url));
  let view = $derived(route.view);
  let modalTicketId = $derived(route.ticket ?? null);
  let newTicketOpen = $state(false);
  let newTicketList = $state('');
  let newTicketName = $state('');

  let settingsOpen = $state(false);
  let historyOpen = $state(false);
  let changelogOpen = $state(false);

  // The forum tab returns to where you were in the forum.
  let lastForum: Route = { view: 'forum' };
  $effect(() => {
    if (route.view === 'forum') lastForum = route;
  });

  // Keep the store informed of whether the forum is open, so it can clear the
  // unread indicator while you're looking at it.
  $effect(() => setForumView(view === 'forum'));

  // `/?at=T-023`: scroll that card into view and highlight it.
  $effect(() => {
    const at = route.at;
    if (!at || !$board) return;
    void tick().then(() => {
      const el = document.querySelector<HTMLElement>(`[data-ticket-id="${CSS.escape(at)}"]`);
      if (!el) return;
      el.scrollIntoView({ behavior: 'smooth', block: 'center', inline: 'center' });
      el.classList.remove('wp-flash');
      void el.offsetWidth;
      el.classList.add('wp-flash');
      setTimeout(() => el.classList.remove('wp-flash'), 1700);
    });
  });

  onMount(() => {
    void bootstrap();
  });
  onDestroy(() => stopLiveUpdates());

  async function selectProject(path: string) {
    currentProject.set(path);
    void go({ view: route.view });
    await reloadProject();
  }

  function openTicket(t: Ticket) {
    void openTicketRoute(t.id);
  }
  function addToList(listId: string, listName: string) {
    newTicketList = listId;
    newTicketName = listName;
    newTicketOpen = true;
  }

  // A clicked notification/toast: open the ticket, or switch to the forum
  // (which opens the thread itself).
  $effect(() => {
    const r = $openRequest;
    if (!r) return;
    openRequest.set(null);
    if (r.ticket) void openTicketRoute(r.ticket);
    else if (r.post ?? r.thread) void openPost((r.post ?? r.thread)!);
  });

  // Boards from before 1.1 convert once: decimal ticket ids (T-23 -> T-017)
  // and the thread forum (one file per post, F-3.1 -> F-00A).
  let legacyIds = $derived(!!$board && $board.ids !== 'hex');
  let legacyForum = $derived(!!$board && $board.forum !== 'posts');
  let translating = $state(false);
  let convertOpen = $state(false);
  async function translateIds() {
    translating = true;
    try {
      await api.translateIds($currentProject ?? undefined);
      await loadBoard();
      await loadForum();
    } catch (e) {
      boardError.set(e instanceof Error ? e.message : String(e));
    } finally {
      translating = false;
    }
  }

  const mod =
    typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl+';

  // In the forum, Ctrl/Cmd+F jumps to the forum's own search box.
  function onKey(e: KeyboardEvent) {
    if (view !== 'forum' || !(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== 'f') return;
    const box = document.querySelector<HTMLInputElement>('.forum input.sin');
    if (box) {
      e.preventDefault();
      box.focus();
      box.select();
    }
  }

  async function refresh() {
    // If we were offline (the "Retry connection" path), a plain reload would show
    // a board that then never updates: bootstrap() is what starts the 0.5s poll and
    // the change WebSocket, and it was skipped while the daemon was down. Re-run it
    // so a recovered connection resumes live updates instead of freezing.
    const wasOffline = !$health;
    if (!(await checkHealth())) return;
    if (wasOffline) {
      await bootstrap();
      return;
    }
    await loadProjects();
    await loadBoard();
    // Also ask the remote: what teammates committed shows up now, not within a minute.
    await refreshSync({ fetch: true, force: true });
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="app">
  <header class="topbar">
    <div class="brand">
      <span class="wordmark">wipe</span>
    </div>
    <div class="sep"></div>
    <ProjectSwitcher onselect={selectProject} />

    <div class="viewtabs">
      <button class:on={view === 'board'} onclick={() => go({ view: 'board' })}>
        <LayoutGrid size={14} /> Board
      </button>
      <button class:on={view === 'forum'} onclick={() => go(lastForum)}>
        <MessagesSquare size={14} /> Forum
        {#if $forumUnread && view !== 'forum'}<span class="unread" title="New forum activity"></span>{/if}
      </button>
    </div>
    <button
      class="findhint"
      title="Search the {view} ({mod}F)"
      onclick={() =>
        window.dispatchEvent(new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, bubbles: true }))}
    >
      <Search size={13} />
      <span class="fh-txt">Search</span>
      <kbd>{mod}F</kbd>
    </button>

    <div class="syncslot">
      <SyncBar onopen={(id) => openTicketRoute(id)} />
    </div>

    <div class="right">
      {#if (legacyIds || legacyForum) && !$readOnly}
        <button
          class="ib wide"
          disabled={translating}
          title={legacyIds
            ? 'This board uses the old ids: convert ticket ids to fixed-width hex (T-017) and the forum to one post per id (F-00A).'
            : 'This board uses the old thread forum: convert it so every post has its own id (F-00A) and replies by @ tag.'}
          onclick={() => (convertOpen = true)}
        >
          <Hash size={14} />
          <span>{translating ? 'Converting…' : 'New ids'}</span>
        </button>
      {/if}
      {#if $health}
        <button
          class="status ok asbtn"
          title="wipe v{$health.version} - what's new"
          aria-label="What's new in wipe"
          onclick={() => (changelogOpen = true)}
        >
          <span class="dot"></span>
          <span class="stxt">v{$health.version}</span>
        </button>
      {:else}
        <span class="status off"><span class="dot"></span><span class="stxt">offline</span></span>
      {/if}

      <NotifyMenu />
      <button
        class="ib"
        class:on={historyOpen}
        aria-label="Board history"
        title="Board history - the wipe branch"
        onclick={() => (historyOpen = !historyOpen)}
      >
        <GitCommitVertical size={16} />
      </button>
      <button class="ib" aria-label="Refresh" title="Refresh" onclick={refresh}>
        <RefreshCw size={16} class={$loading ? 'spin' : ''} />
      </button>
      <ThemeToggle />
      <button class="ib" aria-label="Settings" title="Board settings" onclick={() => (settingsOpen = true)}>
        <Settings size={16} />
      </button>
    </div>
  </header>

  <main class="main">
    {#if !$health}
      <div class="offline">
        <div class="offcard">
          <div class="officon"><WifiOff size={22} /></div>
          <h2>Can't reach the wipe daemon</h2>
          <p>
            Start it from your project with
            <code>wipe serve</code>
            then retry. Expecting the API at
            <span class="mono">{getApiBase()}</span>.
          </p>
          {#if $healthError}<p class="dim">({$healthError})</p>{/if}
          <div class="offactions">
            <Button variant="primary" onclick={refresh}>Retry connection</Button>
            <Button onclick={() => (settingsOpen = true)}>Change API URL</Button>
          </div>
        </div>
      </div>
    {:else if view === 'forum'}
      <div class="boardwrap"><Forum focus={route.post ?? null} /></div>
    {:else}

      {#if $boardError}
        <div class="banner err">{$boardError}</div>
      {/if}

      <SearchBar enabled={view === 'board' && !!$health} />

      <div class="boardwrap">
        {#if $board}
          <Board onopen={openTicket} onadd={addToList} />
        {:else if $loading}
          <div class="loading">Loading board…</div>
        {/if}
      </div>
    {/if}
  </main>
</div>

<Toasts />
<QuickJump enabled={view === 'board' && !!$health && !$readOnly} onopen={openTicket} />
<TicketModal ticketId={modalTicketId} onclose={() => go({ view: 'board' })} />
<NewTicketDialog bind:open={newTicketOpen} listId={newTicketList} listName={newTicketName} />
<HistoryDrawer bind:open={historyOpen} />
<ChangelogDialog bind:open={changelogOpen} />
<BoardSettings bind:open={settingsOpen} />
<Lightbox />
<RefPreview />
<ConvertDialog bind:open={convertOpen} {legacyIds} {legacyForum} busy={translating} onconfirm={translateIds} />
<MentionMenu />

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    /* Mobile browsers: size to the visible viewport, not behind the URL bar. */
    height: 100dvh;
    background: var(--wp-canvas);
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--wp-border);
    background: var(--wp-canvas);
  }
  .wordmark {
    font-family: var(--wp-font-display);
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--wp-text);
  }
  .sep {
    width: 1px;
    height: 20px;
    background: var(--wp-border);
  }
  .viewtabs {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--wp-r-md);
    background: var(--wp-surface);
    border: 1px solid var(--wp-border);
  }
  /* The find hint beside the view switch: Ctrl/Cmd+F searches the view. */
  .findhint {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 6px 0 9px;
    border-radius: var(--wp-r-md);
    border: 1px solid var(--wp-border);
    background: none;
    color: var(--wp-text-subtle);
    font-size: 12px;
    cursor: pointer;
    transition:
      color var(--wp-fast) var(--wp-ease),
      border-color var(--wp-fast) var(--wp-ease);
  }
  .findhint:hover {
    color: var(--wp-text);
    border-color: var(--wp-border-strong);
  }
  .findhint kbd {
    font-family: var(--wp-font-mono);
    font-size: 10.5px;
    padding: 1px 5px;
    border-radius: 4px;
    border: 1px solid var(--wp-border);
    background: var(--wp-surface);
    color: var(--wp-text-muted);
  }
  @media (max-width: 860px) {
    .findhint .fh-txt,
    .findhint kbd {
      display: none;
    }
  }
  .viewtabs button {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 10px;
    border: none;
    border-radius: var(--wp-r-sm);
    background: none;
    color: var(--wp-text-muted);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
  }
  .viewtabs .unread {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--wp-accent);
    box-shadow: 0 0 0 2px var(--wp-surface);
  }
  .viewtabs button:hover {
    color: var(--wp-text);
  }
  .viewtabs button.on {
    background: var(--wp-card);
    color: var(--wp-text);
    box-shadow: var(--wp-shadow);
  }
  .syncslot {
    flex: 1;
    display: flex;
    justify-content: center;
    min-width: 0;
  }
  .right {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: var(--wp-font-mono);
    font-size: 12px;
    color: var(--wp-text-muted);
  }
  .status.asbtn {
    border: 1px solid transparent;
    background: none;
    padding: 3px 8px;
    border-radius: var(--wp-r-pill);
    cursor: pointer;
    transition: all var(--wp-fast) var(--wp-ease);
  }
  .status.asbtn:hover {
    border-color: var(--wp-border);
    color: var(--wp-text);
  }
  .status .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .status.ok .dot {
    background: #7e9b7a;
  }
  .status.off .dot {
    background: var(--wp-error);
  }
  .ib {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 32px;
    width: 32px;
    border-radius: var(--wp-r-sm);
    border: 1px solid var(--wp-border);
    background: var(--wp-card);
    color: var(--wp-text-muted);
    cursor: pointer;
    transition: all var(--wp-fast) var(--wp-ease);
  }
  .ib:hover {
    background: var(--wp-elevated);
    color: var(--wp-text);
  }
  .ib.on {
    color: var(--wp-accent);
    border-color: color-mix(in srgb, var(--wp-accent) 45%, var(--wp-border));
  }
  .ib.wide {
    width: auto;
    gap: 6px;
    padding: 0 10px;
    font-size: 12px;
    font-weight: 500;
    border-color: color-mix(in srgb, var(--wp-accent) 55%, var(--wp-border));
    color: var(--wp-accent);
  }
  :global(.spin) {
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 14px 16px 16px;
    gap: 12px;
  }
  .boardwrap {
    flex: 1;
    min-height: 0;
  }
  .loading {
    display: flex;
    height: 100%;
    align-items: center;
    justify-content: center;
    color: var(--wp-text-muted);
  }
  .banner.err {
    padding: 8px 12px;
    border-radius: var(--wp-r-md);
    border: 1px solid color-mix(in srgb, var(--wp-error) 40%, transparent);
    background: color-mix(in srgb, var(--wp-error) 12%, transparent);
    color: var(--wp-error);
    font-size: 13px;
  }
  .offline {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .offcard {
    max-width: 420px;
    text-align: center;
    padding: 28px;
    background: var(--wp-card);
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-lg);
    box-shadow: var(--wp-shadow);
  }
  .officon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    border-radius: var(--wp-r-pill);
    background: color-mix(in srgb, var(--wp-error) 12%, transparent);
    color: var(--wp-error);
    margin-bottom: 14px;
  }
  .offcard h2 {
    font-family: var(--wp-font-display);
    font-size: 17px;
    font-weight: 600;
    margin-bottom: 8px;
  }
  .offcard p {
    font-size: 13px;
    color: var(--wp-text-muted);
    margin: 0 0 8px;
    line-height: 1.5;
  }
  .offcard code,
  .mono {
    font-family: var(--wp-font-mono);
    font-size: 12px;
  }
  .offcard code {
    background: var(--wp-surface);
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-sm);
    padding: 1px 5px;
    color: var(--wp-text);
  }
  .dim {
    color: var(--wp-text-subtle);
    font-size: 12px;
  }
  /* Phones: two header rows - brand, project and actions on top, the
     Board/Forum switch full-width below - and tighter page padding. */
  @media (max-width: 700px) {
    .topbar {
      flex-wrap: wrap;
      gap: 8px 10px;
      padding: 8px 12px;
    }
    .sep,
    .status {
      display: none;
    }
    .topbar :global(> *:nth-child(3)) {
      min-width: 0;
      flex: 1 1 auto;
    }
    .viewtabs {
      order: 10;
      width: 100%;
    }
    .syncslot {
      order: 11;
      flex-basis: 100%;
    }
    .syncslot:empty {
      display: none;
    }
    .viewtabs button {
      flex: 1;
      justify-content: center;
      height: 34px;
      font-size: 13px;
    }
    .right {
      gap: 6px;
    }
    .ib {
      width: 36px;
      height: 36px;
    }
    .main {
      padding: 10px 0 0;
    }
    .banner.err {
      margin: 0 12px;
    }
  }
  .offactions {
    display: flex;
    justify-content: center;
    gap: 8px;
    margin-top: 16px;
  }
</style>

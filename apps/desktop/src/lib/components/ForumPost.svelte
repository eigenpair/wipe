<script lang="ts">
  // One forum post in a thread, with the replies nested under it (each reply
  // sits under its first parent; any others show as "replying to" chips).
  // The summary is the post's header line.
  import { Trash2, Reply, Pencil, Link as LinkIcon, CornerDownRight, Paperclip } from 'lucide-svelte';
  import { get } from 'svelte/store';
  import Self from './ForumPost.svelte';
  import Avatar from './Avatar.svelte';
  import Markdown from './Markdown.svelte';
  import Chip from './ui/Chip.svelte';
  import MdToolbar from './MdToolbar.svelte';
  import { identities, definitions, currentProject } from '$lib/stores/board';
  import { mediaUrl } from '$lib/api';
  import { openLightbox } from '$lib/stores/lightbox';
  import { mdkeys, mdlists } from '$lib/md';
  import { mention } from '$lib/mention';
  import { splitReplyTags } from '$lib/forum';
  import { refHtml } from '$lib/refs';
  import { formatDate, labelColorFor, mediaKind, autosize } from '$lib/utils';
  import type { ForumPost } from '$lib/types';

  let {
    post,
    kids,
    depth = 0,
    meId = null,
    onreply,
    onsave,
    ondelete,
    oncopy
  }: {
    post: ForumPost;
    kids: Map<string, ForumPost[]>;
    depth?: number;
    meId?: string | null;
    onreply: (p: ForumPost) => void;
    onsave: (p: ForumPost, summary: string, body: string) => Promise<void>;
    ondelete: (p: ForumPost) => void;
    oncopy: (p: ForumPost) => void;
  } = $props();

  function identityFor(id: string) {
    const key = (id.match(/<([^>]+)>/)?.[1] ?? id).toLowerCase();
    return $identities.find((i) => i.id.toLowerCase() === key);
  }
  function displayName(id: string) {
    return identityFor(id)?.display_name ?? id.replace(/\s*<[^>]+>$/, '');
  }
  let mine = $derived(
    !!meId && (identityFor(post.author)?.id ?? post.author).toLowerCase() === meId.toLowerCase()
  );
  // Only labels that still exist in the board's pool.
  let poolLabels = $derived(
    (post.labels ?? []).filter((l) => $definitions.labels.some((d) => d.name === l))
  );
  // The body without the post tags it opens with: those show as "replying to".
  let shown = $derived.by(() => {
    const { tags, rest } = splitReplyTags(post.body);
    const keep = tags.filter((t) => !/^F-/.test(t));
    return (keep.length ? keep.map((t) => `@${t}`).join(' ') + ' ' : '') + rest;
  });
  let children = $derived(kids.get(post.id) ?? []);

  // --- inline edit ---
  let editing = $state(false);
  let eSummary = $state('');
  let eBody = $state('');
  let saving = $state(false);
  let bodyEl = $state<HTMLTextAreaElement | null>(null);
  function startEdit() {
    eSummary = post.summary;
    eBody = post.body;
    editing = true;
  }
  async function save() {
    if (!eSummary.trim() || saving) return;
    saving = true;
    try {
      await onsave(post, eSummary.trim(), eBody);
      editing = false;
    } finally {
      saving = false;
    }
  }
  function proj() {
    return get(currentProject) ?? undefined;
  }
</script>

<div class="node" class:root={depth === 0}>
  <article class="row" data-post-id={post.id} id="post-{post.id}">
    <Avatar id={identityFor(post.author)?.id ?? post.author} identity={identityFor(post.author)} size={depth === 0 ? 30 : 26} />
    <div class="bubble" class:mine>
      <div class="p-head">
        <span class="p-author">{displayName(post.author)}</span>
        <span class="p-time" title={post.created}>{formatDate(post.created)}{post.edited ? ' · edited' : ''}</span>
        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
        <span class="p-id">{@html refHtml('post', post.id)}</span>
        <div class="p-actions">
          <button class="pa" title="Reply" aria-label="Reply" onclick={() => onreply(post)}><Reply size={13} /></button>
          <button class="pa" title="Copy link" aria-label="Copy link" onclick={() => oncopy(post)}><LinkIcon size={13} /></button>
          <button class="pa" title="Edit" aria-label="Edit" onclick={startEdit}><Pencil size={13} /></button>
          <button class="pa danger" title="Delete this post (replies stay)" aria-label="Delete" onclick={() => ondelete(post)}>
            <Trash2 size={13} />
          </button>
        </div>
      </div>
      {#if editing}
        <div class="edit">
          <input class="e-sum" bind:value={eSummary} placeholder="Summary (required)" use:mention use:mdkeys />
          <MdToolbar target={bodyEl} />
          <textarea
            class="e-body"
            bind:this={bodyEl}
            bind:value={eBody}
            use:autosize={eBody}
            use:mention
            use:mdkeys
            use:mdlists
            onkeydown={(e) => {
              if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) void save();
              if (e.key === 'Escape') {
                e.preventDefault();
                editing = false;
              }
            }}
          ></textarea>
          <div class="e-bar">
            <button class="ghost" onclick={() => (editing = false)}>Cancel</button>
            <button class="prim" disabled={!eSummary.trim() || saving} onclick={save}>{saving ? 'Saving…' : 'Save'}</button>
          </div>
        </div>
      {:else}
        {#if depth > 0}
          <div class="p-summary"><Markdown source={post.summary} inline /></div>
        {/if}
        {#if post.parents.length && (depth > 0 || post.parents.length > 0)}
          <div class="p-to">
            <CornerDownRight size={11} /> replying to
            {#each post.parents as id (id)}
              <!-- eslint-disable-next-line svelte/no-at-html-tags -->
              {@html refHtml('post', id)}
            {/each}
          </div>
        {/if}
        {#if poolLabels.length}
          <div class="p-labels">
            {#each poolLabels as l (l)}
              <Chip color={labelColorFor(l, $definitions.labels)}>{l}</Chip>
            {/each}
          </div>
        {/if}
        {#if shown.trim()}
          <div class="p-body"><Markdown source={shown} /></div>
        {/if}
        {#if post.media.length}
          <div class="p-media">
            {#each post.media as m (m.path)}
              {#if mediaKind(m.mime, m.name) === 'image'}
                <button
                  class="thumb"
                  onclick={() => openLightbox({ src: mediaUrl(m.path, proj()), name: m.name, kind: 'image' })}
                >
                  <img src={mediaUrl(m.path, proj())} alt={m.name} loading="lazy" />
                </button>
              {:else}
                <button
                  class="file"
                  onclick={() =>
                    openLightbox({ src: mediaUrl(m.path, proj()), name: m.name, kind: mediaKind(m.mime, m.name) })}
                >
                  <Paperclip size={12} />{m.name}
                </button>
              {/if}
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  </article>

  {#if children.length}
    <div class="children" class:flat={depth >= 5}>
      {#each children as reply (reply.id)}
        <Self post={reply} {kids} depth={depth + 1} {meId} {onreply} {onsave} {ondelete} {oncopy} />
      {/each}
    </div>
  {/if}
</div>

<style>
  .node {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .row {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    border-radius: var(--wp-r-md);
    scroll-margin: 80px;
  }
  .bubble {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-md);
    background: var(--wp-surface);
    padding: 8px 12px 10px;
  }
  .root > .row .bubble {
    background: var(--wp-card);
  }
  .bubble.mine {
    border-color: color-mix(in srgb, var(--wp-accent) 30%, var(--wp-border));
  }
  .p-head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .p-author {
    font-size: 13px;
    font-weight: 600;
  }
  .p-time {
    font-size: 11px;
    color: var(--wp-text-subtle);
  }
  .p-id {
    font-size: 11px;
    opacity: 0.75;
  }
  .p-actions {
    margin-left: auto;
    display: flex;
    gap: 2px;
    opacity: 0;
    transition: opacity var(--wp-fast) var(--wp-ease);
  }
  .row:hover .p-actions,
  .row:focus-within .p-actions {
    opacity: 1;
  }
  .pa {
    display: inline-flex;
    padding: 4px;
    border: none;
    background: none;
    color: var(--wp-text-subtle);
    cursor: pointer;
    border-radius: var(--wp-r-sm);
  }
  .pa:hover {
    color: var(--wp-text);
    background: var(--wp-elevated);
  }
  .pa.danger:hover {
    color: var(--wp-error);
  }
  .p-summary {
    margin-top: 4px;
    font-family: var(--wp-font-display);
    font-size: 14px;
    font-weight: 600;
    line-height: 1.35;
  }
  .p-to {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 3px;
    font-size: 11px;
    color: var(--wp-text-subtle);
  }
  .p-labels {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: 6px 0 2px;
  }
  .p-body {
    margin-top: 5px;
  }
  .p-body :global(.md) {
    font-size: 13.5px;
    line-height: 1.55;
  }
  .p-body :global(table) {
    display: block;
    overflow-x: auto;
    max-width: 100%;
  }
  .p-media {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 8px;
  }
  .thumb {
    padding: 0;
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-sm);
    background: none;
    cursor: zoom-in;
    overflow: hidden;
  }
  .thumb img {
    display: block;
    height: 120px;
    max-width: 240px;
    object-fit: cover;
  }
  .file {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 9px;
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-sm);
    background: var(--wp-card);
    color: var(--wp-text-muted);
    font-size: 12px;
    cursor: pointer;
  }
  .edit {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 6px;
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-sm);
    background: var(--wp-canvas);
  }
  .e-sum {
    border: none;
    border-bottom: 1px solid var(--wp-border);
    background: none;
    padding: 7px 10px;
    font-family: var(--wp-font-display);
    font-weight: 600;
    font-size: 13.5px;
    color: var(--wp-text);
    outline: none;
  }
  .e-body {
    border: none;
    background: none;
    resize: none;
    padding: 6px 10px;
    min-height: 70px;
    font-size: 13px;
    line-height: 1.5;
    color: var(--wp-text);
    outline: none;
  }
  .e-bar {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 4px 6px 6px;
  }
  .ghost,
  .prim {
    height: 26px;
    padding: 0 10px;
    border-radius: var(--wp-r-sm);
    font-size: 12px;
    cursor: pointer;
  }
  .ghost {
    border: 1px solid var(--wp-border);
    background: var(--wp-card);
    color: var(--wp-text-muted);
  }
  .prim {
    border: none;
    background: var(--wp-accent);
    color: var(--wp-on-accent);
  }
  .prim:disabled {
    opacity: 0.5;
  }
  /* A rail down each nesting level connects replies to the post they answer. */
  .children {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-left: 13px;
    padding-left: 18px;
    border-left: 2px solid var(--wp-border);
  }
  /* Deep chains stop indenting so they never get squeezed. */
  .children.flat {
    margin-left: 0;
    padding-left: 0;
    border-left: none;
  }
</style>

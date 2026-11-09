<script lang="ts">
  // Writing a forum post: the one-sentence summary (required - it heads the
  // post) and the Markdown body with its formatting bar, `@` references, and
  // media you paste, drop or pick (attached to the post).
  import { Paperclip, Send, X } from 'lucide-svelte';
  import { get } from 'svelte/store';
  import MdToolbar from './MdToolbar.svelte';
  import { mdkeys, mdlists } from '$lib/md';
  import { mention } from '$lib/mention';
  import { api, mediaUrl } from '$lib/api';
  import { currentProject, definitions } from '$lib/stores/board';
  import { filesFromClipboard, labelColor, mediaKind, autosize } from '$lib/utils';
  import type { Attachment } from '$lib/types';

  let {
    summary = $bindable(''),
    body = $bindable(''),
    labels = $bindable<string[]>([]),
    withLabels = false,
    submitLabel = 'Post',
    summaryPlaceholder = 'Summary - the point of this post in one sentence',
    bodyPlaceholder = 'Write… (Markdown, @ to reference a post, ticket or person)',
    compact = false,
    onsubmit,
    oncancel,
    bodyEl = $bindable<HTMLTextAreaElement | null>(null)
  }: {
    summary?: string;
    body?: string;
    labels?: string[];
    withLabels?: boolean;
    submitLabel?: string;
    summaryPlaceholder?: string;
    bodyPlaceholder?: string;
    compact?: boolean;
    onsubmit: (p: { summary: string; body: string; labels: string[]; attachments: Attachment[] }) => Promise<void>;
    oncancel?: () => void;
    bodyEl?: HTMLTextAreaElement | null;
  } = $props();

  let media = $state<Attachment[]>([]);
  let uploading = $state(0);
  let sending = $state(false);
  let error = $state<string | null>(null);
  let summaryEl = $state<HTMLInputElement | null>(null);
  let fileInput = $state<HTMLInputElement | null>(null);

  let ready = $derived(!!summary.trim() && !sending && uploading === 0);

  function proj() {
    return get(currentProject) ?? undefined;
  }

  async function stage(files: File[]) {
    error = null;
    for (const f of files) {
      uploading++;
      try {
        const a = await api.stageMedia(f, proj());
        if (!media.some((m) => m.path === a.path)) media = [...media, a];
      } catch (e) {
        error = e instanceof Error ? e.message : String(e);
      } finally {
        uploading--;
      }
    }
  }

  function onPaste(e: ClipboardEvent) {
    const files = filesFromClipboard(e.clipboardData);
    if (!files.length) return;
    e.preventDefault();
    void stage(files);
  }
  function onDrop(e: DragEvent) {
    const files = filesFromClipboard(e.dataTransfer);
    if (!files.length) return;
    e.preventDefault();
    void stage(files);
  }

  async function submit() {
    if (!ready) {
      if (!summary.trim()) {
        error = 'Add a summary: one sentence stating the point of the post.';
        summaryEl?.focus();
      }
      return;
    }
    sending = true;
    error = null;
    try {
      await onsubmit({ summary: summary.trim(), body: body.trim(), labels, attachments: media });
      summary = '';
      body = '';
      labels = [];
      media = [];
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      sending = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void submit();
    }
  }
  function toggleLabel(name: string) {
    labels = labels.includes(name) ? labels.filter((l) => l !== name) : [...labels, name];
  }
</script>

<div class="pc" class:compact ondrop={onDrop} ondragover={(e) => e.preventDefault()} role="group">
  <input
    class="summary"
    bind:this={summaryEl}
    bind:value={summary}
    placeholder={summaryPlaceholder}
    use:mention
    use:mdkeys
    onkeydown={(e) => {
      onKey(e);
      // Enter moves on to the body.
      if (e.key === 'Enter' && !e.ctrlKey && !e.metaKey && !e.defaultPrevented) {
        e.preventDefault();
        bodyEl?.focus();
      }
    }}
  />
  <textarea
    class="body"
    rows={compact ? 2 : 4}
    bind:this={bodyEl}
    bind:value={body}
    placeholder={bodyPlaceholder}
    use:autosize={body}
    use:mention
    use:mdkeys
    use:mdlists
    onpaste={onPaste}
    onkeydown={onKey}
  ></textarea>
  {#if media.length || uploading}
    <div class="media">
      {#each media as m (m.path)}
        <span class="att">
          {#if mediaKind(m.mime, m.name) === 'image'}
            <img src={mediaUrl(m.path, proj())} alt={m.name} />
          {:else}
            <Paperclip size={12} /><span class="aname">{m.name}</span>
          {/if}
          <button
            type="button"
            class="arm"
            aria-label="Remove {m.name}"
            onclick={() => (media = media.filter((x) => x !== m))}><X size={11} /></button
          >
        </span>
      {/each}
      {#if uploading}<span class="up">uploading…</span>{/if}
    </div>
  {/if}
  {#if withLabels && $definitions.labels.length}
    <div class="pool">
      {#each $definitions.labels as label (label.name)}
        <button
          type="button"
          class="pchip"
          class:on={labels.includes(label.name)}
          style="--c:{labelColor(label.name, label.color)}"
          onclick={() => toggleLabel(label.name)}>{label.name}</button
        >
      {/each}
    </div>
  {/if}
  <div class="bar">
    <MdToolbar target={bodyEl} />
    <button
      type="button"
      class="tool"
      title="Attach files (or paste / drop them)"
      aria-label="Attach files"
      onclick={() => fileInput?.click()}><Paperclip size={14} /></button
    >
    <input
      type="file"
      multiple
      hidden
      bind:this={fileInput}
      onchange={(e) => {
        const files = Array.from(e.currentTarget.files ?? []);
        e.currentTarget.value = '';
        void stage(files);
      }}
    />
    <span class="grow"></span>
    {#if oncancel}<button type="button" class="ghost" onclick={oncancel}>Cancel</button>{/if}
    <button type="button" class="send" disabled={!ready} title="{submitLabel} (Ctrl+Enter)" onclick={submit}>
      <Send size={13} />{sending ? 'Posting…' : submitLabel}
    </button>
  </div>
  {#if error}<div class="err">{error}</div>{/if}
</div>

<style>
  .pc {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-md);
    background: var(--wp-card);
    overflow: hidden;
    transition: border-color var(--wp-fast) var(--wp-ease);
  }
  .pc:focus-within {
    border-color: var(--wp-border-strong);
  }
  .summary {
    border: none;
    border-bottom: 1px solid var(--wp-border);
    background: none;
    padding: 9px 12px 8px;
    font-family: var(--wp-font-display);
    font-size: 14px;
    font-weight: 600;
    color: var(--wp-text);
    outline: none;
  }
  .summary::placeholder {
    font-weight: 500;
    color: var(--wp-text-subtle);
  }
  .body {
    border: none;
    background: none;
    resize: none;
    padding: 8px 12px;
    min-height: 64px;
    font-size: 13px;
    line-height: 1.55;
    color: var(--wp-text);
    outline: none;
  }
  .compact .body {
    min-height: 44px;
  }
  .media {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 0 12px 8px;
  }
  .att {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 180px;
    padding: 3px 22px 3px 6px;
    border: 1px solid var(--wp-border);
    border-radius: var(--wp-r-sm);
    background: var(--wp-surface);
    font-size: 11.5px;
    color: var(--wp-text-muted);
  }
  .att img {
    display: block;
    height: 46px;
    max-width: 120px;
    object-fit: cover;
    border-radius: 4px;
  }
  .aname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .arm {
    position: absolute;
    top: 2px;
    right: 2px;
    display: inline-flex;
    padding: 2px;
    border: none;
    border-radius: 4px;
    background: none;
    color: var(--wp-text-subtle);
    cursor: pointer;
  }
  .arm:hover {
    color: var(--wp-error);
  }
  .up {
    font-size: 11px;
    color: var(--wp-text-subtle);
    align-self: center;
  }
  .pool {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    padding: 0 12px 8px;
  }
  .pchip {
    display: inline-flex;
    align-items: center;
    height: 21px;
    padding: 0 8px;
    border-radius: var(--wp-r-pill);
    border: 1px solid color-mix(in srgb, var(--c) 55%, transparent);
    background: none;
    color: var(--wp-text-muted);
    font-size: 11px;
    cursor: pointer;
  }
  .pchip::before {
    content: '';
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--c);
    margin-right: 5px;
  }
  .pchip.on {
    background: color-mix(in srgb, var(--c) 16%, transparent);
    border-color: var(--c);
    color: var(--wp-text);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 6px 5px 4px;
    border-top: 1px solid var(--wp-border);
    background: var(--wp-surface);
    flex-wrap: wrap;
  }
  .grow {
    flex: 1;
  }
  .tool {
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
  }
  .tool:hover {
    color: var(--wp-text);
    background: var(--wp-elevated);
  }
  .ghost,
  .send {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 11px;
    border-radius: var(--wp-r-sm);
    font-size: 12px;
    cursor: pointer;
  }
  .ghost {
    border: 1px solid var(--wp-border);
    background: var(--wp-card);
    color: var(--wp-text-muted);
  }
  .send {
    border: none;
    background: var(--wp-accent);
    color: var(--wp-on-accent);
    font-weight: 500;
  }
  .send:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .err {
    padding: 6px 12px 8px;
    font-size: 12px;
    color: var(--wp-error);
  }
</style>

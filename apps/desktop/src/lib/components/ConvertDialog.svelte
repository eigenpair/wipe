<script lang="ts">
  // Confirming the one-time board conversion (old ticket ids / thread forum ->
  // current formats). It is a breaking change for everyone on the board, so it
  // says so plainly and needs an explicit acknowledgement.
  import { fade, scale } from 'svelte/transition';
  import { TriangleAlert, X } from 'lucide-svelte';

  let {
    open = $bindable(false),
    legacyIds,
    legacyForum,
    busy = false,
    onconfirm
  }: {
    open?: boolean;
    legacyIds: boolean;
    legacyForum: boolean;
    busy?: boolean;
    onconfirm: () => void;
  } = $props();

  let understood = $state(false);
  $effect(() => {
    if (open) understood = false;
  });

  function onKey(e: KeyboardEvent) {
    if (open && e.key === 'Escape') {
      e.preventDefault();
      open = false;
    }
  }
</script>

<svelte:window onkeydown={onKey} />

{#if open}
  <div class="scrim" transition:fade={{ duration: 120 }} role="presentation" onclick={() => (open = false)}></div>
  <div class="wrap" role="dialog" aria-modal="true" aria-labelledby="cv-title">
    <div class="dlg" transition:scale={{ start: 0.97, duration: 140, opacity: 0 }}>
      <button class="x" aria-label="Close" onclick={() => (open = false)}><X size={16} /></button>
      <h2 id="cv-title"><TriangleAlert size={18} /> Convert this board?</h2>
      <p>This converts the board's files to the current formats:</p>
      <ul>
        {#if legacyIds}<li>Ticket ids become fixed-width hex: <code>T-23</code> → <code>T-017</code>.</li>{/if}
        {#if legacyForum}
          <li>
            Every forum post gets its own id (<code>F-3.1</code> → <code>F-00A</code>) and a summary line;
            replies are linked by <code>@</code> tags.
          </li>
        {/if}
        <li>Files are renamed and every reference is rewritten. Old ids keep working as aliases.</li>
      </ul>
      <div class="warn">
        <b>Breaking change.</b> Everyone who uses this board must update wipe to <b>1.1 or newer</b> -
        older versions can't read the converted board. Commit or merge open work first, and have
        everyone update and sync right after.
      </div>
      <label class="ack">
        <input type="checkbox" bind:checked={understood} />
        I understand - everyone on this board will update wipe.
      </label>
      <div class="actions">
        <button class="ghost" onclick={() => (open = false)}>Cancel</button>
        <button
          class="danger"
          disabled={!understood || busy}
          onclick={() => {
            open = false;
            onconfirm();
          }}>{busy ? 'Converting…' : 'Convert board'}</button
        >
      </div>
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 150;
    background: rgba(8, 8, 8, 0.6);
    backdrop-filter: blur(5px);
  }
  .wrap {
    position: fixed;
    inset: 0;
    z-index: 151;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 16px;
    pointer-events: none;
  }
  .dlg {
    position: relative;
    pointer-events: auto;
    width: min(520px, 100%);
    padding: 20px 22px 18px;
    border-radius: var(--wp-r-lg);
    border: 1px solid var(--wp-border);
    background: var(--wp-card);
    box-shadow: var(--wp-shadow-lift);
    font-size: 13.5px;
    line-height: 1.55;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 17px;
    margin: 0 0 8px;
  }
  h2 :global(svg) {
    color: var(--wp-d-edited);
  }
  p {
    margin: 0 0 4px;
    color: var(--wp-text-muted);
  }
  ul {
    margin: 0 0 12px;
    padding-left: 1.2em;
  }
  li {
    margin: 3px 0;
  }
  code {
    font-family: var(--wp-font-mono);
    font-size: 12px;
    padding: 0 4px;
    border-radius: 4px;
    background: var(--wp-surface);
  }
  .warn {
    padding: 10px 12px;
    border-radius: var(--wp-r-md);
    border: 1px solid color-mix(in srgb, var(--wp-d-edited) 45%, transparent);
    background: color-mix(in srgb, var(--wp-d-edited) 10%, transparent);
  }
  .ack {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 14px 0 4px;
    font-size: 12.5px;
    color: var(--wp-text-muted);
    cursor: pointer;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 12px;
  }
  .ghost,
  .danger {
    height: 32px;
    padding: 0 14px;
    border-radius: var(--wp-r-sm);
    font-size: 13px;
    cursor: pointer;
  }
  .ghost {
    border: 1px solid var(--wp-border);
    background: var(--wp-card);
    color: var(--wp-text-muted);
  }
  .danger {
    border: none;
    background: var(--wp-accent);
    color: var(--wp-on-accent);
    font-weight: 500;
  }
  .danger:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .x {
    position: absolute;
    top: 12px;
    right: 12px;
    display: inline-flex;
    padding: 4px;
    border: none;
    border-radius: var(--wp-r-sm);
    background: none;
    color: var(--wp-text-subtle);
    cursor: pointer;
  }
</style>

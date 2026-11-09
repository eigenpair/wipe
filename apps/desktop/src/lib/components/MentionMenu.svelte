<script lang="ts">
  // The `@` suggestion menu at the caret (see lib/mention.ts): one instance for
  // the whole app, filled by whichever field is being typed in.
  import { fly } from 'svelte/transition';
  import Avatar from './Avatar.svelte';
  import { mentionMenu, pickMention } from '$lib/mention';
  import { identities } from '$lib/stores/board';
  import { plainInline } from '$lib/refs';

  let list = $state<HTMLDivElement | null>(null);
  // Keep the highlighted item in view while arrowing through.
  $effect(() => {
    const i = $mentionMenu?.index;
    if (i === undefined || !list) return;
    list.children[i]?.scrollIntoView({ block: 'nearest' });
  });
  let below = $derived(!$mentionMenu || $mentionMenu.y + 280 < window.innerHeight);
</script>

{#if $mentionMenu}
  {@const m = $mentionMenu}
  <div
    class="mm"
    bind:this={list}
    style="left:{m.x}px; {below ? `top:${m.y}px` : `bottom:${window.innerHeight - m.y + 26}px`}"
    transition:fly={{ y: below ? -4 : 4, duration: 110 }}
    role="listbox"
  >
    {#each m.items as s, i (s.kind + s.id)}
      <button
        type="button"
        class="it"
        class:on={i === m.index}
        role="option"
        aria-selected={i === m.index}
        onmousedown={(e) => {
          // Keep the field focused; pick on mousedown so blur doesn't close first.
          e.preventDefault();
          pickMention(s);
        }}
        onmouseenter={() => mentionMenu.set({ ...m, index: i })}
      >
        {#if s.kind === 'identity'}
          <Avatar id={s.id} identity={$identities.find((x) => x.id === s.id)} size={18} />
          <span class="lbl">{s.label}</span>
          <span class="sub">{s.hint ?? s.id}</span>
        {:else}
          <span class="kid" class:post={s.kind === 'post'}>{s.id}</span>
          <span class="lbl">{plainInline(s.label)}</span>
        {/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  .mm {
    position: fixed;
    z-index: 320;
    width: 300px;
    max-height: 272px;
    overflow-y: auto;
    padding: 4px;
    border-radius: var(--wp-r-md);
    border: 1px solid var(--wp-border-strong);
    background: var(--wp-elevated);
    box-shadow: var(--wp-shadow-lift);
    scrollbar-width: none;
  }
  .it {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 30px;
    padding: 4px 8px;
    border: none;
    border-radius: var(--wp-r-sm);
    background: none;
    color: var(--wp-text);
    font-size: 12.5px;
    text-align: left;
    cursor: pointer;
  }
  .it.on {
    background: color-mix(in srgb, var(--wp-accent) 14%, transparent);
  }
  .kid {
    flex: none;
    font-family: var(--wp-font-mono);
    font-size: 11px;
    color: var(--wp-accent);
  }
  .kid.post {
    color: var(--wp-focus);
  }
  .lbl {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    flex: none;
    font-size: 10.5px;
    color: var(--wp-text-subtle);
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>

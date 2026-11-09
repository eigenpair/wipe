<script lang="ts">
  import { fade } from 'svelte/transition';
  import { X, Download, ZoomIn, ZoomOut, Maximize } from 'lucide-svelte';
  import { lightbox, closeLightbox } from '$lib/stores/lightbox';

  // Images open fitted to the screen. The wheel zooms around the cursor, a click
  // zooms in on that spot (or back out), and a zoomed image pans by dragging.
  const MIN = 1;
  const MAX = 8;
  let scale = $state(1);
  let tx = $state(0);
  let ty = $state(0);
  // Animate clicks and buttons, not wheel steps or drags (those follow the hand).
  let smooth = $state(true);
  let dragging = $state(false);
  let stage = $state<HTMLDivElement | null>(null);
  let text = $state<string | null>(null);

  function reset() {
    scale = 1;
    tx = 0;
    ty = 0;
  }

  $effect(() => {
    const item = $lightbox;
    reset();
    text = null;
    if (item?.kind === 'text') {
      fetch(item.src)
        .then((r) => r.text())
        .then((t) => {
          if ($lightbox === item) text = t.length > 200_000 ? `${t.slice(0, 200_000)}\n…` : t;
        })
        .catch(() => (text = '(could not load the file)'));
    }
  });

  /** Zoom to `next`, keeping the screen point (x, y) where it is. */
  function zoomAt(next: number, x: number, y: number) {
    if (!stage) return;
    next = Math.min(MAX, Math.max(MIN, next));
    const r = stage.getBoundingClientRect();
    // The point relative to the stage centre, where the unzoomed image sits.
    const px = x - (r.left + r.width / 2);
    const py = y - (r.top + r.height / 2);
    const k = next / scale;
    tx = px - (px - tx) * k;
    ty = py - (py - ty) * k;
    scale = next;
    if (scale === MIN) {
      tx = 0;
      ty = 0;
    }
  }

  function zoomCentre(next: number) {
    if (!stage) return;
    const r = stage.getBoundingClientRect();
    smooth = true;
    zoomAt(next, r.left + r.width / 2, r.top + r.height / 2);
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    smooth = false;
    // Trackpads send small deltas, mice large ones; both feel even this way.
    const factor = Math.exp(-e.deltaY * (e.deltaMode === 1 ? 0.05 : 0.0018));
    zoomAt(scale * factor, e.clientX, e.clientY);
  }

  // Drag to pan; a press without movement is a click (zoom toggle).
  let start: { x: number; y: number; tx: number; ty: number; onImage: boolean } | null = null;
  let moved = false;
  function onDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const onImage = (e.target as HTMLElement).tagName === 'IMG';
    start = { x: e.clientX, y: e.clientY, tx, ty, onImage };
    moved = false;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (!start) return;
    const dx = e.clientX - start.x;
    const dy = e.clientY - start.y;
    if (!moved && Math.hypot(dx, dy) < 4) return;
    moved = true;
    if (scale > MIN) {
      dragging = true;
      smooth = false;
      tx = start.tx + dx;
      ty = start.ty + dy;
    }
  }
  function onUp(e: PointerEvent) {
    if (!start) return;
    const wasClick = !moved;
    const onImage = start.onImage;
    start = null;
    dragging = false;
    if (wasClick && !onImage && scale === MIN) {
      // Beside a fitted image: close, like the backdrop.
      closeLightbox();
    } else if (wasClick) {
      smooth = true;
      zoomAt(scale > MIN ? MIN : 2.5, e.clientX, e.clientY);
    }
  }

  function onKey(e: KeyboardEvent) {
    if (!$lightbox) return;
    if (e.key === 'Escape') {
      // Handled here: keep an underlying modal from also closing.
      e.preventDefault();
      e.stopPropagation();
      closeLightbox();
    } else if ($lightbox.kind === 'image' && (e.key === '+' || e.key === '=')) {
      zoomCentre(scale * 1.5);
    } else if ($lightbox.kind === 'image' && e.key === '-') {
      zoomCentre(scale / 1.5);
    } else if ($lightbox.kind === 'image' && e.key === '0') {
      smooth = true;
      reset();
    }
  }
</script>

<svelte:window onkeydowncapture={onKey} />

{#if $lightbox}
  {@const item = $lightbox}
  <div
    class="lb"
    transition:fade={{ duration: 120 }}
    role="dialog"
    aria-modal="true"
    aria-label={`Preview of ${item.name}`}
  >
    <button class="scrim" aria-label="Close preview" onclick={closeLightbox}></button>
    <header class="bar">
      <span class="name" title={item.name}>{item.name}</span>
      {#if item.kind === 'image'}
        <span class="pct">{Math.round(scale * 100)}%</span>
        <button class="ib" onclick={() => zoomCentre(scale / 1.5)} aria-label="Zoom out" title="Zoom out (-)"
          ><ZoomOut size={16} /></button
        >
        <button class="ib" onclick={() => zoomCentre(scale * 1.5)} aria-label="Zoom in" title="Zoom in (+)"
          ><ZoomIn size={16} /></button
        >
        <button
          class="ib"
          onclick={() => {
            smooth = true;
            reset();
          }}
          aria-label="Fit to screen"
          title="Fit (0)"><Maximize size={15} /></button
        >
      {/if}
      <a class="ib" href={item.src} download={item.name} aria-label="Download" title="Download"
        ><Download size={16} /></a
      >
      <button class="ib" onclick={closeLightbox} aria-label="Close" title="Close"
        ><X size={18} /></button
      >
    </header>
    <div class="stage" bind:this={stage}>
      {#if item.kind === 'image'}
        <div
          class="viewport"
          class:zoomed={scale > MIN}
          class:dragging
          role="img"
          aria-label={item.name}
          onwheel={onWheel}
          onpointerdown={onDown}
          onpointermove={onMove}
          onpointerup={onUp}
          onpointercancel={() => {
            start = null;
            dragging = false;
          }}
        >
          <img
            src={item.src}
            alt={item.name}
            draggable="false"
            class:smooth
            style="transform: translate({tx}px, {ty}px) scale({scale})"
          />
        </div>
      {:else if item.kind === 'video'}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video src={item.src} controls autoplay playsinline></video>
      {:else if item.kind === 'audio'}
        <audio src={item.src} controls autoplay></audio>
      {:else if item.kind === 'pdf'}
        <iframe src={item.src} title={item.name}></iframe>
      {:else if item.kind === 'text'}
        <pre class="wp-scroll">{text ?? 'Loading…'}</pre>
      {:else}
        <p class="none">No preview for this file type - use Download.</p>
      {/if}
    </div>
  </div>
{/if}

<style>
  .lb {
    position: fixed;
    inset: 0;
    z-index: 200;
    display: flex;
    flex-direction: column;
  }
  .scrim {
    position: absolute;
    inset: 0;
    border: none;
    background: rgba(0, 0, 0, 0.85);
    cursor: zoom-out;
  }
  .bar {
    position: relative;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    padding-top: max(10px, env(safe-area-inset-top));
    color: #fff;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }
  .pct {
    font-family: var(--wp-font-mono);
    font-size: 11.5px;
    color: rgba(255, 255, 255, 0.7);
    min-width: 42px;
    text-align: right;
  }
  .ib {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: var(--wp-r-sm);
    border: 1px solid rgba(255, 255, 255, 0.2);
    background: rgba(255, 255, 255, 0.08);
    color: #fff;
    cursor: pointer;
  }
  .ib:hover {
    background: rgba(255, 255, 255, 0.16);
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 12px 16px;
    pointer-events: none;
  }
  .stage > :global(*) {
    pointer-events: auto;
  }
  /* The image area: fitted image, zoomed and panned by transform only (no
     layout shifts, so it never jumps into a corner). */
  .viewport {
    position: absolute;
    inset: 0 12px 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    touch-action: none;
    user-select: none;
  }
  .viewport:not(.zoomed) {
    cursor: zoom-out;
  }
  .viewport:not(.zoomed) img {
    cursor: zoom-in;
  }
  .viewport.zoomed {
    cursor: grab;
  }
  .viewport.dragging {
    cursor: grabbing;
  }
  .viewport img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: var(--wp-r-sm);
    transform-origin: center center;
    will-change: transform;
  }
  .viewport img.smooth {
    transition: transform 220ms cubic-bezier(0.2, 0.7, 0.2, 1);
  }
  @media (prefers-reduced-motion: reduce) {
    .viewport img.smooth {
      transition: none;
    }
  }
  video {
    max-width: 100%;
    max-height: calc(100dvh - 90px);
  }
  iframe {
    width: min(900px, 100%);
    height: 100%;
    border: none;
    background: #fff;
    border-radius: var(--wp-r-sm);
  }
  pre {
    width: min(900px, 100%);
    max-height: 100%;
    overflow: auto;
    margin: 0;
    padding: 14px;
    border-radius: var(--wp-r-sm);
    background: var(--wp-card);
    color: var(--wp-text);
    font-family: var(--wp-font-mono);
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .none {
    color: #ddd;
    font-size: 14px;
  }
</style>

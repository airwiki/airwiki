<script lang="ts">
  import type { Snippet } from 'svelte';

  export let sidebar: Snippet;
  export let children: Snippet;
  export let collapsed = false;
  export let width = 224;
  export let resizeLabel: string;
  export let oncollapse: () => void = () => {};

  let viewportWidth = 1024;
  let drag: { x: number; width: number; pointerId: number } | null = null;
  $: maximum = Math.min(360, Math.max(200, Math.floor(viewportWidth * 0.4)));
  $: renderedWidth = Math.max(200, Math.min(maximum, width));

  function resize(value: number) {
    width = Math.max(200, Math.min(maximum, value));
  }

  function beginResize(event: PointerEvent) {
    if (event.button !== 0) return;
    const separator = event.currentTarget as HTMLElement;
    separator.setPointerCapture(event.pointerId);
    separator.focus({ preventScroll: true });
    drag = { x: event.clientX, width: renderedWidth, pointerId: event.pointerId };
    event.preventDefault();
  }

  function moveResize(event: PointerEvent) {
    if (drag?.pointerId === event.pointerId) resize(drag.width + event.clientX - drag.x);
  }

  function keyboardResize(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault();
      oncollapse();
      return;
    }
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    resize(event.key === 'Home' ? 200 : event.key === 'End' ? maximum
      : renderedWidth + (event.key === 'ArrowRight' ? 16 : -16));
  }
</script>

<svelte:window bind:innerWidth={viewportWidth} />

<div class="workspace-frame" class:collapsed style:--sidebar-width={`${renderedWidth}px`}>
  <div class="workspace-navigation" id="workspace-navigation" hidden={collapsed}>
      {@render sidebar()}
      <!-- The ARIA window splitter pattern defines a focusable separator with arrow keys. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions -->
      <div
        class="sidebar-resizer"
        class:dragging={drag !== null}
        role="separator"
        tabindex="0"
        aria-label={resizeLabel}
        aria-controls="workspace-navigation"
        aria-orientation="vertical"
        aria-valuemin={200}
        aria-valuemax={maximum}
        aria-valuenow={renderedWidth}
        onpointerdown={beginResize}
        onpointermove={moveResize}
        onpointerup={() => { drag = null; }}
        onpointercancel={() => { drag = null; }}
        onlostpointercapture={() => { drag = null; }}
        onkeydown={keyboardResize}
      ></div>
  </div>
  {@render children()}
</div>

<style>
  .workspace-frame { display: grid; grid-template-columns: var(--sidebar-width) minmax(0, 1fr); min-width: 0; min-height: 0; overflow: hidden; }
  .workspace-frame.collapsed { grid-template-columns: minmax(0, 1fr); }
  .workspace-navigation { position: relative; min-width: 0; min-height: 0; background: var(--rail); border-right: 1px solid var(--line); }
  .workspace-navigation[hidden] { display: none; }
  .sidebar-resizer { position: absolute; z-index: 6; top: 0; right: -3px; bottom: 0; width: 6px; cursor: col-resize; touch-action: none; }
  .sidebar-resizer:hover, .sidebar-resizer.dragging { background: color-mix(in srgb, var(--cyan) 35%, transparent); }
  .sidebar-resizer:focus-visible { outline: 2px solid var(--cyan); outline-offset: -2px; }
  @media (forced-colors: active) { .sidebar-resizer:focus-visible { outline-color: Highlight; } }
</style>

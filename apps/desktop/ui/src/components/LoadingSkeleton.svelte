<script lang="ts">
  export let variant: 'results' | 'workspace' | 'page';
  export let rows = 3;

  $: rowIndexes = Array.from({ length: Math.max(1, rows) }, (_, index) => index);
</script>

<div class={`loading-skeleton ${variant}`} aria-hidden="true">
  {#if variant === 'results'}
    {#each rowIndexes as row (row)}
      <div class="result-row">
        <span class="skeleton-line meta"></span>
        <span class="skeleton-line title"></span>
        <span class="skeleton-line copy"></span>
        <span class="skeleton-line copy short"></span>
      </div>
    {/each}
  {:else if variant === 'workspace'}
    <aside class="workspace-list">
      {#each rowIndexes as row (row)}
        <div class="workspace-row">
          <span class="skeleton-block icon"></span>
          <span><i class="skeleton-line item-title"></i><i class="skeleton-line item-path"></i></span>
        </div>
      {/each}
    </aside>
    <section class="workspace-page">
      <span class="skeleton-line eyebrow"></span>
      <span class="skeleton-line heading"></span>
      <div class="assurance-grid"><i></i><i></i><i></i><i></i></div>
      <span class="skeleton-line paragraph"></span>
      <span class="skeleton-line paragraph"></span>
      <span class="skeleton-line paragraph short"></span>
    </section>
  {:else}
    <section class="workspace-page page-only">
      <span class="skeleton-line eyebrow"></span>
      <span class="skeleton-line heading"></span>
      <div class="assurance-grid"><i></i><i></i><i></i><i></i></div>
      <span class="skeleton-line paragraph"></span>
      <span class="skeleton-line paragraph"></span>
      <span class="skeleton-line paragraph short"></span>
    </section>
  {/if}
</div>

<style>
  .loading-skeleton {
    min-width: 0;
  }

  .skeleton-line,
  .skeleton-block,
  .assurance-grid i {
    position: relative;
    display: block;
    overflow: hidden;
    background: color-mix(in srgb, var(--surface-raised) 82%, var(--line));
    border-radius: 999px;
  }

  .skeleton-line::after,
  .skeleton-block::after,
  .assurance-grid i::after {
    content: '';
    position: absolute;
    inset: 0 auto 0 -42%;
    width: 34%;
    background: color-mix(in srgb, var(--strong) 10%, transparent);
    box-shadow: 0 0 22px color-mix(in srgb, var(--strong) 9%, transparent);
    animation: skeleton-sweep 1.65s ease-in-out infinite;
  }

  .results {
    display: grid;
    border-top: 1px solid var(--line);
  }

  .result-row {
    display: grid;
    gap: 10px;
    min-height: 148px;
    padding: 22px 16px;
    border-bottom: 1px solid var(--line);
  }

  .result-row:nth-child(2) { opacity: .82; }
  .result-row:nth-child(3) { opacity: .64; }
  .result-row .meta { width: 29%; height: 10px; }
  .result-row .title { width: 52%; height: 18px; }
  .result-row .copy { width: 92%; height: 11px; }
  .result-row .copy.short { width: 68%; }

  .workspace {
    display: grid;
    grid-template-columns: minmax(250px, 34%) minmax(0, 1fr);
    min-height: 360px;
    background: var(--slate);
    border: 1px solid var(--line);
    border-radius: var(--panel-radius);
    overflow: hidden;
  }

  .workspace-list {
    display: grid;
    align-content: start;
    gap: 3px;
    padding: 12px;
    border-right: 1px solid var(--line);
  }

  .workspace-row {
    display: grid;
    grid-template-columns: 28px minmax(0, 1fr);
    align-items: center;
    gap: 10px;
    min-height: 54px;
    padding: 7px;
  }

  .workspace-row > span:last-child { display: grid; gap: 7px; }
  .workspace-row:nth-child(2) { opacity: .86; }
  .workspace-row:nth-child(3) { opacity: .72; }
  .workspace-row:nth-child(n + 4) { opacity: .58; }
  .icon { width: 28px; height: 28px; border-radius: 7px; }
  .item-title { width: 72%; height: 11px; }
  .item-path { width: 90%; height: 8px; }

  .workspace-page {
    display: grid;
    align-content: start;
    gap: 14px;
    min-width: 0;
    padding: 34px 38px;
  }

  .workspace-page.page-only { padding: 8px 0; }
  .eyebrow { width: 20%; height: 9px; }
  .heading { width: min(420px, 72%); height: 25px; }
  .paragraph { width: 96%; height: 11px; }
  .paragraph.short { width: 66%; }
  .assurance-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px 18px; margin: 7px 0 10px; padding: 15px; border: 1px solid var(--line); border-radius: var(--control-radius); }
  .assurance-grid i { height: 24px; border-radius: 5px; }

  @keyframes skeleton-sweep {
    0% { transform: translateX(0); }
    60%, 100% { transform: translateX(430%); }
  }

  @media (prefers-reduced-motion: reduce) {
    .skeleton-line::after,
    .skeleton-block::after,
    .assurance-grid i::after { animation: none; }
  }
</style>

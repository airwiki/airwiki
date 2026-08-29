<script lang="ts">
  import Spinner from './Spinner.svelte';
  import ShimmerText from './ShimmerText.svelte';

  export let label: string;
  export let detail: string | null = null;
  export let compact = false;
  export let tone: 'neutral' | 'ai' = 'neutral';
</script>

<div class:compact class:ai={tone === 'ai'} class="loading-state" role="status" aria-live="polite">
  <Spinner size={compact ? 'small' : 'medium'} />
  <span class="loading-copy">
    <strong><ShimmerText text={label} {tone} /></strong>
    {#if detail}<small>{detail}</small>{/if}
  </span>
</div>

<style>
  .loading-state {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 72px;
    color: var(--muted);
  }

  .loading-state.compact {
    gap: 8px;
    min-height: 32px;
    font-size: 13px;
  }

  .loading-copy {
    display: grid;
    gap: 2px;
    min-width: 0;
  }

  .loading-copy strong {
    color: var(--strong);
    font-weight: 620;
  }

  .loading-state.ai :global(.spinner) {
    border-color: color-mix(in srgb, var(--violet) 22%, var(--line));
    border-top-color: var(--violet);
  }

  .loading-copy small {
    color: var(--muted);
    line-height: 1.35;
  }

</style>

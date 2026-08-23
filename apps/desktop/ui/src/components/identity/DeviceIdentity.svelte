<script lang="ts">
  import Globe2 from '@lucide/svelte/icons/globe-2';
  import type { HostPlatform } from '../../api';
  import PlatformIcon from './PlatformIcon.svelte';

  export let name: string;
  export let platform: HostPlatform | null = null;
  export let platformLabel: string;
  export let detail: string | null = null;
  export let source: 'device' | 'public' = 'device';
  export let compact = false;
  export let chip = false;
</script>

<div class:compact class:chip class:public={source === 'public'} class="device-identity">
  {#if source === 'public'}
    <span class="public-icon" role="img" aria-label={platformLabel}>
      <Globe2 size={compact ? 13 : 17} strokeWidth={1.9} aria-hidden="true" />
    </span>
  {:else}
    <PlatformIcon {platform} label={platformLabel} size={compact ? 24 : 34} />
  {/if}
  <span class="device-copy">
    <strong>{name}</strong>
    {#if detail}<small>{detail}</small>{/if}
  </span>
</div>

<style>
  .device-identity { display: inline-flex; align-items: center; gap: 10px; min-width: 0; }
  .device-copy { display: grid; gap: 2px; min-width: 0; }
  .device-copy strong, .device-copy small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .device-copy strong { color: var(--strong); font: 600 13px system-ui, sans-serif; }
  .device-copy small { color: var(--muted); font-size: 11.5px; }
  .public-icon { display: inline-grid; flex: 0 0 auto; place-items: center; width: 34px; height: 34px; color: var(--public-accent, var(--violet)); background: color-mix(in srgb, var(--public-accent, var(--violet)) 9%, var(--surface-raised)); border: 1px solid color-mix(in srgb, var(--public-accent, var(--violet)) 24%, var(--line)); border-radius: 50%; }
  .compact { gap: 7px; }
  .compact .public-icon { width: 24px; height: 24px; }
  .compact .device-copy strong { font-size: 12px; }
  .chip { max-width: 240px; padding: 3px 8px 3px 4px; background: var(--surface-raised); border: 1px solid var(--line); border-radius: 999px; }
  .chip .device-copy strong { font-size: 11px; font-weight: 570; }
</style>

<script lang="ts">
  import Command from '@lucide/svelte/icons/command';
  import Monitor from '@lucide/svelte/icons/monitor';
  import type { HostPlatform } from '../../api';

  export let platform: HostPlatform | null;
  export let label: string;
  export let size = 18;
  export let decorative = false;
</script>

<span
  class:macos={platform === 'macOs'}
  class:windows={platform === 'windows'}
  class:unknown={platform === null}
  class="platform-icon"
  role={decorative ? undefined : 'img'}
  aria-label={decorative ? undefined : label}
  aria-hidden={decorative ? 'true' : undefined}
  style={`--platform-icon-size: ${size}px`}
>
  {#if platform === 'macOs'}
    <Command size={Math.round(size * 0.62)} strokeWidth={2} aria-hidden="true" />
  {:else if platform === 'windows'}
    <svg viewBox="0 0 24 24" width={Math.round(size * 0.62)} height={Math.round(size * 0.62)} aria-hidden="true">
      <path d="M3 4.8 10.5 3.7v7.45H3V4.8Zm8.55-1.25L21 2.2v8.95h-9.45v-7.6ZM3 12.25h7.5v7.45L3 18.6v-6.35Zm8.55 0H21v8.95l-9.45-1.35v-7.6Z" fill="currentColor" />
    </svg>
  {:else}
    <Monitor size={Math.round(size * 0.62)} strokeWidth={1.9} aria-hidden="true" />
  {/if}
</span>

<style>
  .platform-icon {
    display: inline-grid;
    flex: 0 0 auto;
    place-items: center;
    width: var(--platform-icon-size);
    height: var(--platform-icon-size);
    color: var(--strong);
    background: color-mix(in srgb, var(--strong) 5%, var(--surface-raised));
    border: 1px solid color-mix(in srgb, var(--line) 82%, var(--strong));
    border-radius: calc(var(--control-radius) - 1px);
    box-shadow: inset 0 1px 0 color-mix(in srgb, white 8%, transparent);
  }

  .platform-icon.macos { border-radius: 30%; }
  .platform-icon.windows { border-radius: max(3px, calc(var(--control-radius) - 3px)); }
  .platform-icon.unknown { color: var(--muted); }
</style>

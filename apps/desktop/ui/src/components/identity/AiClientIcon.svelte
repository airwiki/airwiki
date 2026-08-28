<script lang="ts">
  import Blocks from '@lucide/svelte/icons/blocks';
  import chatGptIcon from '../../assets/brands/chatgpt.png';
  import claudeCodeIcon from '../../assets/brands/claude-code.svg';
  import claudeDesktopIcon from '../../assets/brands/claude-desktop.svg';
  import codexIcon from '../../assets/brands/codex.png';
  import geminiCliIcon from '../../assets/brands/gemini-cli.png';
  import type { IntegrationClient } from '../../api';

  export let client: IntegrationClient | 'codex';
  export let label: string;
  export let size = 34;
  export let decorative = false;

  $: glyphSize = Math.round(size * 0.5);

  function brandAssetFor(value: IntegrationClient | 'codex'): string | null {
    switch (value) {
      case 'chatGptDesktop': return chatGptIcon;
      case 'codex': return codexIcon;
      case 'claudeDesktop': return claudeDesktopIcon;
      case 'claudeCode': return claudeCodeIcon;
      case 'geminiCli': return geminiCliIcon;
      case 'genericMcp': return null;
    }
  }

  $: brandAsset = brandAssetFor(client);
</script>

<span
  class={`ai-client-icon client-${client}`}
  role={decorative ? undefined : 'img'}
  aria-label={decorative ? undefined : label}
  aria-hidden={decorative ? 'true' : undefined}
  style={`--ai-client-size: ${size}px`}
>
  {#if brandAsset}
    <img class="brand-image" src={brandAsset} alt="" draggable="false" />
  {:else}
    <Blocks size={glyphSize} strokeWidth={1.9} aria-hidden="true" />
  {/if}
</span>

<style>
  .ai-client-icon {
    display: inline-grid;
    flex: 0 0 auto;
    place-items: center;
    width: var(--ai-client-size);
    height: var(--ai-client-size);
    color: var(--cyan);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--control-radius);
    overflow: hidden;
  }

  .brand-image {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
    user-select: none;
  }

  .client-claudeCode {
    background: color-mix(in srgb, #d97757 5%, var(--surface-raised));
    border-color: color-mix(in srgb, #d97757 18%, var(--line));
  }

  .client-claudeCode .brand-image {
    width: 66%;
    height: 66%;
  }

  .client-genericMcp {
    color: var(--cyan);
    background: color-mix(in srgb, var(--cyan) 8%, var(--surface-raised));
    border-color: color-mix(in srgb, var(--cyan) 22%, var(--line));
    box-shadow: inset 0 1px 0 color-mix(in srgb, white 8%, transparent);
  }
</style>

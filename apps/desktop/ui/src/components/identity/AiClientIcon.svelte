<script lang="ts">
  import Blocks from '@lucide/svelte/icons/blocks';
  import Bot from '@lucide/svelte/icons/bot';
  import CodeXml from '@lucide/svelte/icons/code-xml';
  import MessageCircle from '@lucide/svelte/icons/message-circle';
  import Sparkles from '@lucide/svelte/icons/sparkles';
  import SquareTerminal from '@lucide/svelte/icons/square-terminal';
  import type { IntegrationClient } from '../../api';

  export let client: IntegrationClient | 'codex';
  export let label: string;
  export let size = 34;
  export let decorative = false;

  $: glyphSize = Math.round(size * 0.5);
</script>

<span
  class={`ai-client-icon client-${client}`}
  role={decorative ? undefined : 'img'}
  aria-label={decorative ? undefined : label}
  aria-hidden={decorative ? 'true' : undefined}
  style={`--ai-client-size: ${size}px`}
>
  {#if client === 'chatGptDesktop'}
    <Bot size={glyphSize} strokeWidth={1.9} aria-hidden="true" />
  {:else if client === 'codex'}
    <CodeXml size={glyphSize} strokeWidth={1.9} aria-hidden="true" />
  {:else if client === 'claudeDesktop'}
    <MessageCircle size={glyphSize} strokeWidth={1.9} aria-hidden="true" />
  {:else if client === 'claudeCode'}
    <SquareTerminal size={glyphSize} strokeWidth={1.9} aria-hidden="true" />
  {:else if client === 'geminiCli'}
    <Sparkles size={glyphSize} strokeWidth={1.9} aria-hidden="true" />
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
    color: var(--violet);
    background: color-mix(in srgb, var(--violet) 9%, var(--surface-raised));
    border: 1px solid color-mix(in srgb, var(--violet) 24%, var(--line));
    border-radius: calc(var(--control-radius) + 1px);
    box-shadow: inset 0 1px 0 color-mix(in srgb, white 8%, transparent);
  }

  .client-genericMcp { color: var(--cyan); background: color-mix(in srgb, var(--cyan) 8%, var(--surface-raised)); border-color: color-mix(in srgb, var(--cyan) 22%, var(--line)); }
</style>

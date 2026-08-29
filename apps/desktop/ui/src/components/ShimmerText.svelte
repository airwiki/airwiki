<script lang="ts">
  export let text: string;
  export let active = true;
  export let tone: 'neutral' | 'ai' = 'ai';
</script>

<span class:active class:neutral={tone === 'neutral'} class:ai={tone === 'ai'} class="shimmer-text">{text}</span>

<style>
  .shimmer-text {
    color: inherit;
  }

  .shimmer-text.neutral {
    --shimmer-accent: var(--cyan);
  }

  .shimmer-text.ai {
    --shimmer-accent: var(--violet);
  }

  .shimmer-text.active {
    color: transparent;
    background-image: linear-gradient(
      100deg,
      color-mix(in srgb, var(--shimmer-accent, var(--violet)) 40%, var(--strong)) 15%,
      var(--strong) 46%,
      color-mix(in srgb, var(--shimmer-accent, var(--violet)) 40%, var(--strong)) 78%
    );
    background-position: 120% 50%;
    background-size: 240% 100%;
    background-clip: text;
    -webkit-background-clip: text;
    animation: shimmer-text 2.2s var(--ease-native) infinite;
  }

  @keyframes shimmer-text {
    to { background-position: -120% 50%; }
  }

  @media (prefers-reduced-motion: reduce), (forced-colors: active) {
    .shimmer-text.active {
      color: inherit;
      background: none;
      animation: none;
    }
  }
</style>

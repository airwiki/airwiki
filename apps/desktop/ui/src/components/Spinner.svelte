<script lang="ts">
  export let size: 'small' | 'medium' = 'medium';
</script>

<span class:small={size === 'small'} class="spinner" aria-hidden="true"></span>

<style>
  .spinner {
    box-sizing: border-box;
    display: inline-block;
    flex: 0 0 auto;
    width: 18px;
    height: 18px;
    border: 2px solid color-mix(in srgb, var(--cyan) 22%, var(--line));
    border-top-color: var(--cyan);
    border-radius: 50%;
    animation: spinner-rotate 720ms linear infinite;
  }

  .spinner.small {
    width: 14px;
    height: 14px;
  }

  @keyframes spinner-rotate {
    to { transform: rotate(360deg); }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
      border-right-color: var(--cyan);
    }
  }
</style>

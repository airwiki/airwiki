<script context="module" lang="ts">
  let nextCheckboxId = 0;

  function allocateCheckboxDescriptionId(): string {
    const id = nextCheckboxId;
    nextCheckboxId += 1;
    return `checkbox-${id}-description`;
  }
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  export let label: string;
  export let accessibleLabel: string = label;
  export let checked = false;
  export let description: string | undefined = undefined;
  export let disabled = false;
  export let compact = false;
  export let leading: Snippet | undefined = undefined;
  export let onchange: ((checked: boolean) => void) | undefined = undefined;

  const descriptionId = allocateCheckboxDescriptionId();

  function update(nextChecked: boolean) {
    checked = nextChecked;
    onchange?.(nextChecked);
  }
</script>

<label class:compact class:disabled class:checked class:has-leading={Boolean(leading)} class="check-control">
  <input
    type="checkbox"
    checked={checked}
    {disabled}
    aria-label={accessibleLabel}
    aria-describedby={description ? descriptionId : undefined}
    onchange={(event) => update(event.currentTarget.checked)}
  />
  <span aria-hidden="true" class="check-indicator"></span>
  {#if leading}<span class="check-leading">{@render leading()}</span>{/if}
  <span class="check-copy">
    <span class="check-label">{label}</span>
    {#if description}<small id={descriptionId}>{description}</small>{/if}
  </span>
</label>

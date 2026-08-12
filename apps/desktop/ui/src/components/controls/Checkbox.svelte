<script context="module" lang="ts">
  let nextCheckboxId = 0;

  function allocateCheckboxDescriptionId(): string {
    const id = nextCheckboxId;
    nextCheckboxId += 1;
    return `checkbox-${id}-description`;
  }
</script>

<script lang="ts">
  export let label: string;
  export let checked = false;
  export let description: string | undefined = undefined;
  export let disabled = false;
  export let compact = false;
  export let onchange: ((checked: boolean) => void) | undefined = undefined;

  const descriptionId = allocateCheckboxDescriptionId();

  function update(nextChecked: boolean) {
    checked = nextChecked;
    onchange?.(nextChecked);
  }
</script>

<label class:compact class:disabled class="check-control">
  <input
    type="checkbox"
    checked={checked}
    {disabled}
    aria-label={label}
    aria-describedby={description ? descriptionId : undefined}
    onchange={(event) => update(event.currentTarget.checked)}
  />
  <span aria-hidden="true" class="check-indicator"></span>
  <span class="check-copy">
    <span class="check-label">{label}</span>
    {#if description}<small id={descriptionId}>{description}</small>{/if}
  </span>
</label>

<script context="module" lang="ts">
  let nextSwitchId = 0;

  function allocateSwitchDescriptionId(): string {
    const id = nextSwitchId;
    nextSwitchId += 1;
    return `switch-${id}-description`;
  }
</script>

<script lang="ts">
  export let label: string;
  export let checked = false;
  export let description: string | undefined = undefined;
  export let disabled = false;
  export let onchange: ((checked: boolean) => void) | undefined = undefined;

  const descriptionId = allocateSwitchDescriptionId();

  function update(nextChecked: boolean) {
    checked = nextChecked;
    onchange?.(nextChecked);
  }
</script>

<label class:disabled class="switch-control">
  <span class="switch-copy">
    <span class="switch-label">{label}</span>
    {#if description}<small id={descriptionId}>{description}</small>{/if}
  </span>
  <input
    type="checkbox"
    role="switch"
    checked={checked}
    {disabled}
    aria-label={label}
    aria-describedby={description ? descriptionId : undefined}
    onchange={(event) => update(event.currentTarget.checked)}
  />
  <span aria-hidden="true" class="switch-track"><span></span></span>
</label>

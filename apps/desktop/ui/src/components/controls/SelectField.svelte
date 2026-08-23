<script context="module" lang="ts">
  let nextSelectId = 0;

  function allocateSelectId(): string {
    const id = nextSelectId;
    nextSelectId += 1;
    return `select-${id}`;
  }
</script>

<script lang="ts">
  type SelectOption = { value: string; label: string };

  export let label: string;
  export let value: string;
  export let options: SelectOption[];
  export let description: string | undefined = undefined;
  export let disabled = false;
  export let onchange: ((value: string) => void) | undefined = undefined;

  const controlId = allocateSelectId();
  const descriptionId = `${controlId}-description`;
</script>

<div class="control-field">
  <label class="control-label" for={controlId}>{label}</label>
  <span class="select-control">
    <select id={controlId} bind:value {disabled} aria-describedby={description ? descriptionId : undefined} onchange={(event) => onchange?.(event.currentTarget.value)}>
      {#each options as option (option.value)}
        <option value={option.value}>{option.label}</option>
      {/each}
    </select>
    <span aria-hidden="true" class="select-chevron"></span>
  </span>
  {#if description}<small id={descriptionId} class="control-description">{description}</small>{/if}
</div>

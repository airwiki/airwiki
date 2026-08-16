<script context="module" lang="ts">
  let nextFieldId = 0;

  function allocateFieldId(): string {
    const id = nextFieldId;
    nextFieldId += 1;
    return `field-${id}`;
  }
</script>

<script lang="ts">
  export let label: string;
  export let id: string | undefined = undefined;
  export let value = '';
  export let description: string | undefined = undefined;
  export let error: string | undefined = undefined;
  export let describedby: string | undefined = undefined;
  export let placeholder = '';
  export let autocomplete: 'off' | 'on' = 'off';
  export let maxlength: number | undefined = undefined;
  export let rows = 3;
  export let required = false;
  export let disabled = false;
  export let multiline = false;
  export let variant: 'standard' | 'search' = 'standard';
  export let onfocus: (() => void) | undefined = undefined;
  export let oninput: ((value: string) => void) | undefined = undefined;

  const generatedId = allocateFieldId();
  $: controlId = id ?? generatedId;
  $: descriptionId = description ? `${controlId}-description` : undefined;
  $: errorId = error ? `${controlId}-error` : undefined;
  $: describedBy = [describedby, descriptionId, errorId].filter(Boolean).join(' ') || undefined;

  function update(nextValue: string) {
    value = nextValue;
    oninput?.(nextValue);
  }
</script>

<div class:search={variant === 'search'} class="control-field">
  <label class:sr-only={variant === 'search'} class="control-label" for={controlId}>{label}</label>
  {#if multiline}
    <textarea
      id={controlId}
      value={value}
      {placeholder}
      {autocomplete}
      {maxlength}
      {rows}
      {required}
      {disabled}
      aria-invalid={error ? 'true' : undefined}
      aria-describedby={describedBy}
      onfocus={() => onfocus?.()}
      oninput={(event) => update(event.currentTarget.value)}
    ></textarea>
  {:else}
    <input
      id={controlId}
      value={value}
      {placeholder}
      {autocomplete}
      {maxlength}
      {required}
      {disabled}
      aria-invalid={error ? 'true' : undefined}
      aria-describedby={describedBy}
      onfocus={() => onfocus?.()}
      oninput={(event) => update(event.currentTarget.value)}
    />
  {/if}
  {#if description}<small id={descriptionId} class="control-description">{description}</small>{/if}
  {#if error}<small id={errorId} class="control-error">{error}</small>{/if}
</div>

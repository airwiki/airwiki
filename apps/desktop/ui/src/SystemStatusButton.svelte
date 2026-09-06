<script lang="ts">
  import Settings2 from '@lucide/svelte/icons/settings-2';
  import type { AppSnapshot } from './api';
  import type { MessageArgs } from './i18n';
  import { pendingApprovalCount, systemStatuses } from './systemStatus';

  export let snapshot: AppSnapshot;
  export let t: (id: string, args?: MessageArgs) => string;
  export let onclick: () => void;

  let statuses: ReturnType<typeof systemStatuses>;
  let approvals: number;
  let accessibleLabel: string;
  $: statuses = systemStatuses(snapshot, t);
  $: approvals = pendingApprovalCount(snapshot);
  $: accessibleLabel = [
    t('desktop-nav-system'),
    ...statuses.map((status) => `${status.label}: ${status.detail}`),
    ...(approvals > 0 ? [t('desktop-status-ai-apps-pending', { count: approvals })] : [])
  ].join('. ');
</script>

<button
  class="system-status-button"
  aria-label={accessibleLabel}
  title={accessibleLabel}
  {onclick}
>
  <Settings2 size={17} strokeWidth={2} aria-hidden="true" />
  <span class="settings-button-copy"><strong>{t('desktop-nav-system')}</strong>{#each statuses.filter((status) => status.tone === 'warning' || status.tone === 'failed' || status.tone === 'working') as status (status.id)}<small class={status.tone}>{status.label}: {status.detail}</small>{/each}</span>
  {#if approvals > 0}<span class="status-approval-badge" aria-hidden="true">{approvals > 9 ? '9+' : approvals}</span>{/if}
</button>

<style>
  .system-status-button { display: flex; gap: 10px; width: 100%; height: auto; min-height: 36px; padding: 8px; border: 1px solid transparent; border-radius: var(--control-radius); color: var(--muted); background: transparent; text-align: left; cursor: pointer; }
  .system-status-button:hover { color: var(--strong); background: var(--nav-active); }
  .system-status-button > :global(svg) { position: static; flex: none; }
  .settings-button-copy { display: grid; flex: 1; gap: 4px; min-width: 0; }
  strong { font-size: var(--font-size-ui); font-weight: 500; }
  small { font-size: 11px; line-height: 1.4; }
  .warning, .failed { color: var(--recovery-accent); }
  .status-approval-badge { position: static; flex: none; }
</style>

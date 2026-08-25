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
  <svg class="status-donut" viewBox="0 0 44 44" aria-hidden="true">
    <circle class="status-donut-track" cx="22" cy="22" r="18" pathLength="100" />
    {#each statuses as status, index (status.id)}
      <circle
        class={`status-segment segment-${index + 1} ${status.tone}`}
        cx="22"
        cy="22"
        r="18"
        pathLength="100"
        stroke-dasharray="30 70"
        stroke-dashoffset={index * -33.333}
        transform="rotate(-90 22 22)"
      />
    {/each}
  </svg>
  <Settings2 size={17} strokeWidth={2} aria-hidden="true" />
  {#if approvals > 0}<span class="status-approval-badge" aria-hidden="true">{approvals > 9 ? '9+' : approvals}</span>{/if}
</button>

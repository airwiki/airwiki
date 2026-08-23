<script lang="ts">
  import Blocks from '@lucide/svelte/icons/blocks';
  import BrainCircuit from '@lucide/svelte/icons/brain-circuit';
  import Network from '@lucide/svelte/icons/network';
  import ShimmerText from './components/ShimmerText.svelte';
  import type { AppSnapshot } from './api';
  import type { MessageArgs } from './i18n';

  type ServiceTarget = 'knowledge' | 'connections' | 'apps';
  type ServiceTone = 'ready' | 'working' | 'off' | 'attention';
  type ServiceStatus = { id: ServiceTarget; label: string; detail: string; tone: ServiceTone };

  export let snapshot: AppSnapshot;
  export let t: (id: string, args?: MessageArgs) => string;
  export let onselect: (target: ServiceTarget) => void;

  function statuses(current: AppSnapshot, translate: typeof t): ServiceStatus[] {
    const indexingWorking = current.wikiScans.length > 0 || (current.wikiHealth?.updatingCount ?? 0) > 0;
    const indexingFailed = current.wikiHealth?.status === 'failed' || (current.wikiHealth?.errorCount ?? 0) > 0;
    const knowledge: ServiceStatus = current.modelInstall
      ? { id: 'knowledge', label: translate('desktop-status-knowledge'), detail: translate('desktop-status-knowledge-preparing'), tone: 'working' }
      : indexingFailed
        ? { id: 'knowledge', label: translate('desktop-status-knowledge'), detail: translate('status-needs-attention'), tone: 'attention' }
        : !current.model?.active
          ? { id: 'knowledge', label: translate('desktop-status-knowledge'), detail: translate('desktop-status-knowledge-needs-setup'), tone: 'off' }
          : indexingWorking
            ? { id: 'knowledge', label: translate('desktop-status-knowledge'), detail: translate('desktop-status-knowledge-preparing'), tone: 'working' }
            : { id: 'knowledge', label: translate('desktop-status-knowledge'), detail: translate('desktop-status-knowledge-ready'), tone: 'ready' };

    const lanWorking = current.lanRuntime?.listener === 'starting' || current.lanRuntime?.discovery === 'starting';
    const lanFailed = current.lanRuntime?.listener === 'failed' || current.lanRuntime?.discovery === 'failed';
    const lanAvailable = current.lanRuntime?.listener === 'listening' && current.lanRuntime.discovery === 'active';
    const publicWikis = current.wikis.filter((wiki) => wiki.internetPublic);
    const publicAvailable = publicWikis.some((wiki) => wiki.publicAnnouncement.status === 'advertised');
    const publicFailed = publicWikis.length > 0 && !publicAvailable;
    const connections: ServiceStatus = lanWorking
      ? { id: 'connections', label: translate('desktop-status-connections'), detail: translate('status-working'), tone: 'working' }
      : lanFailed || publicFailed
        ? { id: 'connections', label: translate('desktop-status-connections'), detail: translate('status-needs-attention'), tone: 'attention' }
        : lanAvailable && publicAvailable
          ? { id: 'connections', label: translate('desktop-status-connections'), detail: translate('desktop-status-nearby-and-public'), tone: 'ready' }
          : lanAvailable
            ? { id: 'connections', label: translate('desktop-status-connections'), detail: translate('desktop-status-nearby-available'), tone: 'ready' }
            : publicAvailable
              ? { id: 'connections', label: translate('desktop-status-connections'), detail: translate('desktop-status-public-active'), tone: 'ready' }
              : { id: 'connections', label: translate('desktop-status-connections'), detail: translate('desktop-status-private'), tone: 'off' };

    const connectedApps = current.integrations?.integrations.filter((integration) => integration.status === 'configured').length ?? 0;
    const apps: ServiceStatus = current.mcpUrl
      ? { id: 'apps', label: translate('desktop-status-ai-apps'), detail: connectedApps > 0 ? translate('desktop-status-ai-apps-connected', { count: connectedApps }) : translate('desktop-status-available'), tone: 'ready' }
      : { id: 'apps', label: translate('desktop-status-ai-apps'), detail: translate('desktop-status-unavailable'), tone: 'attention' };

    return [knowledge, connections, apps];
  }

  let services: ServiceStatus[];
  $: services = statuses(snapshot, t);
</script>

<footer class="system-status-bar" aria-label={t('desktop-system-status')}>
  <strong>{t('desktop-system-status')}</strong>
  <div>
    {#each services as service (service.id)}
      <button onclick={() => onselect(service.id)} aria-label={`${service.label}: ${service.detail}`}>
        <span class="service-icon" aria-hidden="true">
          {#if service.id === 'knowledge'}<BrainCircuit size={17} strokeWidth={1.9} />{:else if service.id === 'connections'}<Network size={17} strokeWidth={1.9} />{:else}<Blocks size={17} strokeWidth={1.9} />{/if}
          <span class={`status-dot ${service.tone}`}></span>
        </span>
        <span class="service-copy"><span>{service.label}</span><small>{#if service.id === 'knowledge' && service.tone === 'working'}<ShimmerText text={service.detail} />{:else}{service.detail}{/if}</small></span>
      </button>
    {/each}
  </div>
</footer>

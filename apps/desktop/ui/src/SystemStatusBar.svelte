<script lang="ts">
  import type { AppSnapshot } from './api';

  type ServiceTarget = 'knowledge' | 'connections' | 'apps';
  type ServiceTone = 'ready' | 'working' | 'off' | 'attention';
  type ServiceStatus = { id: ServiceTarget; label: string; detail: string; tone: ServiceTone };

  export let snapshot: AppSnapshot;
  export let t: (id: string) => string;
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

    const apps: ServiceStatus = current.mcpUrl
      ? { id: 'apps', label: translate('desktop-status-ai-apps'), detail: translate('desktop-status-available'), tone: 'ready' }
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
        <span class={`status-dot ${service.tone}`} aria-hidden="true"></span>
        <span>{service.label}</span>
        <small>{service.detail}</small>
      </button>
    {/each}
  </div>
</footer>

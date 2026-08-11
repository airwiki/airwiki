<script lang="ts">
  import type { AppSnapshot } from './api';

  type ServiceTarget = 'model' | 'lan' | 'public' | 'mcp' | 'indexing';
  type ServiceTone = 'ready' | 'working' | 'off' | 'attention';
  type ServiceStatus = { id: ServiceTarget; label: string; detail: string; tone: ServiceTone };

  export let snapshot: AppSnapshot;
  export let t: (id: string) => string;
  export let onselect: (target: ServiceTarget) => void;

  function statuses(current: AppSnapshot, translate: typeof t): ServiceStatus[] {
    const model: ServiceStatus = current.modelInstall
      ? { id: 'model', label: translate('desktop-status-local-ai'), detail: translate('status-working'), tone: 'working' }
      : current.model?.degraded
        ? { id: 'model', label: translate('desktop-status-local-ai'), detail: translate('status-needs-attention'), tone: 'attention' }
        : current.model?.active
          ? { id: 'model', label: translate('desktop-status-local-ai'), detail: translate('status-ready'), tone: 'ready' }
          : { id: 'model', label: translate('desktop-status-local-ai'), detail: translate('desktop-status-not-configured'), tone: 'off' };

    const lan: ServiceStatus = current.lanRuntime?.listener === 'starting'
      ? { id: 'lan', label: translate('desktop-status-lan'), detail: translate('status-working'), tone: 'working' }
      : current.lanRuntime?.listener === 'failed'
        ? { id: 'lan', label: translate('desktop-status-lan'), detail: translate('status-needs-attention'), tone: 'attention' }
        : current.lanRuntime?.listener === 'listening'
          ? { id: 'lan', label: translate('desktop-status-lan'), detail: translate('desktop-status-available'), tone: 'ready' }
          : { id: 'lan', label: translate('desktop-status-lan'), detail: translate('status-optional-disabled'), tone: 'off' };

    const publicWikis = current.wikis.filter((wiki) => wiki.internetPublic);
    const publicSharing: ServiceStatus = publicWikis.some((wiki) => wiki.publicAnnouncement.status === 'advertised')
      ? { id: 'public', label: translate('desktop-status-public-sharing'), detail: translate('desktop-status-sharing'), tone: 'ready' }
      : publicWikis.length > 0
        ? { id: 'public', label: translate('desktop-status-public-sharing'), detail: translate('desktop-status-not-published'), tone: 'attention' }
        : { id: 'public', label: translate('desktop-status-public-sharing'), detail: translate('desktop-status-not-shared'), tone: 'off' };

    const mcp: ServiceStatus = current.mcpUrl
      ? { id: 'mcp', label: 'MCP', detail: translate('desktop-status-available'), tone: 'ready' }
      : { id: 'mcp', label: 'MCP', detail: translate('desktop-status-unavailable'), tone: 'attention' };

    const indexing: ServiceStatus = current.wikiScans.length > 0 || (current.wikiHealth?.updatingCount ?? 0) > 0
      ? { id: 'indexing', label: translate('desktop-status-indexing'), detail: translate('status-working'), tone: 'working' }
      : current.wikiHealth?.status === 'failed' || (current.wikiHealth?.errorCount ?? 0) > 0
        ? { id: 'indexing', label: translate('desktop-status-indexing'), detail: translate('status-needs-attention'), tone: 'attention' }
        : { id: 'indexing', label: translate('desktop-status-indexing'), detail: translate('status-ready'), tone: 'ready' };

    return [model, lan, publicSharing, mcp, indexing];
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

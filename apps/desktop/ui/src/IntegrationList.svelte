<script lang="ts">
  import type { IntegrationActionInput, IntegrationSummary } from './api';
  import type { MessageArgs } from './i18n';

  let {
    integrations,
    busy,
    t,
    onaction,
    oncopy
  }: {
    integrations: IntegrationSummary[];
    busy: boolean;
    t: (id: string, args?: MessageArgs) => string;
    onaction: (action: IntegrationActionInput) => void;
    oncopy: (command: string, args: string[]) => void;
  } = $props();

  function clientName(client: IntegrationSummary['client']): string {
    if (client === 'chatGptDesktop') return 'ChatGPT Desktop / Work';
    if (client === 'claudeDesktop') return 'Claude Desktop';
    if (client === 'claudeCode') return 'Claude Code';
    if (client === 'geminiCli') return 'Gemini CLI';
    return t('integrations-generic-mcp');
  }

  function connectionState(status: IntegrationSummary['status']): string {
    const labels: Record<IntegrationSummary['status'], string> = {
      notInstalled: 'integration-status-not-installed',
      available: 'integration-status-available',
      awaitingClientApproval: 'integration-status-awaiting-approval',
      configured: 'integration-status-configured',
      updateAvailable: 'integration-status-update-available',
      conflict: 'integration-status-conflict',
      unsupported: 'integration-status-unsupported',
      error: 'integration-status-error'
    };
    return t(labels[status]);
  }

  function guideState(status: IntegrationSummary['workflowGuide']['status']): string {
    return t(`workflow-guide-status-${status}`);
  }

  function setupText(command: string, args: string[]): string {
    return JSON.stringify({ command, args }, null, 2);
  }

  function connectOrUpdate(integration: IntegrationSummary) {
    onaction({ kind: 'connect', client: integration.client });
  }

  function installGuide(integration: IntegrationSummary) {
    onaction({ kind: 'installWorkflowGuide', client: integration.client });
  }

  function removeGuide(integration: IntegrationSummary) {
    onaction({ kind: 'removeWorkflowGuide', client: integration.client });
  }
</script>

<div class="integration-list">
  {#each integrations as integration (integration.client)}
    <article class="integration-item">
      <div class="integration-client-heading">
        <strong>{clientName(integration.client)}</strong>
        {#if integration.detectedVersion}<small>{t('integrations-version', { version: integration.detectedVersion })}</small>{/if}
        {#if integration.activityRecent}<small class="recent-activity">{t('integrations-recent-activity')}</small>{/if}
      </div>
      <dl class="integration-state-list">
        <div>
          <dt>{t('integrations-local-connection')}</dt>
          <dd class:state-warning={integration.status === 'conflict' || integration.status === 'error'}>{connectionState(integration.status)}</dd>
        </div>
        <div>
          <dt>{t('integrations-assisted-memory')}</dt>
          <dd class:state-warning={integration.workflowGuide.status === 'conflict'}>{guideState(integration.workflowGuide.status)}</dd>
        </div>
      </dl>
      <div class="integration-actions">
        {#if (integration.status === 'available' || integration.status === 'updateAvailable') && integration.workflowGuide.status !== 'conflict' && integration.workflowGuide.status !== 'unsupported'}
          <button class="secondary" disabled={busy} onclick={() => connectOrUpdate(integration)}>
            {integration.status === 'updateAvailable' ? t('integrations-update') : t('integrations-connect')}
          </button>
        {:else if (integration.status === 'configured' || integration.status === 'conflict') && (integration.workflowGuide.status === 'available' || integration.workflowGuide.status === 'updateAvailable')}
          <button class="secondary" disabled={busy} onclick={() => installGuide(integration)}>
            {integration.workflowGuide.status === 'updateAvailable' ? t('workflow-guide-update') : t('workflow-guide-install')}
          </button>
        {:else if integration.status === 'configured'}
          <button class="text-action" disabled={busy} onclick={() => onaction({ kind: 'disconnect', client: integration.client })}>{t('integrations-disconnect')}</button>
        {/if}
        {#if integration.workflowGuide.kind === 'nativeSkill' && (integration.workflowGuide.status === 'installed' || integration.workflowGuide.status === 'updateAvailable')}
          <button class="text-action subtle-action" disabled={busy} onclick={() => removeGuide(integration)}>{t('workflow-guide-remove')}</button>
        {/if}
      </div>
      {#if integration.workflowGuide.status === 'conflict'}
        <p class="integration-guidance warning-text">{t('workflow-guide-conflict-help')}</p>
      {:else if integration.workflowGuide.restartRequired && (integration.workflowGuide.status === 'installed' || integration.workflowGuide.status === 'builtIn')}
        <p class="integration-guidance">{t('workflow-guide-new-conversation')}</p>
      {/if}
      {#if integration.mcpSetup}
        <div class="mcp-setup">
          <div><strong>{t('integrations-generic-setup')}</strong><small>{t('integrations-generic-setup-help')}</small></div>
          <pre>{setupText(integration.mcpSetup.command, integration.mcpSetup.args)}</pre>
          <button class="secondary" onclick={() => oncopy(integration.mcpSetup?.command ?? '', integration.mcpSetup?.args ?? [])}>{t('action-copy')}</button>
        </div>
      {/if}
    </article>
  {/each}
</div>

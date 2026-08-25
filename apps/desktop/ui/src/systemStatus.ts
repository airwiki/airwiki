import type { AppSnapshot } from './api';
import type { MessageArgs } from './i18n';

export type SystemStatusTarget = 'general' | 'connections' | 'apps';
export type SystemStatusTone = 'ready' | 'working' | 'off' | 'warning' | 'failed';
export type SystemStatusItem = {
  id: SystemStatusTarget;
  label: string;
  detail: string;
  tone: SystemStatusTone;
};

type Translate = (id: string, args?: MessageArgs) => string;

export function systemStatuses(snapshot: AppSnapshot, t: Translate): SystemStatusItem[] {
  const indexingWorking = snapshot.wikiScans.length > 0 || (snapshot.wikiHealth?.updatingCount ?? 0) > 0;
  const indexingFailed = snapshot.wikiHealth?.status === 'failed' || (snapshot.wikiHealth?.errorCount ?? 0) > 0;
  const knowledge = snapshot.modelInstall
    ? status('general', 'desktop-status-knowledge', 'desktop-status-knowledge-preparing', 'working', t)
    : indexingFailed
      ? status('general', 'desktop-status-knowledge', 'status-failed', 'failed', t)
      : !snapshot.model?.active
        ? status('general', 'desktop-status-knowledge', 'desktop-status-knowledge-needs-setup', 'off', t)
        : snapshot.model.issues.length > 0
          ? status('general', 'desktop-status-knowledge', 'status-needs-attention', 'warning', t)
          : indexingWorking
            ? status('general', 'desktop-status-knowledge', 'desktop-status-knowledge-preparing', 'working', t)
            : status('general', 'desktop-status-knowledge', 'desktop-status-knowledge-ready', 'ready', t);

  const lanWorking = snapshot.lanRuntime?.listener === 'starting' || snapshot.lanRuntime?.discovery === 'starting';
  const lanFailed = snapshot.lanRuntime?.listener === 'failed' || snapshot.lanRuntime?.discovery === 'failed';
  const lanAvailable = snapshot.lanRuntime?.listener === 'listening' && snapshot.lanRuntime.discovery === 'active';
  const publicWikis = snapshot.wikis.filter((wiki) => wiki.internetPublic);
  const publicAvailable = publicWikis.some((wiki) => wiki.publicAnnouncement.status === 'advertised');
  const publicFailed = publicWikis.length > 0 && !publicAvailable;
  const connections = lanWorking
    ? status('connections', 'desktop-status-connections', 'status-working', 'working', t)
    : lanFailed
      ? status('connections', 'desktop-status-connections', 'status-failed', 'failed', t)
      : publicFailed
        ? status('connections', 'desktop-status-connections', 'status-needs-attention', 'warning', t)
        : lanAvailable && publicAvailable
          ? status('connections', 'desktop-status-connections', 'desktop-status-nearby-and-public', 'ready', t)
          : lanAvailable
            ? status('connections', 'desktop-status-connections', 'desktop-status-nearby-available', 'ready', t)
            : publicAvailable
              ? status('connections', 'desktop-status-connections', 'desktop-status-public-active', 'ready', t)
              : status('connections', 'desktop-status-connections', 'desktop-status-private', 'off', t);

  const integrations = snapshot.integrations?.integrations ?? [];
  const connectedApps = integrations.filter((integration) => integration.status === 'configured').length;
  const integrationFailed = integrations.some((integration) => integration.status === 'error' || integration.status === 'conflict');
  const integrationWarning = integrations.some((integration) => integration.status === 'awaitingClientApproval' || integration.status === 'updateAvailable');
  const pendingApprovals = snapshot.projectMemoryRequests.length + snapshot.pendingComputations.length;
  const apps = integrationFailed
    ? status('apps', 'desktop-status-ai-apps', 'status-failed', 'failed', t)
    : pendingApprovals > 0
      ? {
        id: 'apps' as const,
        label: t('desktop-status-ai-apps'),
        detail: t('desktop-status-ai-apps-pending', { count: pendingApprovals }),
        tone: 'warning' as const
      }
      : integrationWarning
        ? status('apps', 'desktop-status-ai-apps', 'status-needs-attention', 'warning', t)
        : snapshot.mcpUrl
          ? {
            id: 'apps' as const,
            label: t('desktop-status-ai-apps'),
            detail: connectedApps > 0
              ? t('desktop-status-ai-apps-connected', { count: connectedApps })
              : t('desktop-status-available'),
            tone: 'ready' as const
          }
          : status('apps', 'desktop-status-ai-apps', 'desktop-status-unavailable', 'off', t);

  return [knowledge, connections, apps];
}

function status(
  id: SystemStatusTarget,
  labelId: string,
  detailId: string,
  tone: SystemStatusTone,
  t: Translate
): SystemStatusItem {
  return { id, label: t(labelId), detail: t(detailId), tone };
}

export function pendingApprovalCount(snapshot: AppSnapshot): number {
  return snapshot.projectMemoryRequests.length + snapshot.pendingComputations.length;
}

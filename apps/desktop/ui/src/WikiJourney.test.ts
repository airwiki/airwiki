import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import WikiJourney from './WikiJourney.svelte';
import type { ApplicationAccessSummary, IntegrationSummary, WikiSummary } from './api';
import { message, type MessageArgs } from './i18n';
import { readySnapshot } from './test/fixtures';

const t = (id: string, args?: MessageArgs) => message('es', id, args);

function integration(overrides: Partial<IntegrationSummary> = {}): IntegrationSummary {
  return {
    client: 'chatGptDesktop',
    status: 'configured',
    detectedVersion: '1.0.0',
    activityRecent: false,
    restartRequired: false,
    mcpSetup: null,
    workflowGuide: { kind: 'nativeSkill', status: 'installed', version: '1', restartRequired: false },
    ...overrides
  };
}

function application(overrides: Partial<ApplicationAccessSummary> = {}): ApplicationAccessSummary {
  return {
    appId: 'codex-desktop',
    clientName: 'codex',
    displayName: 'Codex',
    producer: 'OpenAI',
    active: true,
    ownedWikiCount: 0,
    managedBytes: 0,
    grants: [{ wikiId: readySnapshot().wikis[0].id, role: 'reader' }],
    ...overrides
  };
}

function setup(
  wikiOverrides: Partial<WikiSummary> = {},
  integrations: IntegrationSummary[] = [],
  applications: ApplicationAccessSummary[] = [],
  repairAvailable = false,
  reanalyzing = false
) {
  const wiki = { ...readySnapshot().wikis[0], ...wikiOverrides };
  const callbacks = {
    onreview: vi.fn(), ondetails: vi.fn(), onrepair: vi.fn(), onaccess: vi.fn(), onapps: vi.fn()
  };
  const result = render(WikiJourney, {
    wiki,
    scanState: null,
    reanalyzing,
    sourceIssueCount: 0,
    peerAccessCount: 0,
    repairAvailable,
    integrations,
    applications,
    integrationsBusy: false,
    t,
    ...callbacks
  });
  return { ...result, wiki, callbacks };
}

describe('Wiki compact status bar', () => {
  afterEach(cleanup);

  it('keeps knowledge, exposure and sharing in one compact control surface', () => {
    setup();

    const summary = screen.getByRole('region', { name: 'Estado de Atlas mientras exploras su contenido' });
    expect(within(summary).getByText('Atlas')).toBeInTheDocument();
    expect(within(summary).getByText('Buscable')).toBeInTheDocument();
    expect(within(summary).getByText('2 de 2 revisados · 0 borradores · 0 excluidos')).toBeInTheDocument();
    expect(within(summary).getByLabelText('Local: Activa')).toBeInTheDocument();
    expect(within(summary).getByLabelText('LAN: Desactivada')).toBeInTheDocument();
    expect(within(summary).getByLabelText('Internet: Desactivada')).toBeInTheDocument();
    expect(within(summary).getByText('Sin apps conectadas')).toBeInTheDocument();
    expect(within(summary).getByRole('button', { name: 'Compartir' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: '¿Está lista esta wiki?' })).not.toBeInTheDocument();
  });

  it('shows every detected AI destination with its own identity and Wiki access state', () => {
    const { container } = setup(
      { localOnly: false, allowExternalAi: true },
      [integration(), integration({ client: 'claudeCode' })],
      [
        application({ appId: 'chatgpt-desktop', clientName: 'chatgpt-desktop', displayName: 'ChatGPT/Codex', producer: 'codex/managed' }),
        application({ appId: 'claude-code', clientName: 'claude-code', displayName: 'Claude Code', producer: 'Anthropic' })
      ]
    );

    expect(screen.getByText('Acceso en 2 apps')).toBeInTheDocument();
    expect(container.querySelector('[title="ChatGPT/Codex: Acceso permitido"]')).not.toBeNull();
    expect(container.querySelector('[title="Claude Code: Acceso permitido"]')).not.toBeNull();
    expect(container.querySelectorAll('[title="ChatGPT/Codex: Acceso permitido"]')).toHaveLength(1);
    expect(screen.getByRole('button', { name: /Gestionar apps de IA.*Acceso en 2 apps/ })).toBeInTheDocument();
  });

  it('includes explicitly granted applications such as Codex when AI access is active', () => {
    const { container } = setup({ localOnly: false, allowExternalAi: true }, [], [application()]);

    expect(screen.getByText('Acceso en 1 app')).toBeInTheDocument();
    expect(container.querySelector('[title="Codex: Acceso permitido"]')).not.toBeNull();
  });

  it.each(['invalid', 'missing', 'identityConflict'] as const)(
    'blocks persisted AI grants while project memory health is %s',
    (projectMemoryHealth) => {
      const { container } = setup(
        {
          origin: 'aiMemory',
          memoryKind: 'project',
          projectMemoryHealth,
          localOnly: false,
          peerShareable: true,
          allowExternalAi: true,
          internetPublic: true,
          publicAnnouncement: { status: 'advertised', acceptedIndexes: 1 }
        },
        [integration()],
        [application()]
      );

      expect(screen.getByText('Acceso bloqueado')).toBeInTheDocument();
      expect(screen.queryByText('Acceso en 1 app')).not.toBeInTheDocument();
      expect(container.querySelector('[title="Codex: Conectada · sin acceso"]')).not.toBeNull();
      expect(screen.getByLabelText('LAN: No disponible')).toBeInTheDocument();
      expect(screen.getByLabelText('Internet: No disponible')).toBeInTheDocument();
    }
  );

  it('does not confuse a configured client with access to this Wiki', () => {
    const { container } = setup({}, [integration()]);

    expect(screen.getByText('Revisar conexiones')).toBeInTheDocument();
    expect(screen.queryByText('Acceso en 1 app')).not.toBeInTheDocument();
    expect(container.querySelector('[title="ChatGPT: Conectada · sin acceso"]')).not.toBeNull();
  });

  it('distinguishes enabled-but-offline Internet exposure from actual publication', () => {
    setup({ localOnly: false, peerShareable: true, internetPublic: true, publicAnnouncement: { status: 'offline' } });

    expect(screen.getByLabelText('LAN: Habilitada')).toBeInTheDocument();
    expect(screen.getByLabelText('Internet: Habilitada · offline')).toBeInTheDocument();
    expect(screen.queryByLabelText('Internet: Pública')).not.toBeInTheDocument();
  });

  it('opens sharing and AI settings from explicit, keyboard-operable controls', async () => {
    const { callbacks } = setup();

    await fireEvent.click(screen.getByRole('button', { name: 'Compartir' }));
    expect(callbacks.onaccess).toHaveBeenCalledOnce();
    await fireEvent.click(screen.getByRole('button', { name: /Gestionar apps de IA/ }));
    expect(callbacks.onapps).toHaveBeenCalledOnce();
  });

  it('keeps guided repair and pending review reachable through the knowledge status', async () => {
    const repair = setup({ maintenanceRequired: true }, [], [], true);
    await fireEvent.click(screen.getByRole('button', { name: /Hay que comprobar el contenido publicado.*Revisar reparación segura/ }));
    expect(repair.callbacks.onrepair).toHaveBeenCalledOnce();

    cleanup();
    const review = setup({ needsReviewCount: 2 });
    await fireEvent.click(screen.getByRole('button', { name: /Revisa 2 cambios.*Revisar propuestas/ }));
    expect(review.callbacks.onreview).toHaveBeenCalledOnce();
  });

  it('shows Wiki-level reanalysis as one background operation', () => {
    setup({ needsReviewCount: 2 }, [], [], false, true);

    expect(screen.getByRole('button', { name: /Volviendo a analizar los borradores actuales/ })).toBeInTheDocument();
    expect(screen.getByText('Trabajando')).toBeInTheDocument();
  });
});

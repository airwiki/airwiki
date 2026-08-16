import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { allowPeerPairingAgain, approveReview, browsePublicWiki, configureFirewall, connect, loadWikiBundle, loadWikiPage, manageIntegration, openSystemDestination, pickOkfImport, prepareGuidedWikiRepair, quitCompletely, reanalyzeReview, rejectReview, updatePreferences, validateOkfImport, verifyWikiConcept } from './api';
import type { UiEventEnvelope } from './generated/ui-contract';
import { readySnapshot } from './test/fixtures';

let snapshot = readySnapshot();
let snapshotListener: ((event: UiEventEnvelope) => void) | null = null;
const tauriListeners = new Map<string, (event: unknown) => void>();
function activateLocalSearch() {
  snapshot.model = {
    stateSequence: 1, profile: 'automatic', recommendedModelId: 'synthetic-model',
    displayName: 'Synthetic local model', recommendationReason: null, active: true,
    installed: true, degraded: false, issues: [], pendingModelId: null,
    downloadBytes: 0, requiredFreeBytes: 0, fitsAvailableDisk: true,
    licenseAccepted: true, license: null, licenseUrl: null, revision: 'fixture'
  };
}
const accessibilityCases = (['es', 'en'] as const).flatMap((locale) =>
  (['light', 'dark'] as const).flatMap((theme) =>
    (['wikis', 'search', 'system/models', 'system/preferences', 'system/updates'] as const)
      .map((route) => [locale, theme, route] as const)
  )
);

vi.mock('./api', async (importOriginal) => {
  const original = await importOriginal() as typeof import('./api');
  return {
    ...original,
    connect: vi.fn(async (onEvent: (event: UiEventEnvelope) => void) => {
      snapshotListener = onEvent;
      return snapshot;
    }),
    refreshAutostart: vi.fn(async () => undefined),
    refreshConnectivity: vi.fn(async () => undefined),
    refreshWikiHealth: vi.fn(async () => undefined),
    prepareGuidedWikiRepair: vi.fn(async () => 'repair-request'),
    updatePreferences: vi.fn(async () => undefined),
    configureFirewall: vi.fn(async () => undefined),
    openSystemDestination: vi.fn(async () => undefined),
    manageIntegration: vi.fn(async () => undefined),
    allowPeerPairingAgain: vi.fn(async () => undefined),
    browsePublicWiki: vi.fn(async () => 'public-browse-request'),
    pickOkfImport: vi.fn(async () => null),
    validateOkfImport: vi.fn(),
    loadWikiBundle: vi.fn(async () => undefined),
    loadWikiPage: vi.fn(async () => undefined),
    verifyWikiConcept: vi.fn(async () => undefined),
    approveReview: vi.fn(async () => undefined),
    rejectReview: vi.fn(async () => undefined),
    reanalyzeReview: vi.fn(async () => undefined),
    quitCompletely: vi.fn(async () => undefined)
  };
});

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (event: unknown) => void) => {
    tauriListeners.set(event, handler);
    return () => { tauriListeners.delete(event); };
  })
}));

describe('AirWiki wiki workspace', () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    delete document.documentElement.dataset.theme;
    delete document.documentElement.dataset.platform;
    document.documentElement.style.colorScheme = '';
    Reflect.deleteProperty(window, '__TAURI_INTERNALS__');
  });

  beforeEach(() => {
    window.location.hash = '';
    snapshot = readySnapshot();
    snapshotListener = null;
    tauriListeners.clear();
  });

  it('renders one wiki workspace with global search and no redundant sidebar', async () => {
    render(App);

    expect((await screen.findAllByText('Atlas')).length).toBeGreaterThan(0);
    expect(screen.getByRole('heading', { name: 'Wikis' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Configuración' })).toBeInTheDocument();
    expect(screen.getByRole('search')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Nueva wiki' })).toBeInTheDocument();
    expect(screen.queryByRole('complementary')).not.toBeInTheDocument();
  });

  it('does not let a stale connect response replace a newer channel snapshot', async () => {
    const starting = readySnapshot();
    starting.sequence = 0;
    starting.phase = 'starting';
    starting.preferences = null;
    const latest = readySnapshot();
    latest.sequence = 2;
    vi.mocked(connect).mockImplementationOnce(async (onEvent) => {
      onEvent({
        schemaVersion: latest.schemaVersion,
        sequence: latest.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot: latest
      });
      return starting;
    });

    render(App);

    expect(await screen.findByRole('heading', { name: 'Wikis' })).toBeInTheDocument();
    expect(screen.queryByText('Trabajando')).not.toBeInTheDocument();
  });

  it('replaces an unrecoverable startup wait with a safe localized exit', async () => {
    snapshot.phase = 'failed';
    snapshot.preferences = null;

    const { container } = render(App);

    expect(await screen.findByRole('heading', { name: 'AirWiki could not start' })).toBeInTheDocument();
    expect(screen.queryByText('Working')).not.toBeInTheDocument();
    const quit = screen.getByRole('button', { name: 'Quit completely' });
    await fireEvent.click(quit);
    expect(quitCompletely).toHaveBeenCalledOnce();
    const results = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(results.violations.filter((violation) => ['critical', 'serious'].includes(violation.impact ?? ''))).toHaveLength(0);
  });

  it('groups technical services into clear user-facing system states', async () => {
    render(App);

    expect(await screen.findByRole('button', { name: 'Conocimiento local: Configura la búsqueda local' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Conexiones: Solo este dispositivo' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Apps de IA: Disponible' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /MCP:/ })).not.toBeInTheDocument();
  });

  it('summarizes ready local knowledge and both network scopes without losing their state', async () => {
    activateLocalSearch();
    snapshot.lanRuntime = { listener: 'listening', discovery: 'active', addressCount: 1 };
    snapshot.wikis[0].internetPublic = true;
    snapshot.wikis[0].publicAnnouncement = { status: 'advertised', acceptedIndexes: 1 };
    render(App);

    expect(await screen.findByRole('button', { name: 'Conocimiento local: Listo para buscar' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Conexiones: Cercana y pública' })).toBeInTheDocument();
  });

  it('keeps connections pending until nearby discovery is active', async () => {
    snapshot.lanRuntime = { listener: 'listening', discovery: 'starting', addressCount: 1 };
    render(App);

    expect(await screen.findByRole('button', { name: 'Conexiones: Trabajando' })).toBeInTheDocument();
  });

  it('surfaces a nearby discovery failure as an actionable connection state', async () => {
    snapshot.lanRuntime = { listener: 'listening', discovery: 'failed', addressCount: 1 };
    render(App);

    expect(await screen.findByRole('button', { name: 'Conexiones: Necesita atención' })).toBeInTheDocument();
  });

  it('gives advanced connection sections distinct names', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Conexiones: Solo este dispositivo' }));

    expect(screen.getByRole('dialog', { name: 'Conexiones' })).toBeInTheDocument();
    expect(screen.getByText('Controles de red')).toBeInTheDocument();
    expect(screen.getAllByText('Detalles avanzados')).toHaveLength(1);
    await waitFor(() => expect(manageIntegration).toHaveBeenCalledWith(expect.any(String), { kind: 'refresh' }));
  });

  it('loads integrations when connections open from public sharing', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Conexiones' }));

    expect(screen.getByRole('dialog', { name: 'Conexiones' })).toBeInTheDocument();
    await waitFor(() => expect(manageIntegration).toHaveBeenCalledWith(expect.any(String), { kind: 'refresh' }));
  });

  it('shows a copyable generic MCP setup without exposing a capability', async () => {
    snapshot.integrations = {
      externalAiWikiCount: 0,
      integrations: [{
        client: 'genericMcp', status: 'configured', detectedVersion: null,
        activityRecent: false, restartRequired: false,
        mcpSetup: { command: '/synthetic/managed/airwiki-mcp-bridge', args: ['--client', 'generic-mcp'] },
        workflowGuide: { kind: 'mcpInstructions', status: 'builtIn', version: '1', restartRequired: true }
      }]
    };
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Apps de IA: Disponible' }));

    expect(screen.getByText('Cliente MCP genérico')).toBeInTheDocument();
    const setup = screen.getByText(/synthetic\/managed\/airwiki-mcp-bridge/);
    expect(setup).toHaveTextContent('"generic-mcp"');
    expect(setup).not.toHaveTextContent('capability');
  });

  it('refreshes integrations when an integration deep link opens connections', async () => {
    window.location.hash = '#system/integrations';
    render(App);

    expect(await screen.findByRole('dialog', { name: 'Conexiones' })).toBeInTheDocument();
    await waitFor(() => expect(manageIntegration).toHaveBeenCalledWith(expect.any(String), { kind: 'refresh' }));
    expect(window.location.hash).toBe('#wikis');
  });

  it('uses durable snapshot state to finish a coalesced integration request', async () => {
    snapshot.integrations = {
      externalAiWikiCount: 0,
      integrations: [{
        client: 'genericMcp', status: 'available', detectedVersion: null,
        activityRecent: false, restartRequired: false, mcpSetup: null,
        workflowGuide: { kind: 'mcpInstructions', status: 'builtIn', version: '1', restartRequired: true }
      }]
    };
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Configuración' }));
    await waitFor(() => expect(manageIntegration).toHaveBeenCalledWith(expect.any(String), { kind: 'refresh' }));
    const refreshRequestId = vi.mocked(manageIntegration).mock.calls[0]?.[0];
    expect(refreshRequestId).toEqual(expect.any(String));

    await fireEvent.click(screen.getByRole('button', { name: 'Apps de IA: Disponible' }));
    const genericMcpArticle = (await screen.findByText('Cliente MCP genérico')).closest('article');
    expect(genericMcpArticle).not.toBeNull();
    const connectButton = within(genericMcpArticle as HTMLElement).getByRole('button', { name: 'Conectar' });
    expect(connectButton).toBeDisabled();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      integrationRequestId: null,
      integrationCompletedRequestId: null
    };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot
      });
    });
    expect(connectButton).toBeDisabled();

    snapshot = { ...snapshot, sequence: snapshot.sequence + 1, integrationRequestId: refreshRequestId ?? null };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot
      });
    });
    expect(connectButton).toBeDisabled();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      integrationRequestId: null,
      integrationCompletedRequestId: refreshRequestId ?? null
    };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot
      });
    });
    expect(connectButton).toBeEnabled();
  });

  it('releases integration controls when dispatch fails after an active snapshot', async () => {
    let rejectIntegration: ((error: Error) => void) | null = null;
    vi.mocked(manageIntegration).mockImplementationOnce(() => new Promise<void>((_resolve, reject) => {
      rejectIntegration = reject;
    }));
    snapshot.integrations = {
      externalAiWikiCount: 0,
      integrations: [{
        client: 'genericMcp', status: 'available', detectedVersion: null,
        activityRecent: false, restartRequired: false, mcpSetup: null,
        workflowGuide: { kind: 'mcpInstructions', status: 'builtIn', version: '1', restartRequired: true }
      }]
    };
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Configuración' }));
    await waitFor(() => expect(manageIntegration).toHaveBeenCalledWith(expect.any(String), { kind: 'refresh' }));
    const refreshRequestId = vi.mocked(manageIntegration).mock.calls[0]?.[0];
    expect(refreshRequestId).toEqual(expect.any(String));

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      integrationRequestId: refreshRequestId ?? null
    };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot
      });
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Apps de IA: Disponible' }));
    const genericMcpArticle = (await screen.findByText('Cliente MCP genérico')).closest('article');
    expect(genericMcpArticle).not.toBeNull();
    const connectButton = within(genericMcpArticle as HTMLElement).getByRole('button', { name: 'Conectar' });
    expect(connectButton).toBeDisabled();

    if (!rejectIntegration) throw new Error('integration rejection was not captured');
    await act(async () => {
      rejectIntegration?.(new Error('synthetic dispatch failure'));
      await Promise.resolve();
    });
    expect(connectButton).toBeEnabled();
  });

  it('opens device preferences from disabled local-network guidance', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Conexiones: Solo este dispositivo' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Preferencias del dispositivo' }));

    expect(await screen.findByRole('heading', { name: 'Configuración', level: 1 })).toBeInTheDocument();
    expect(screen.getByRole('combobox', { name: 'Red local' })).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: 'Conexiones' })).not.toBeInTheDocument();
    expect(window.location.hash).toBe('#system/preferences');
  });

  it('offers the closed Windows firewall action only when the helper is verified', async () => {
    snapshot.preferences!.lanPreference = 'enabled';
    snapshot.connectivity = { systemPermission: 'notApplicable', networkProfile: 'private', firewall: 'rulesMissing', firewallHelper: 'verified' };
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Conexiones: Solo este dispositivo' }));

    await fireEvent.click(screen.getByRole('button', { name: 'Configurar firewall…' }));
    expect(configureFirewall).toHaveBeenCalledWith(expect.any(String), true);

    cleanup();
    snapshot.connectivity.networkProfile = 'public';
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Conexiones: Solo este dispositivo' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Abrir configuración de red' }));
    expect(openSystemDestination).toHaveBeenCalledWith(expect.any(String), 'networkSettings');
  });

  it('explains a rejected pairing and requires an explicit safe retry', async () => {
    snapshot.preferences!.lanPreference = 'enabled';
    snapshot.peers = [{
      peerId: '12D3KooBlockedSyntheticPeer', deviceName: 'Office PC', address: '',
      trust: 'blocked', activity: 'discovered', sasWords: null, grantedWikiIds: []
    }];
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Conexiones: Solo este dispositivo' }));

    expect(screen.getByText('Verificación bloqueada')).toBeInTheDocument();
    expect(screen.getByText(/Los códigos no coincidieron/)).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Permitir volver a verificar' }));

    expect(allowPeerPairingAgain).toHaveBeenCalledWith('12D3KooBlockedSyntheticPeer');
  });

  it('keeps modal focus inside connections when advanced disclosures are closed', async () => {
    const { container } = render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Conexiones: Solo este dispositivo' }));

    const shell = container.querySelector('.drive-shell');
    const closeButton = screen.getByRole('button', { name: 'Cerrar' });
    const advancedSummary = screen.getByText('Detalles avanzados');
    expect((shell as HTMLElement & { inert: boolean }).inert).toBe(true);
    expect(shell).toHaveAttribute('aria-hidden', 'true');
    await waitFor(() => expect(closeButton).toHaveFocus());
    await fireEvent.keyDown(window, { key: 'Tab', shiftKey: true });
    expect(advancedSummary).toHaveFocus();
    await fireEvent.keyDown(window, { key: 'Tab' });
    expect(closeButton).toHaveFocus();
    await fireEvent.click(closeButton);
    expect((shell as HTMLElement & { inert: boolean }).inert).toBe(false);
    expect(shell).not.toHaveAttribute('aria-hidden');
  });

  it('makes the wiki source chooser modal and restores focus when Escape closes it', async () => {
    const { container } = render(App);
    const newWikiButton = await screen.findByRole('button', { name: 'Nueva wiki' });
    newWikiButton.focus();
    await fireEvent.click(newWikiButton);

    const shell = container.querySelector('.drive-shell');
    const folderChoice = screen.getByRole('button', { name: /Desde una carpeta/ });
    expect((shell as HTMLElement & { inert: boolean }).inert).toBe(true);
    await waitFor(() => expect(folderChoice).toHaveFocus());
    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(screen.queryByRole('dialog', { name: '¿De dónde viene esta wiki?' })).not.toBeInTheDocument();
    expect((shell as HTMLElement & { inert: boolean }).inert).toBe(false);
    await waitFor(() => expect(newWikiButton).toHaveFocus());
  });

  it('keeps the OKF import confirmation modal and focuses its name field', async () => {
    vi.mocked(pickOkfImport).mockResolvedValue({ token: 'okf-selection', displayName: 'Portable OKF' });
    vi.mocked(validateOkfImport).mockResolvedValue({
      entryCount: 2,
      conceptCount: 1,
      uncompressedBytes: 1024,
      declaredOkfVersion: '0.2',
      compatibility: { kind: 'declaredV02' },
      warningCount: 0,
      warnings: [],
      restrictions: []
    });
    const { container } = render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Nueva wiki' }));
    await fireEvent.click(screen.getByRole('button', { name: /Importar carpeta OKF/ }));

    const dialog = await screen.findByRole('dialog', { name: 'Revisa el bundle antes de importarlo' });
    const nameField = within(dialog).getByRole('textbox', { name: 'Nombre de la wiki' });
    const shell = container.querySelector('.drive-shell');
    expect((shell as HTMLElement & { inert: boolean }).inert).toBe(true);
    await waitFor(() => expect(nameField).toHaveFocus());
    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(screen.queryByRole('dialog', { name: 'Revisa el bundle antes de importarlo' })).not.toBeInTheDocument();
    expect((shell as HTMLElement & { inert: boolean }).inert).toBe(false);
  });

  it('moves the focus trap to the close confirmation above an open drawer', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Conexiones: Solo este dispositivo' }));
    await waitFor(() => expect(tauriListeners.has('close-choice-required')).toBe(true));

    await act(() => {
      tauriListeners.get('close-choice-required')?.({ payload: null });
    });

    const hideButton = await screen.findByRole('button', { name: 'Ocultar en bandeja' });
    await waitFor(() => expect(hideButton).toHaveFocus());
    expect(screen.getByRole('dialog', { name: 'Conexiones', hidden: true })).toHaveAttribute('aria-modal', 'true');
    expect(hideButton.closest('.close-confirmation-backdrop')).not.toBeNull();

    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: '¿Qué debe pasar al cerrar?' })).not.toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: 'Conexiones' })).toBeInTheDocument();
  });

  it('restores focus after cancelling a standalone close confirmation', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
    render(App);
    const settingsButton = await screen.findByRole('button', { name: 'Configuración' });
    settingsButton.focus();
    await waitFor(() => expect(tauriListeners.has('close-choice-required')).toBe(true));

    await act(() => {
      tauriListeners.get('close-choice-required')?.({ payload: null });
    });
    await fireEvent.click(await screen.findByRole('button', { name: 'Cancelar' }));

    await waitFor(() => expect(settingsButton).toHaveFocus());
  });

  it('opens a wiki as an independent page and requests its OKF bundle', async () => {
    render(App);
    const wikiButton = await screen.findByRole('row', { name: /Atlas 2 publicados/ });
    expect(wikiButton).not.toBeNull();
    await fireEvent.click(wikiButton);

    expect(loadWikiBundle).toHaveBeenCalledWith(snapshot.wikis[0].id);
    expect(window.location.hash).toBe(`#wikis/${snapshot.wikis[0].id}`);
    expect(await screen.findByRole('tab', { name: 'Contenido' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Pendientes/ })).toBeInTheDocument();
  });

  it('opens device grants from the wiki access summary', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('row', { name: /Atlas 2 publicados/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Gestionar acceso' }));

    expect(screen.getByRole('dialog', { name: 'Conexiones' })).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: 'Atlas' })).not.toBeInTheDocument();
  });

  it('keeps wiki details and sharing as separate actions', async () => {
    const { container } = render(App);
    await fireEvent.click(await screen.findByRole('row', { name: /Atlas 2 publicados/ }));

    const detailsButton = screen.getByRole('button', { name: 'Detalles' });
    detailsButton.focus();
    await fireEvent.click(detailsButton);
    expect(screen.getByRole('dialog', { name: 'Atlas' })).toHaveTextContent('Estado de la fuente');
    expect(screen.queryByText('Equipos cercanos')).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Cerrar' })).toHaveFocus());
    await fireEvent.keyDown(window, { key: 'Tab', shiftKey: true });
    expect(screen.getByText('Detalles avanzados')).toHaveFocus();
    let results = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(results.violations.filter((violation) => violation.impact === 'critical' || violation.impact === 'serious')).toEqual([]);
    await fireEvent.click(screen.getByRole('button', { name: 'Cerrar' }));
    await waitFor(() => expect(detailsButton).toHaveFocus());

    const shareButton = screen.getByRole('button', { name: 'Compartir' });
    shareButton.focus();
    await fireEvent.click(shareButton);
    expect(screen.getByRole('dialog', { name: 'Atlas' })).toHaveTextContent('Equipos cercanos');
    expect(screen.queryByText('Documentos de origen')).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Cerrar' })).toHaveFocus());
    results = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(results.violations.filter((violation) => violation.impact === 'critical' || violation.impact === 'serious')).toEqual([]);
  });

  it('distinguishes maintenance from an empty source-issue state', async () => {
    snapshot.wikis[0].maintenanceRequired = true;
    render(App);
    await fireEvent.click(await screen.findByRole('row', { name: /Atlas 2 publicados/ }));
    await fireEvent.click(screen.getAllByRole('button', { name: 'Detalles' }).at(-1)!);

    expect(screen.getByText('El contenido publicado necesita una comprobación')).toBeInTheDocument();
    expect(screen.queryByText('No hay problemas con la fuente')).not.toBeInTheDocument();
  });

  it('carries a maintenance reason and its safe next action into the opened wiki', async () => {
    const wiki = snapshot.wikis[0];
    wiki.maintenanceRequired = true;
    snapshot.wikiHealth!.attentionWikiId = wiki.id;
    const { container } = render(App);

    expect(await screen.findByText('Comprobar contenido publicado')).toBeInTheDocument();
    await fireEvent.click(screen.getByText('Ver qué hacer'));

    expect(screen.getByRole('heading', { name: 'Qué necesita de ti' })).toBeInTheDocument();
    expect(screen.getByText('Comprueba el contenido publicado')).toBeInTheDocument();
    expect(screen.getByText(/Revisa exactamente qué cambiaría antes de confirmar/)).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Revisar reparación segura…' }));
    expect(prepareGuidedWikiRepair).toHaveBeenCalledWith(wiki.id);
    const results = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(results.violations.filter((violation) => violation.impact === 'critical' || violation.impact === 'serious')).toEqual([]);
  });

  it('explains skipped source files and translates their safe issue code', async () => {
    const wiki = snapshot.wikis[0];
    wiki.failedCount = 1;
    snapshot.sourceIssues = [{
      wikiId: wiki.id,
      wikiName: wiki.name,
      sourceName: 'manual.pdf',
      code: 'EncryptedPdf'
    }];
    render(App);

    expect(await screen.findByText('1 archivo necesita corrección')).toBeInTheDocument();
    await fireEvent.click(screen.getByText('Ver qué hacer'));
    expect(screen.getByText('Corrige 1 archivo de la carpeta')).toBeInTheDocument();
    expect(screen.getByText(/no se reemplazará con resultados incompletos/)).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Ver archivos y solución' }));
    expect(screen.getByRole('dialog', { name: wiki.name })).toHaveTextContent('El PDF está cifrado');
    expect(screen.getByRole('dialog', { name: wiki.name })).toHaveTextContent('Guarda una copia sin contraseña en la carpeta de origen');
    expect(screen.queryByText('EncryptedPdf')).not.toBeInTheDocument();
  });

  it('states that pending AI proposals remain unpublished until a person decides', async () => {
    const wiki = snapshot.wikis[0];
    wiki.needsReviewCount = 2;
    render(App);

    await fireEvent.click(await screen.findByText('Ver qué hacer'));
    expect(screen.getByText('Revisa 2 propuestas')).toBeInTheDocument();
    expect(screen.getByText(/No se publicarán hasta que revises la evidencia y decidas/)).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Revisar propuestas' }));
    expect(screen.getByRole('tab', { name: /Pendientes/ })).toHaveAttribute('aria-selected', 'true');
  });

  it('keeps legacy review cleanup available without offering an impossible publication', async () => {
    const wiki = snapshot.wikis[0];
    wiki.needsReviewCount = 1;
    wiki.okfVersion = '0.1';
    wiki.declaredOkfVersion = '0.1';
    wiki.okfCompatibility = { kind: 'legacyV01' };
    wiki.restrictions = ['legacyReadOnly'];
    const review = {
      conceptId: 'legacy-review', wikiId: wiki.id, sourceRevision: 2,
      sourceName: 'legacy.md', wikiName: wiki.name,
      draft: {
        type: 'Reference', title: 'Legacy proposal', description: 'Synthetic fixture.',
        language: 'es', tags: [], entities: [], links: [], summary: 'Cannot be republished as v0.1.',
        classificationConfidence: 1, classificationExplanation: 'Synthetic fixture.'
      }
    };
    snapshot.reviews = [review];
    snapshot.reviewEvidence = {
      requestId: 'legacy-evidence', conceptId: review.conceptId, sourceRevision: review.sourceRevision,
      status: 'ready', excerpts: [{ ordinal: 0, headingOrPage: 'Legacy', text: 'Synthetic evidence.', truncated: false }],
      totalChunks: 1, nextOrdinal: null
    };
    render(App);

    await fireEvent.click(await screen.findByRole('row', { name: /Atlas 2 publicados/ }));
    await fireEvent.click(screen.getByRole('tab', { name: /Pendientes/ }));
    expect(await screen.findByRole('dialog', { name: 'legacy.md' })).toBeInTheDocument();

    expect(screen.getByRole('status')).toHaveTextContent('Vuelve a crearla desde la carpeta de origen');
    expect(screen.getByRole('button', { name: 'Aprobar y publicar' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Volver a analizar' })).toBeDisabled();
    await fireEvent.click(screen.getByRole('button', { name: 'Rechazar borrador' }));
    expect(rejectReview).toHaveBeenCalledWith(review.conceptId);
    expect(approveReview).not.toHaveBeenCalled();
    expect(reanalyzeReview).not.toHaveBeenCalled();
  });

  it('presents restricted OKF wikis as local read-only without impossible actions', async () => {
    const wiki = snapshot.wikis[0];
    wiki.origin = 'folder';
    wiki.indexingMode = 'manual';
    wiki.okfVersion = '0.1';
    wiki.declaredOkfVersion = '0.1';
    wiki.okfCompatibility = { kind: 'legacyV01' };
    wiki.restrictions = ['legacyReadOnly'];
    render(App);

    await fireEvent.click(await screen.findByRole('row', { name: /Atlas 2 publicados/ }));

    expect(screen.getByText(/AirWiki detuvo su indexación y uso compartido/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Actualizar' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Compartir' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Gestionar acceso' })).not.toBeInTheDocument();
    await fireEvent.click(screen.getAllByRole('button', { name: 'Detalles' }).at(-1)!);
    expect(screen.queryByRole('checkbox', { name: 'Mantener actualizada automáticamente' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Volver a vincular carpeta' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Eliminar wiki' })).toBeInTheDocument();
  });

  it('keeps guided repair reachable from the unified wiki workspace', async () => {
    const wiki = snapshot.wikis[0];
    wiki.maintenanceRequired = true;
    snapshot.wikiHealth!.attentionWikiId = wiki.id;
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Revisar reparación segura…' }));
    expect(prepareGuidedWikiRepair).toHaveBeenCalledWith(wiki.id);
  });

  it('opens a local search result inside its wiki without placing the query in the URL', async () => {
    const wiki = snapshot.wikis[0];
    const conceptId = 'concept-atlas';
    activateLocalSearch();
    snapshot.search = {
      requestId: 'search-fixture', status: 'complete', coverage: 'complete',
      hits: [{ conceptId, wikiId: wiki.id, title: 'Evidencia Atlas', snippet: 'Contenido verificado.', headingOrPage: 'Atlas', logicalResourceUri: 'urn:airwiki:fixture', sourceRevision: 1, sourceSha256: '0123456789abcdef', rank: 1, nodeId: snapshot.nodeId!, assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false }, lifecycle: 'stable' }]
    };
    snapshot.knowledge = {
      wikiId: wiki.id, wikiName: wiki.name, version: 'search-fixture', status: 'ready', errorCount: 0, warningCount: 0,
      concepts: [{ conceptId, page: { kind: 'concept', path: 'guides/atlas.md' }, title: 'Evidencia Atlas', description: 'Contenido verificado.', conceptType: 'Reference', tags: [], lifecycle: 'stable', generatedBy: 'airwiki/test', verifiedBy: ['human:test'], sources: [], staleAfter: null, assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false }, warnings: [], executionAvailable: false, fingerprint: 'a'.repeat(64) }],
      links: []
    };
    window.location.hash = '#search';
    render(App);
    expect(await screen.findByText('Revisado por una persona')).toBeInTheDocument();
    await fireEvent.click((await screen.findAllByRole('button', { name: 'Abrir' }))[0]);

    expect(loadWikiBundle).toHaveBeenCalledWith(wiki.id);
    expect(loadWikiPage).toHaveBeenCalledWith(wiki.id, { kind: 'concept', path: 'guides/atlas.md' });
    expect(window.location.hash).toBe(`#wikis/${wiki.id}`);
    expect(window.location.hash).not.toContain('Evidencia');
  });

  it('labels a trusted peer result as nearby instead of public', async () => {
    const peerId = '12D3KooSyntheticNearbyNode';
    activateLocalSearch();
    snapshot.peers = [{
      peerId, deviceName: 'RUSTICO', address: '/ip4/192.0.2.1/tcp/4242',
      trust: 'trusted', activity: 'connected', sasWords: null, grantedWikiIds: []
    }];
    snapshot.search = {
      requestId: 'nearby-search', status: 'complete', coverage: 'complete',
      hits: [{ conceptId: 'nearby-concept', wikiId: 'nearby-wiki', title: 'Evidencia cercana', snippet: 'Contenido autorizado.', headingOrPage: 'Responsable', logicalResourceUri: 'urn:airwiki:nearby', sourceRevision: 1, sourceSha256: '0123456789abcdef', rank: 1, nodeId: peerId, assurance: null, lifecycle: null }]
    };
    window.location.hash = '#search';
    render(App);

    const article = (await screen.findByRole('heading', { name: 'Evidencia cercana' })).closest('article');
    expect(article).not.toBeNull();
    const nearbyResult = within(article as HTMLElement);
    expect(nearbyResult.getByText('RUSTICO · Responsable')).toBeInTheDocument();
    expect(nearbyResult.getByText('Equipo cercano')).toBeInTheDocument();
    expect(nearbyResult.queryByText('Red pública')).not.toBeInTheDocument();
    expect(nearbyResult.queryByRole('button', { name: 'Abrir' })).not.toBeInTheDocument();
  });

  it('never shows a completed empty search together with a stale progress message', async () => {
    activateLocalSearch();
    snapshot.search = { requestId: 'empty-search', status: 'complete', coverage: 'complete', hits: [] };
    window.location.hash = '#search';
    render(App);

    expect(await screen.findByText('No encontramos evidencia coincidente')).toBeInTheDocument();
    expect(screen.getByText('Buscamos en este equipo y en los equipos autorizados disponibles. Prueba palabras que aparezcan en el contenido publicado.')).toBeInTheDocument();
    expect(screen.queryByText('Consultando los equipos disponibles…')).not.toBeInTheDocument();
  });

  it('names every source checked by a completed public search with no matches', async () => {
    activateLocalSearch();
    snapshot.search = { requestId: 'empty-public-search', status: 'complete', coverage: 'complete', hits: [] };
    window.location.hash = '#search';
    render(App);
    await fireEvent.click(await screen.findByRole('checkbox', { name: 'Red pública' }));

    expect(screen.getByText('Buscamos en este equipo, en los equipos autorizados disponibles y en la red pública. Prueba palabras que aparezcan en el contenido publicado.')).toBeInTheDocument();
  });

  it('does not present an unavailable public search as a conclusive empty result', async () => {
    activateLocalSearch();
    snapshot.search = { requestId: 'offline-public-search', status: 'complete', coverage: 'publicNetworkOffline', hits: [] };
    window.location.hash = '#search';
    render(App);

    expect(await screen.findByText('No se pudieron consultar todas las fuentes')).toBeInTheDocument();
    expect(screen.getByText('La red pública está offline. La búsqueda local y en equipos emparejados sigue disponible.')).toBeInTheDocument();
    expect(screen.queryByText('No encontramos evidencia coincidente')).not.toBeInTheDocument();
  });

  it('shows public v2 assurance and labels legacy metadata as unavailable', async () => {
    snapshot.publicBrowse = {
      requestId: 'browse-v2', status: 'direct', publisherId: 'publisher', wikiId: 'public-wiki',
      wikiName: 'Wiki pública', description: 'Conocimiento compartido', languages: ['es'],
      okfCompatibility: { kind: 'declaredV02' }, nextCursor: null,
      concepts: [{
        conceptId: 'public-v2', conceptType: 'Reference', title: 'Concepto v2',
        description: '', language: 'es', tags: [], summary: 'Resumen público', sourceRevision: 1,
        lifecycle: 'stable', assurance: { trust: 'machineConfirmed', freshness: 'stale', verificationOutdated: false }
      }, {
        conceptId: 'public-v1', conceptType: 'Document', title: 'Concepto anterior',
        description: '', language: 'es', tags: [], summary: 'Resumen anterior', sourceRevision: 1,
        lifecycle: null, assurance: null
      }]
    };
    render(App);

    expect(await screen.findByText('OKF v0.2')).toBeInTheDocument();
    expect(screen.getByText('Confirmado por proceso · Necesita revalidación · stable')).toBeInTheDocument();
    expect(screen.getByText('Metadata de confianza no disponible (nodo anterior)')).toBeInTheDocument();
  });

  it('opens a public search result in a dedicated viewer and returns to the results', async () => {
    activateLocalSearch();
    snapshot.search = {
      requestId: 'public-search', status: 'complete', coverage: 'complete',
      hits: [{ conceptId: 'public-concept', wikiId: 'public-wiki', title: 'Resultado público', snippet: 'Evidencia pública.', headingOrPage: 'Guía', logicalResourceUri: 'urn:airwiki:public', sourceRevision: 1, sourceSha256: '0123456789abcdef', rank: 1, nodeId: '12D3KooPublicPublisher', assurance: null, lifecycle: 'stable' }]
    };
    window.location.hash = '#search';
    const { container } = render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Abrir' }));
    expect(browsePublicWiki).toHaveBeenCalledWith('12D3KooPublicPublisher', 'public-wiki');
    expect(screen.getByText('Abriendo wiki pública')).toBeInTheDocument();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      publicBrowse: {
        requestId: 'public-browse-request', status: 'direct', publisherId: '12D3KooPublicPublisher', wikiId: 'public-wiki',
        wikiName: 'Wiki pública', description: 'Bundle validado', languages: ['es'], okfCompatibility: { kind: 'declaredV02' }, nextCursor: null,
        concepts: [{ conceptId: 'public-concept', conceptType: 'Guide', title: 'Concepto público', description: '', language: 'es', tags: [], summary: 'Resumen visible', sourceRevision: 1, lifecycle: 'stable', assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false } }]
      }
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'public-browse-request', kind: 'stateChanged', snapshot });
    });

    expect(screen.getByRole('heading', { name: 'Wiki pública' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Concepto público' })).toBeInTheDocument();
    expect(screen.getByText('Conexión directa autenticada')).toBeInTheDocument();
    const accessibility = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(accessibility.violations.filter((violation) => ['critical', 'serious'].includes(violation.impact ?? ''))).toEqual([]);
    await fireEvent.click(screen.getByRole('button', { name: 'Volver a los resultados' }));
    expect(screen.getByRole('heading', { name: 'Resultado público' })).toBeInTheDocument();
  });

  it('keeps search visible but explains why it cannot run before local AI is ready', async () => {
    window.location.hash = '#search';
    render(App);

    expect(await screen.findByRole('heading', { name: 'Preparando la búsqueda local' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Preparando la búsqueda local' })).toBeDisabled();
    await fireEvent.click(screen.getByRole('button', { name: 'Ver estado de la IA local' }));
    expect(window.location.hash).toBe('#system/models');
  });

  it('keeps graph view selected when opening a graph node', async () => {
    const wiki = snapshot.wikis[0];
    snapshot.knowledge = {
      wikiId: wiki.id, wikiName: wiki.name, version: 'graph-fixture', status: 'ready', errorCount: 0, warningCount: 0,
      concepts: [{ conceptId: 'concept-atlas', page: { kind: 'concept', path: 'architecture/atlas.md' }, title: 'Atlas concept', description: 'Verified concept', conceptType: 'Reference', tags: [], lifecycle: 'stable', generatedBy: 'airwiki/test', verifiedBy: ['human:test'], sources: [], staleAfter: null, assurance: { trust: 'humanReviewed', freshness: 'notDeclared', verificationOutdated: false }, warnings: [], executionAvailable: false, fingerprint: 'a'.repeat(64) }],
      links: [{ source: { kind: 'index' }, target: { kind: 'concept', path: 'architecture/atlas.md' }, label: 'Verified concept' }]
    };
    render(App);
    const wikiButton = await screen.findByRole('row', { name: /Atlas 2 publicados/ });
    await fireEvent.click(wikiButton);
    const graphButton = screen.getByRole('button', { name: 'Grafo' });
    await fireEvent.click(graphButton);
    await fireEvent.click(await screen.findByRole('button', { name: 'Atlas concept' }));

    expect(graphButton).toHaveClass('active');
    expect(loadWikiPage).toHaveBeenCalledWith(wiki.id, { kind: 'concept', path: 'architecture/atlas.md' });
  });

  it('offers human verification only for editable managed OKF revisions', async () => {
    const wiki = snapshot.wikis[0];
    wiki.origin = 'importedOkf';
    const fingerprint = 'a'.repeat(64);
    snapshot.knowledge = {
      wikiId: wiki.id, wikiName: wiki.name, version: 'managed-fixture', status: 'ready', errorCount: 0, warningCount: 0,
      concepts: [{ conceptId: 'managed-concept', page: { kind: 'concept', path: 'memory/decision.md' }, title: 'Decision', description: 'Unverified decision', conceptType: 'Decision', tags: [], lifecycle: 'stable', generatedBy: 'codex/1', verifiedBy: [], sources: [], staleAfter: null, assurance: { trust: 'unverified', freshness: 'notDeclared', verificationOutdated: false }, warnings: [], executionAvailable: false, fingerprint }],
      links: []
    };
    snapshot.knowledgePage = {
      wikiId: wiki.id, page: { kind: 'concept', path: 'memory/decision.md' }, concept: snapshot.knowledge.concepts[0], title: 'Decision', status: 'ready', blocks: [], metadata: [], backlinks: [], truncated: false
    };
    window.location.hash = `#wikis/${wiki.id}`;
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Marcar como revisado por una persona' }));

    expect(verifyWikiConcept).toHaveBeenCalledWith(wiki.id, 'memory/decision.md', fingerprint);
    expect(loadWikiBundle).toHaveBeenCalledWith(wiki.id);
  });

  it('keeps concept assurance atomic with the loaded page', async () => {
    const wiki = snapshot.wikis[0];
    const first = {
      conceptId: 'first', page: { kind: 'concept' as const, path: 'first.md' }, title: 'First',
      description: '', conceptType: 'Reference', tags: [], lifecycle: 'stable', generatedBy: 'process:first',
      verifiedBy: ['human:first'], sources: [], staleAfter: null,
      assurance: { trust: 'humanReviewed' as const, freshness: 'fresh' as const, verificationOutdated: false },
      warnings: [], executionAvailable: false, fingerprint: 'a'.repeat(64)
    };
    const second = {
      ...first, conceptId: 'second', page: { kind: 'concept' as const, path: 'second.md' }, title: 'Second',
      generatedBy: null, verifiedBy: [], assurance: { trust: 'unverified' as const, freshness: 'notDeclared' as const, verificationOutdated: false },
      fingerprint: 'b'.repeat(64)
    };
    snapshot.knowledge = {
      wikiId: wiki.id, wikiName: wiki.name, version: 'atomic-assurance', status: 'ready',
      concepts: [first, second], links: [], errorCount: 0, warningCount: 0
    };
    snapshot.knowledgePage = {
      wikiId: wiki.id, page: first.page, concept: first, title: first.title,
      status: 'ready', blocks: [{ kind: 'paragraph', text: 'First body' }], metadata: [], backlinks: [], truncated: false
    };
    window.location.hash = `#wikis/${wiki.id}`;
    render(App);

    expect(await screen.findByRole('heading', { name: 'First' })).toBeInTheDocument();
    expect(screen.getByText('Revisado por una persona')).toBeInTheDocument();
    expect(screen.getByText('process:first')).toBeInTheDocument();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      knowledgePage: {
        wikiId: wiki.id, page: second.page, concept: second, title: second.title,
        status: 'ready', blocks: [{ kind: 'paragraph', text: 'Second body' }], metadata: [], backlinks: [], truncated: false
      }
    };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot
      });
    });

    expect(screen.getByRole('heading', { name: 'Second' })).toBeInTheDocument();
    expect(screen.getByText('Reference')).toBeInTheDocument();
    expect(screen.getByText('Sin verificar')).toBeInTheDocument();
    expect(screen.queryByText('Revisado por una persona')).not.toBeInTheDocument();
    expect(screen.queryByText('process:first')).not.toBeInTheDocument();
  });

  it('uses independent settings pages that always return to the top', async () => {
    const scrollTo = vi.spyOn(HTMLElement.prototype, 'scrollTo');
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Configuración' }));
    expect(window.location.hash).toBe('#system/preferences');
    await fireEvent.click(screen.getByRole('link', { name: 'IA local' }));
    expect(window.location.hash).toBe('#system/models');
    await waitFor(() => expect(scrollTo).toHaveBeenCalledWith({ top: 0, left: 0, behavior: 'auto' }));
  });

  it('lets users change the local-network preference after onboarding', async () => {
    window.location.hash = '#system/preferences';
    render(App);

    const networkPreference = await screen.findByRole('combobox', { name: 'Red local' });
    await fireEvent.change(networkPreference, { target: { value: 'enabled' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar preferencias' }));

    expect(updatePreferences).toHaveBeenCalledWith(expect.objectContaining({ lanPreference: 'enabled' }));
  });

  it('preserves unsaved preferences while background snapshots arrive', async () => {
    window.location.hash = '#system/preferences';
    render(App);

    const networkPreference = await screen.findByRole('combobox', { name: 'Red local' });
    await fireEvent.change(networkPreference, { target: { value: 'enabled' } });
    const staleSnapshot = structuredClone(snapshot);
    staleSnapshot.sequence += 1;
    await act(() => {
      snapshotListener?.({
        schemaVersion: staleSnapshot.schemaVersion,
        sequence: staleSnapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot: staleSnapshot
      });
    });

    expect(networkPreference).toHaveValue('enabled');
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar preferencias' }));
    expect(updatePreferences).toHaveBeenCalledWith(expect.objectContaining({ lanPreference: 'enabled' }));
  });

  it('shows an explicit local-network choice for existing undecided preferences', async () => {
    snapshot.preferences!.lanPreference = 'undecided';
    window.location.hash = '#system/preferences';
    render(App);

    expect(await screen.findByRole('combobox', { name: 'Red local' })).toHaveValue('undecided');
    expect(screen.getByRole('option', { name: 'Preguntar antes de habilitar' })).toBeInTheDocument();
  });

  it('redirects previous top-level routes without retaining the old UI', async () => {
    window.location.hash = '#library';
    render(App);
    expect(await screen.findByRole('heading', { name: 'Wikis' })).toBeInTheDocument();
    cleanup();
    window.location.hash = '#review';
    render(App);
    expect(await screen.findByRole('heading', { name: 'Wikis' })).toBeInTheDocument();
  });

  it('supports local navigation shortcuts and platform theming', async () => {
    snapshot.platform = 'windows';
    snapshot.preferences!.theme = 'dark';
    render(App);
    await screen.findAllByText('Atlas');
    await fireEvent.keyDown(window, { key: '1', ctrlKey: true });
    expect(window.location.hash).toBe('#wikis');
    expect(document.documentElement.dataset.platform).toBe('windows');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it.each(accessibilityCases)('has no serious or critical accessibility violations in %s/%s/%s', async (locale, theme, route) => {
    snapshot.preferences!.locale = locale;
    snapshot.preferences!.theme = theme;
    window.location.hash = `#${route}`;
    const { container } = render(App);
    await screen.findByRole('search');
    const results = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(results.violations.filter((violation) => violation.impact === 'critical' || violation.impact === 'serious')).toEqual([]);
  });
});

import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { allowPeerPairingAgain, configureFirewall, loadWikiBundle, loadWikiPage, openSystemDestination, prepareGuidedWikiRepair, updatePreferences } from './api';
import { readySnapshot } from './test/fixtures';

let snapshot = readySnapshot();
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
    connect: vi.fn(async () => snapshot),
    refreshAutostart: vi.fn(async () => undefined),
    refreshConnectivity: vi.fn(async () => undefined),
    refreshWikiHealth: vi.fn(async () => undefined),
    prepareGuidedWikiRepair: vi.fn(async () => 'repair-request'),
    updatePreferences: vi.fn(async () => undefined),
    configureFirewall: vi.fn(async () => undefined),
    openSystemDestination: vi.fn(async () => undefined),
    manageIntegration: vi.fn(async () => undefined),
    allowPeerPairingAgain: vi.fn(async () => undefined),
    loadWikiBundle: vi.fn(async () => undefined),
    loadWikiPage: vi.fn(async () => undefined)
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

  it('gives advanced connection sections distinct names', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Conexiones: Solo este dispositivo' }));

    expect(screen.getByRole('dialog', { name: 'Conexiones' })).toBeInTheDocument();
    expect(screen.getByText('Controles de red')).toBeInTheDocument();
    expect(screen.getAllByText('Detalles avanzados')).toHaveLength(1);
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
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Conexiones: Solo este dispositivo' }));

    const closeButton = screen.getByRole('button', { name: 'Cerrar' });
    const advancedSummary = screen.getByText('Detalles avanzados');
    await waitFor(() => expect(closeButton).toHaveFocus());
    await fireEvent.keyDown(window, { key: 'Tab', shiftKey: true });
    expect(advancedSummary).toHaveFocus();
    await fireEvent.keyDown(window, { key: 'Tab' });
    expect(closeButton).toHaveFocus();
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
    await fireEvent.click(screen.getByRole('button', { name: 'Detalles' }));

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
      hits: [{ conceptId, wikiId: wiki.id, title: 'Evidencia Atlas', snippet: 'Contenido verificado.', headingOrPage: 'Atlas', logicalResourceUri: 'urn:airwiki:fixture', sourceRevision: 1, sourceSha256: '0123456789abcdef', rank: 1, nodeId: snapshot.nodeId! }]
    };
    window.location.hash = '#search';
    render(App);
    await fireEvent.click((await screen.findAllByRole('button', { name: 'Abrir' }))[0]);

    expect(loadWikiBundle).toHaveBeenCalledWith(wiki.id);
    expect(loadWikiPage).toHaveBeenCalledWith(wiki.id, { kind: 'concept', id: conceptId });
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
      hits: [{ conceptId: 'nearby-concept', wikiId: 'nearby-wiki', title: 'Evidencia cercana', snippet: 'Contenido autorizado.', headingOrPage: 'Responsable', logicalResourceUri: 'urn:airwiki:nearby', sourceRevision: 1, sourceSha256: '0123456789abcdef', rank: 1, nodeId: peerId }]
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
      concepts: [{ page: { kind: 'concept', id: 'concept-atlas' }, title: 'Atlas concept', description: 'Verified concept', conceptType: 'Reference', tags: [] }],
      links: [{ source: { kind: 'index' }, target: { kind: 'concept', id: 'concept-atlas' }, label: 'Verified concept' }]
    };
    render(App);
    const wikiButton = await screen.findByRole('row', { name: /Atlas 2 publicados/ });
    await fireEvent.click(wikiButton);
    const graphButton = screen.getByRole('button', { name: 'Grafo' });
    await fireEvent.click(graphButton);
    await fireEvent.click(await screen.findByRole('button', { name: 'Atlas concept' }));

    expect(graphButton).toHaveClass('active');
    expect(loadWikiPage).toHaveBeenCalledWith(wiki.id, { kind: 'concept', id: 'concept-atlas' });
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

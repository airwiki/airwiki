import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { configureFirewall, loadWikiBundle, loadWikiPage, openSystemDestination, prepareGuidedWikiRepair, updatePreferences } from './api';
import { readySnapshot } from './test/fixtures';

let snapshot = readySnapshot();
const tauriListeners = new Map<string, (event: unknown) => void>();
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

  it('shows real service states in the sidebar without treating disabled services as healthy', async () => {
    render(App);

    expect(await screen.findByRole('button', { name: 'IA local: Sin configurar' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Red local: Opcional · Desactivado' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Red pública: Opcional · Desactivado' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'MCP: Disponible' })).toBeInTheDocument();
  });

  it('gives advanced connection sections distinct names', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Red local: Opcional · Desactivado' }));

    expect(screen.getByRole('dialog', { name: 'Conexiones' })).toBeInTheDocument();
    expect(screen.getByText('Controles de red')).toBeInTheDocument();
    expect(screen.getAllByText('Detalles avanzados')).toHaveLength(1);
  });

  it('opens device preferences from disabled local-network guidance', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Red local: Opcional · Desactivado' }));
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
    await fireEvent.click(await screen.findByRole('button', { name: 'Red local: Opcional · Desactivado' }));

    await fireEvent.click(screen.getByRole('button', { name: 'Configurar firewall…' }));
    expect(configureFirewall).toHaveBeenCalledWith(expect.any(String), true);

    cleanup();
    snapshot.connectivity.networkProfile = 'public';
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Red local: Opcional · Desactivado' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Abrir configuración de red' }));
    expect(openSystemDestination).toHaveBeenCalledWith(expect.any(String), 'networkSettings');
  });

  it('keeps modal focus inside connections when advanced disclosures are closed', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Red local: Opcional · Desactivado' }));

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
    await fireEvent.click(await screen.findByRole('button', { name: 'Red local: Opcional · Desactivado' }));
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

  it('never shows a completed empty search together with a stale progress message', async () => {
    snapshot.search = { requestId: 'empty-search', status: 'complete', coverage: 'complete', hits: [] };
    window.location.hash = '#search';
    render(App);

    expect(await screen.findByText('No encontramos evidencia coincidente')).toBeInTheDocument();
    expect(screen.queryByText('Consultando los equipos disponibles…')).not.toBeInTheDocument();
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

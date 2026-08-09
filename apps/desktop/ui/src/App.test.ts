import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { loadKnowledgeBundle, loadKnowledgePage } from './api';
import { readySnapshot } from './test/fixtures';

let snapshot = readySnapshot();
const accessibilityCases = (['es', 'en'] as const).flatMap((locale) =>
  (['light', 'dark'] as const).flatMap((theme) =>
    (['library', 'review', 'search', 'share/devices', 'share/integrations', 'share/connectivity', 'system/models', 'system/preferences', 'system/updates'] as const)
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
    manageIntegration: vi.fn(async () => undefined),
    loadKnowledgeBundle: vi.fn(async () => undefined),
    loadKnowledgePage: vi.fn(async () => undefined)
  };
});

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => undefined)
}));

describe('AirWiki desktop shell', () => {
  afterEach(() => {
    cleanup();
    delete document.documentElement.dataset.theme;
    delete document.documentElement.dataset.platform;
    document.documentElement.style.colorScheme = '';
  });

  beforeEach(() => {
    window.location.hash = '';
    snapshot = readySnapshot();
  });

  it('renders a cohesive wiki, sharing, and system navigation', async () => {
    render(App);

    expect(await screen.findByText('Atlas')).toBeInTheDocument();
    for (const destination of ['Wiki', 'Compartir', 'Sistema']) {
      expect(screen.getByRole('button', { name: destination })).toBeInTheDocument();
    }
    expect(screen.getByRole('search')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Por revisar · 0' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Agregar carpeta' })).toBeInTheDocument();
  });

  it('keeps collection loading feedback contextual instead of leaving a stale global message', async () => {
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Abrir' }));

    expect(loadKnowledgeBundle).toHaveBeenCalledOnce();
    expect(screen.queryByText(/Comprobando la salud/)).not.toBeInTheDocument();
  });

  it('keeps search available while moving between product areas', async () => {
    render(App);

    const search = await screen.findByRole('textbox', { name: 'Pregunta a tu conocimiento' });
    await fireEvent.click(screen.getByRole('button', { name: 'Compartir' }));
    expect(search).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Sistema' }));
    expect(search).toBeInTheDocument();
    await fireEvent.focus(search);
    expect(await screen.findByRole('heading', { name: 'Buscar evidencia' })).toBeInTheDocument();
  });

  it('explains the editorial path and keeps review inside the wiki', async () => {
    render(App);

    expect(await screen.findByRole('navigation', { name: 'Cómo entra el conocimiento en tu wiki' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Fuentes 3' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /IA local/ })).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Por revisar · 0' }));
    expect(await screen.findByRole('heading', { name: 'Revisión humana' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Wiki publicada' })).toBeInTheDocument();
  });

  it('groups collection, device, AI, and public access under Share', async () => {
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Compartir' }));
    expect(await screen.findByRole('heading', { name: 'Compartir empieza con conocimiento revisado' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Elige qué puede salir de este equipo' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Otros equipos' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Aplicaciones de chat' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Conexión local' })).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Actualizaciones' })).not.toBeInTheDocument();
  });

  it('opens a local search hit as a document even when the graph was selected', async () => {
    const collection = snapshot.collections.at(0);
    expect(collection).toBeDefined();
    if (!collection || !snapshot.nodeId) return;
    const conceptId = 'concept-atlas';
    snapshot.knowledge = {
      collectionId: collection.id,
      collectionName: collection.name,
      version: 'fixture-v1',
      status: 'ready',
      concepts: [{
        page: { kind: 'concept', id: conceptId },
        title: 'Evidencia Atlas',
        description: 'Evidencia sintética',
        conceptType: 'Document',
        tags: []
      }],
      links: [{ source: { kind: 'index' }, target: { kind: 'concept', id: conceptId }, label: 'contains' }],
      errorCount: 0,
      warningCount: 0
    };
    snapshot.knowledgePage = {
      collectionId: collection.id,
      page: { kind: 'concept', id: conceptId },
      title: 'Evidencia Atlas',
      status: 'ready',
      blocks: [{ kind: 'paragraph', text: 'Contenido verificado.' }],
      metadata: [],
      backlinks: [],
      truncated: false
    };
    snapshot.search = {
      requestId: 'search-fixture',
      status: 'complete',
      coverage: 'complete',
      hits: [{
        conceptId,
        collectionId: collection.id,
        title: 'Evidencia Atlas',
        snippet: 'Contenido verificado.',
        headingOrPage: 'Atlas',
        logicalResourceUri: 'urn:airwiki:fixture',
        sourceRevision: 1,
        sourceSha256: '0123456789abcdef',
        rank: 1,
        nodeId: snapshot.nodeId
      }]
    };
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Abrir' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Grafo' }));
    await fireEvent.focus(screen.getByRole('textbox', { name: 'Pregunta a tu conocimiento' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Abrir evidencia local' }));

    expect(loadKnowledgePage).toHaveBeenCalledWith(collection.id, { kind: 'concept', id: conceptId });
    expect(await screen.findByText('Contenido verificado.')).toBeInTheDocument();
  });

  it('keeps system actions reachable from keyboard navigation', async () => {
    render(App);
    const system = await screen.findByRole('button', { name: 'Sistema' });
    await fireEvent.click(system);

    expect(await screen.findByRole('link', { name: 'Preferencias del dispositivo' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('button', { name: 'Guardar preferencias' })).toBeEnabled();
  });

  it('renders System subsections as independent pages', async () => {
    const scrollTo = vi.spyOn(HTMLElement.prototype, 'scrollTo');
    const pushState = vi.spyOn(window.history, 'pushState');
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Sistema' }));
    await waitFor(() => expect(scrollTo).toHaveBeenCalledWith({
      top: 0, left: 0, behavior: 'auto'
    }));
    expect(window.location.hash).toBe('#system/preferences');
    expect(pushState).toHaveBeenCalledTimes(1);
    await fireEvent.click(screen.getByRole('button', { name: 'Sistema' }));
    expect(pushState).toHaveBeenCalledTimes(1);
    expect(document.getElementById('system-preferences')).toBeInTheDocument();
    expect(document.getElementById('system-models')).not.toBeInTheDocument();
    expect(document.getElementById('system-updates')).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('link', { name: 'Actualizaciones' }));
    await waitFor(() => expect(scrollTo).toHaveBeenLastCalledWith({
      top: 0, left: 0, behavior: 'auto'
    }));
    expect(window.location.hash).toBe('#system/updates');
    expect(document.getElementById('system-preferences')).not.toBeInTheDocument();
    expect(document.getElementById('system-updates')).toBeInTheDocument();
    expect(pushState).toHaveBeenCalledTimes(2);
    await fireEvent.click(screen.getByRole('link', { name: 'Actualizaciones' }));
    expect(pushState).toHaveBeenCalledTimes(2);

    await fireEvent.click(screen.getByRole('button', { name: 'Wiki' }));
    await waitFor(() => expect(scrollTo).toHaveBeenLastCalledWith({
      top: 0, left: 0, behavior: 'auto'
    }));
    expect(await screen.findByRole('heading', { name: 'Tu wiki verificada' })).toBeInTheDocument();

    window.history.pushState(null, '', '#search');
    window.dispatchEvent(new PopStateEvent('popstate'));
    await waitFor(() => expect(scrollTo).toHaveBeenLastCalledWith({
      top: 0, left: 0, behavior: 'auto'
    }));
    expect(await screen.findByRole('heading', { name: 'Buscar evidencia' })).toBeInTheDocument();
  });

  it('shows stable installed memory when the operating system cannot estimate availability', async () => {
    const hardware = snapshot.hardware;
    expect(hardware).not.toBeNull();
    if (hardware) hardware.availableMemoryBytes = 0;
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Compartir' }));
    await fireEvent.click(screen.getByRole('link', { name: 'Otros equipos' }));

    expect(await screen.findByText('Memoria instalada')).toBeInTheDocument();
    expect(screen.getByText('16.0 GiB')).toBeInTheDocument();
    expect(screen.queryByText('0.0 GiB')).not.toBeInTheDocument();
  });

  it('renders the same primary navigation in English', async () => {
    const preferences = snapshot.preferences;
    expect(preferences).not.toBeNull();
    if (preferences) preferences.locale = 'en';
    render(App);

    expect(await screen.findByRole('button', { name: 'Wiki' })).toBeInTheDocument();
    for (const destination of ['Share', 'System']) {
      expect(screen.getByRole('button', { name: destination })).toBeInTheDocument();
    }
    expect(screen.queryByRole('button', { name: 'Compartir' })).not.toBeInTheDocument();
  });

  it('applies an explicit persisted theme to the document', async () => {
    const preferences = snapshot.preferences;
    expect(preferences).not.toBeNull();
    if (preferences) preferences.theme = 'light';
    render(App);

    await screen.findByText('Atlas');
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(document.documentElement.style.colorScheme).toBe('light');
  });

  it('applies the typed host platform and renders the contextual atlas', async () => {
    snapshot.platform = 'windows';
    render(App);

    expect(await screen.findByText('Atlas')).toBeInTheDocument();
    expect(document.documentElement.dataset.platform).toBe('windows');
    expect(screen.getByRole('complementary', { name: 'Flujo de conocimiento' })).toBeInTheDocument();
  });

  it('supports local desktop navigation shortcuts', async () => {
    render(App);
    await screen.findByRole('button', { name: 'Wiki' });

    await fireEvent.keyDown(window, { key: 'k', metaKey: true });
    expect(await screen.findByRole('heading', { name: 'Buscar evidencia' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Pregunta a tu conocimiento' })).toHaveFocus();
    await fireEvent.keyDown(window, { key: ',', metaKey: true });
    expect(await screen.findByRole('link', { name: 'Preferencias del dispositivo' })).toHaveAttribute('aria-current', 'page');
  });

  it('opens a stable System subsection from its hash route', async () => {
    const scrollTo = vi.spyOn(HTMLElement.prototype, 'scrollTo');
    window.location.hash = '#system/updates';
    render(App);

    const updates = await screen.findByRole('link', { name: 'Actualizaciones' });
    await waitFor(() => expect(scrollTo).toHaveBeenCalledWith({
      top: 0, left: 0, behavior: 'auto'
    }));
    expect(updates).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('button', { name: 'Sistema' })).toHaveClass('active');
    expect(document.getElementById('system-updates')).toBeInTheDocument();
    expect(document.getElementById('system-preferences')).not.toBeInTheDocument();
  });

  it.each(accessibilityCases)('has no serious or critical accessibility violations in %s/%s/%s', async (locale, theme, route) => {
    const preferences = snapshot.preferences;
    expect(preferences).not.toBeNull();
    if (preferences) {
      preferences.locale = locale;
      preferences.theme = theme;
    }
    window.location.hash = route;
    const { container } = render(App);
    await screen.findByRole('button', { name: 'Wiki' });
    const result = await axe.run(container, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] } });
    expect(result.violations.filter((violation: (typeof result.violations)[number]) => ['serious', 'critical'].includes(violation.impact ?? ''))).toEqual([]);
  });
});

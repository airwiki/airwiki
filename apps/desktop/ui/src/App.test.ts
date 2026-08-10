import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { loadWikiBundle, loadWikiPage } from './api';
import { readySnapshot } from './test/fixtures';

let snapshot = readySnapshot();
const accessibilityCases = (['es', 'en'] as const).flatMap((locale) =>
  (['light', 'dark'] as const).flatMap((theme) =>
    (['home', 'wikis', 'shared', 'search', 'system/models', 'system/preferences', 'system/updates'] as const)
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
    loadWikiBundle: vi.fn(async () => undefined),
    loadWikiPage: vi.fn(async () => undefined)
  };
});

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

describe('AirWiki wiki workspace', () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    delete document.documentElement.dataset.theme;
    delete document.documentElement.dataset.platform;
    document.documentElement.style.colorScheme = '';
  });

  beforeEach(() => {
    window.location.hash = '';
    snapshot = readySnapshot();
  });

  it('renders familiar primary navigation, global search, and the wiki list', async () => {
    render(App);

    expect((await screen.findAllByText('Atlas')).length).toBeGreaterThan(0);
    for (const destination of ['Inicio', 'Wikis', 'Compartido', 'Configuración']) {
      expect(screen.getByRole('button', { name: destination })).toBeInTheDocument();
    }
    expect(screen.getByRole('search')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Nueva wiki' })).toBeInTheDocument();
    expect(screen.queryByText('Biblioteca')).not.toBeInTheDocument();
  });

  it('opens a wiki as an independent page and requests its OKF bundle', async () => {
    render(App);
    const wikiButton = (await screen.findAllByText('Atlas'))[1].closest('button');
    expect(wikiButton).not.toBeNull();
    await fireEvent.click(wikiButton!);

    expect(loadWikiBundle).toHaveBeenCalledWith(snapshot.wikis[0].id);
    expect(window.location.hash).toBe(`#wikis/${snapshot.wikis[0].id}`);
    expect(await screen.findByRole('tab', { name: 'Contenido' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Pendientes/ })).toBeInTheDocument();
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

  it('uses independent settings pages that always return to the top', async () => {
    const scrollTo = vi.spyOn(HTMLElement.prototype, 'scrollTo');
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Configuración' }));
    expect(window.location.hash).toBe('#system/preferences');
    await fireEvent.click(screen.getByRole('link', { name: 'IA local' }));
    expect(window.location.hash).toBe('#system/models');
    await waitFor(() => expect(scrollTo).toHaveBeenCalledWith({ top: 0, left: 0, behavior: 'auto' }));
  });

  it('redirects previous top-level routes without retaining the old UI', async () => {
    window.location.hash = '#library';
    render(App);
    expect(await screen.findByRole('heading', { name: 'Wikis' })).toBeInTheDocument();
    cleanup();
    window.location.hash = '#review';
    render(App);
    expect(await screen.findByRole('heading', { name: 'Inicio' })).toBeInTheDocument();
  });

  it('supports local navigation shortcuts and platform theming', async () => {
    snapshot.platform = 'windows';
    snapshot.preferences!.theme = 'dark';
    render(App);
    await screen.findAllByText('Atlas');
    await fireEvent.keyDown(window, { key: '3', ctrlKey: true });
    expect(window.location.hash).toBe('#shared/owned');
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

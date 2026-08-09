import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { readySnapshot } from './test/fixtures';

let snapshot = readySnapshot();
const accessibilityCases = (['es', 'en'] as const).flatMap((locale) =>
  (['light', 'dark'] as const).flatMap((theme) =>
    (['library', 'review', 'search', 'system/models', 'system/preferences', 'system/updates', 'system/connectivity', 'system/devices', 'system/integrations'] as const)
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
    manageIntegration: vi.fn(async () => undefined)
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

  it('renders the four primary destinations and a local-first collection', async () => {
    render(App);

    expect(await screen.findByText('Atlas')).toBeInTheDocument();
    for (const destination of ['Biblioteca', 'Revisión', 'Buscar', 'Sistema']) {
      expect(screen.getByRole('button', { name: destination })).toBeInTheDocument();
    }
    expect(screen.getByRole('button', { name: 'Agregar carpeta' })).toBeInTheDocument();
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

    await fireEvent.click(screen.getByRole('button', { name: 'Biblioteca' }));
    await waitFor(() => expect(scrollTo).toHaveBeenLastCalledWith({
      top: 0, left: 0, behavior: 'auto'
    }));
    expect(await screen.findByRole('heading', { name: 'Tu conocimiento, en este equipo' })).toBeInTheDocument();

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

    await fireEvent.click(await screen.findByRole('button', { name: 'Sistema' }));
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

    expect(await screen.findByRole('button', { name: 'Library' })).toBeInTheDocument();
    for (const destination of ['Review', 'Search', 'System']) {
      expect(screen.getByRole('button', { name: destination })).toBeInTheDocument();
    }
    expect(screen.queryByRole('button', { name: 'Biblioteca' })).not.toBeInTheDocument();
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
    await screen.findByRole('button', { name: 'Biblioteca' });

    await fireEvent.keyDown(window, { key: '3', metaKey: true });
    expect(await screen.findByRole('heading', { name: 'Buscar evidencia' })).toBeInTheDocument();
    await fireEvent.keyDown(window, { key: ',', metaKey: true });
    expect(await screen.findByRole('link', { name: 'Preferencias del dispositivo' })).toHaveAttribute('aria-current', 'page');
  });

  it('opens a stable System subsection from its hash route', async () => {
    window.location.hash = '#system/updates';
    render(App);

    const updates = await screen.findByRole('link', { name: 'Actualizaciones' });
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
    await screen.findByRole('button', { name: locale === 'en' ? 'Library' : 'Biblioteca' });
    const result = await axe.run(container, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] } });
    expect(result.violations.filter((violation: (typeof result.violations)[number]) => ['serious', 'critical'].includes(violation.impact ?? ''))).toEqual([]);
  });
});

import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { readySnapshot } from './test/fixtures';

let snapshot = readySnapshot();

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

    expect(await screen.findByText('Identidad y capacidad local')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Guardar preferencias' })).toBeEnabled();
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

  it('opens a stable System subsection from its hash route', async () => {
    window.location.hash = '#system/updates';
    render(App);

    const updates = await screen.findByRole('link', { name: 'Actualizaciones' });
    expect(updates).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('button', { name: 'Sistema' })).toHaveClass('active');
    expect(document.getElementById('system-updates')).toBeInTheDocument();
  });

  it.each([
    ['es', 'light', 'library'], ['es', 'light', 'review'], ['es', 'light', 'search'], ['es', 'light', 'system/preferences'],
    ['es', 'dark', 'library'], ['es', 'dark', 'review'], ['es', 'dark', 'search'], ['es', 'dark', 'system/preferences'],
    ['en', 'light', 'library'], ['en', 'light', 'review'], ['en', 'light', 'search'], ['en', 'light', 'system/preferences'],
    ['en', 'dark', 'library'], ['en', 'dark', 'review'], ['en', 'dark', 'search'], ['en', 'dark', 'system/preferences']
  ] as const)('has no serious or critical accessibility violations in %s/%s/%s', async (locale, theme, route) => {
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

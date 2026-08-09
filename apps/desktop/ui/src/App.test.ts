import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { readySnapshot } from './test/fixtures';

const snapshot = readySnapshot();

vi.mock('./api', async (importOriginal) => {
  const original = await importOriginal<typeof import('./api')>();
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
  afterEach(cleanup);

  beforeEach(() => {
    window.location.hash = '';
  });

  it('renders the four primary destinations and a local-first collection', async () => {
    render(App);

    expect(await screen.findByText('Atlas')).toBeInTheDocument();
    for (const destination of ['Biblioteca', 'Revisión', 'Buscar', 'Sistema']) {
      expect(screen.getByRole('button', { name: destination })).toBeInTheDocument();
    }
  });

  it('keeps system actions reachable from keyboard navigation', async () => {
    render(App);
    const system = await screen.findByRole('button', { name: 'Sistema' });
    await fireEvent.click(system);

    expect(await screen.findByText('Identidad y capacidad local')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Guardar preferencias' })).toBeEnabled();
  });

  it('has no serious or critical accessibility violations in the library view', async () => {
    const { container } = render(App);
    await screen.findByText('Atlas');
    const result = await axe.run(container, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] } });
    expect(result.violations.filter((violation) => ['serious', 'critical'].includes(violation.impact ?? ''))).toEqual([]);
  });
});

import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import GlobalSearch from './GlobalSearch.svelte';
import { message } from './i18n';

describe('GlobalSearch', () => {
  afterEach(cleanup);

  it('offers an accessible route to local AI settings while search is unavailable', async () => {
    const onopenmodelsettings = vi.fn();
    render(GlobalSearch, {
      question: '',
      includePublic: false,
      busy: false,
      ready: false,
      platform: 'macOs' as const,
      privateScopeLabel: 'Este equipo',
      t: (id: string) => message('es', id),
      onquestion: vi.fn(),
      oncompositionstart: vi.fn(),
      oncompositionend: vi.fn(),
      onpublic: vi.fn(),
      onsearch: vi.fn(),
      onopen: vi.fn(),
      onopenmodelsettings
    });

    expect(screen.getByRole('button', { name: 'Preparando la búsqueda local' })).toBeDisabled();
    await fireEvent.click(screen.getByRole('button', { name: 'Ver estado de la IA local' }));
    expect(onopenmodelsettings).toHaveBeenCalledTimes(1);
  });
});

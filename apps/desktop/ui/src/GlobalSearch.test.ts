import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import GlobalSearch from './GlobalSearch.svelte';
import { message } from './i18n';

describe('GlobalSearch', () => {
  afterEach(cleanup);

  it('keeps the search bar compact and opens its context while search is unavailable', async () => {
    const onopen = vi.fn();
    render(GlobalSearch, {
      question: '',
      includePublic: false,
      busy: false,
      state: 'unavailable' as const,
      platform: 'macOs' as const,
      privateScopeLabel: 'Este equipo',
      t: (id: string) => message('es', id),
      onquestion: vi.fn(),
      oncompositionstart: vi.fn(),
      oncompositionend: vi.fn(),
      onpublic: vi.fn(),
      onsearch: vi.fn(),
      onopen
    });

    expect(screen.getByRole('button', { name: 'La búsqueda local no está disponible' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Ver estado de la IA local' })).not.toBeInTheDocument();
    const input = screen.getByRole('textbox', { name: 'Pregunta a tu conocimiento' });
    expect(input).toHaveAccessibleDescription(/Revisa el estado de la IA local/);
    await fireEvent.focus(input);
    expect(onopen).toHaveBeenCalledTimes(1);
  });
});

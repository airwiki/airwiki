import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { PublicCatalogWikiSummary } from './api';
import { message, type MessageArgs } from './i18n';
import PublicWikiTable from './PublicWikiTable.svelte';

const t = (id: string, args?: MessageArgs) => message('es', id, args);

const wiki: PublicCatalogWikiSummary = {
  publisherId: '12D3KooWpublisher',
  wikiId: '00000000-0000-4000-8000-000000000011',
  name: 'Manual comunitario',
  description: 'Prácticas de cultivo local',
  languages: ['es'],
  conceptCount: 18,
  okfCompatibility: { kind: 'declaredV02' }
};

describe('Public Wiki list', () => {
  afterEach(cleanup);

  it('shows useful profile metadata without exposing the publisher identity', () => {
    render(PublicWikiTable, { wikis: [wiki], t, onopen: vi.fn() });

    const row = screen.getByRole('button', { name: /Manual comunitario.*18 conceptos.*Red pública/ });
    expect(row).toHaveTextContent('Prácticas de cultivo local');
    expect(row).toHaveTextContent('18 conceptos');
    expect(row).toHaveTextContent('es');
    expect(row).not.toHaveTextContent(wiki.publisherId);
  });

  it('opens the selected signed profile from the full row target', async () => {
    const onopen = vi.fn();
    render(PublicWikiTable, { wikis: [wiki], t, onopen });

    await fireEvent.click(screen.getByRole('button', { name: /Manual comunitario/ }));
    expect(onopen).toHaveBeenCalledWith(wiki);
  });

  it('renders metadata values that happen to have the same visible text', () => {
    const conceptCount = t('desktop-public-wiki-concept-count', { count: 1 });
    const collidingMetadata = { ...wiki, conceptCount: 1, languages: [conceptCount] };

    render(PublicWikiTable, { wikis: [collidingMetadata], t, onopen: vi.fn() });

    expect(screen.getAllByText(conceptCount)).toHaveLength(2);
  });
});

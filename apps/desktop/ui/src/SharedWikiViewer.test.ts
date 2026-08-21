import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import SharedWikiViewer from './SharedWikiViewer.svelte';
import type { PublicBrowseSummary } from './api';

function publicWiki(overrides: Partial<PublicBrowseSummary> = {}): PublicBrowseSummary {
  return {
    requestId: 'browse-a',
    status: 'direct',
    publisherId: 'publisher-a',
    wikiId: 'wiki-a',
    wikiName: 'Shared name',
    description: null,
    languages: ['en'],
    okfCompatibility: { kind: 'declaredV02' },
    concepts: [{
      conceptId: 'concept-a',
      conceptType: 'Guide',
      title: 'First concept',
      description: '',
      language: 'en',
      tags: [],
      summary: 'First summary.',
      sourceRevision: 1,
      lifecycle: 'stable',
      assurance: null
    }],
    nextCursor: null,
    appendFailed: false,
    ...overrides
  };
}

const labels: Record<string, string> = {
  'desktop-shared-back-results': 'Back to results',
  'desktop-page-search-title': 'Search',
  'desktop-shared-access-title': 'Shared access',
  'desktop-shared-read-only': 'Read only',
  'desktop-public-direct': 'Direct',
  'desktop-wiki-sections': 'Wiki sections',
  'desktop-wiki-content-tab': 'Content',
  'desktop-okf-compatibility-declaredV02': 'OKF 0.2',
  'knowledge-pages': 'Pages',
  'desktop-shared-summary-label': 'Summary',
  'desktop-concept-assurance-title': 'Assurance',
  'desktop-concept-type': 'Type',
  'desktop-concept-trust': 'Trust',
  'desktop-shared-source': 'Source',
  'desktop-shared-summary-note': 'Published summary',
  'desktop-shared-tags': 'Tags',
  'search-public-block-publisher': 'Block publisher'
};

function translate(id: string): string {
  return labels[id] ?? id;
}

describe('SharedWikiViewer', () => {
  afterEach(cleanup);

  it('treats publisher and Wiki IDs as part of the page identity', async () => {
    const firstWiki = publicWiki();
    const { rerender } = render(SharedWikiViewer, {
      source: 'public',
      sourceName: 'Public network',
      browse: firstWiki,
      loading: false,
      t: translate,
      metadata: () => 'Unverified',
      onback: vi.fn(),
      onmore: vi.fn()
    });

    const heading = screen.getByRole('heading', { name: 'Shared name' });
    await waitFor(() => expect(heading).toHaveFocus());
    screen.getByRole('button', { name: 'Back to results' }).focus();
    expect(heading).not.toHaveFocus();

    await rerender({
      source: 'public',
      sourceName: 'Public network',
      browse: publicWiki({
        requestId: 'browse-b',
        publisherId: 'publisher-b',
        wikiId: 'wiki-b',
        concepts: [{
          conceptId: 'concept-b',
          conceptType: 'Guide',
          title: 'Second concept',
          description: '',
          language: 'en',
          tags: [],
          summary: 'Second summary.',
          sourceRevision: 1,
          lifecycle: 'stable',
          assurance: null
        }]
      }),
      loading: false,
      t: translate,
      metadata: () => 'Unverified',
      onback: vi.fn(),
      onmore: vi.fn()
    });

    await waitFor(() => expect(heading).toHaveFocus());
    expect(screen.getByText('Second summary.')).toBeInTheDocument();
    expect(screen.queryByText('First summary.')).not.toBeInTheDocument();
  });

  it('blocks only the publisher represented by the current view', async () => {
    const onblock = vi.fn();
    render(SharedWikiViewer, {
      source: 'public',
      sourceName: 'Public network',
      browse: publicWiki(),
      loading: false,
      t: translate,
      metadata: () => 'Unverified',
      onback: vi.fn(),
      onmore: vi.fn(),
      onblock
    });

    await fireEvent.click(screen.getByRole('button', { name: 'Block publisher' }));
    expect(onblock).toHaveBeenCalledWith('publisher-a');
  });
});

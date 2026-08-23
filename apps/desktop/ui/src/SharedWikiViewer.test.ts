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
    workspaceSupported: false,
    workspaceFingerprint: null,
    reservedPages: [],
    documents: [],
    links: [],
    nextGraphCursor: null,
    page: null,
    appendFailed: false,
    ...overrides
  };
}

function completePublicWiki(): PublicBrowseSummary {
  const conceptPage = { kind: 'concept' as const, conceptId: 'concept-a' };
  const conceptFingerprint = '3'.repeat(64);
  return publicWiki({
    workspaceSupported: true,
    workspaceFingerprint: '0'.repeat(64),
    reservedPages: [{
      page: { kind: 'index' }, logicalPath: 'index.md', title: 'Index',
      fingerprint: '1'.repeat(64)
    }, {
      page: { kind: 'log' }, logicalPath: 'log.md', title: 'Log',
      fingerprint: '2'.repeat(64)
    }],
    documents: [{
      page: conceptPage, logicalPath: 'guides/first.md', title: 'First concept',
      fingerprint: conceptFingerprint
    }],
    links: [{ source: { kind: 'index' }, target: conceptPage, label: 'First concept' }],
    page: {
      descriptor: {
        page: conceptPage, logicalPath: 'guides/first.md', title: 'First concept',
        fingerprint: conceptFingerprint
      },
      blocks: [
        { kind: 'heading', level: 1, text: 'Published guide' },
        { kind: 'paragraph', text: 'The complete published OKF page is visible.' }
      ],
      metadata: [['generated.by', 'human:owner']],
      backlinks: [{ kind: 'index' }],
      truncated: false
    }
  });
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
  'knowledge-index-title': 'Index',
  'knowledge-recovery-history': 'History',
  'desktop-wiki-view': 'View',
  'desktop-view-list': 'List',
  'desktop-view-graph': 'Graph',
  'desktop-graph-loading': 'Building the wiki graph…',
  'desktop-shared-published-metadata': 'Published metadata',
  'desktop-shared-summary-label': 'Summary',
  'desktop-concept-assurance-title': 'Assurance',
  'desktop-concept-type': 'Type',
  'desktop-concept-trust': 'Trust',
  'desktop-shared-source': 'Source',
  'search-public-block-publisher': 'Block publisher'
};

function translate(id: string): string {
  return labels[id] ?? id;
}

describe('SharedWikiViewer', () => {
  afterEach(cleanup);

  it('reserves the full remote workspace while access is validated', () => {
    const { container } = render(SharedWikiViewer, {
      source: 'public',
      sourceName: 'Public network',
      browse: null,
      loading: true,
      structureLoading: false,
      pageLoading: false,
      t: translate,
      metadata: () => 'Unverified',
      onback: vi.fn(),
      onopenpage: vi.fn()
    });

    expect(screen.getByRole('status')).toHaveTextContent('desktop-shared-loading-title');
    expect(container.querySelector('.loading-skeleton.workspace')).toBeInTheDocument();
  });

  it('renders the complete published workspace without visible pagination', async () => {
    const onopenpage = vi.fn();
    render(SharedWikiViewer, {
      source: 'public',
      sourceName: 'Public network',
      browse: completePublicWiki(),
      loading: false,
      structureLoading: false,
      pageLoading: false,
      initialConceptId: 'concept-a',
      t: translate,
      metadata: () => 'Human reviewed',
      onback: vi.fn(),
      onopenpage
    });

    expect(await screen.findByText('The complete published OKF page is visible.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /index\.md/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /log\.md/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'List' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('button', { name: 'Graph' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /load more/i })).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: /index\.md/ }));
    expect(onopenpage).toHaveBeenCalledWith({ kind: 'index' }, '1'.repeat(64));

    await fireEvent.click(screen.getByRole('button', { name: 'Graph' }));
    expect(screen.getByText('Building the wiki graph…')).toBeInTheDocument();
    await fireEvent.click(await screen.findByRole('button', { name: 'First concept' }));
    expect(screen.getByRole('button', { name: 'Graph' })).toHaveAttribute('aria-pressed', 'true');
    expect(onopenpage).toHaveBeenLastCalledWith(
      { kind: 'concept', conceptId: 'concept-a' },
      '3'.repeat(64)
    );
  });

  it('renders a requested page before its outline page arrives', async () => {
    const wiki = completePublicWiki();
    const requestedPage = {
      page: { kind: 'concept' as const, conceptId: 'concept-z' },
      logicalPath: 'guides/selected.md',
      title: 'Selected concept',
      fingerprint: '9'.repeat(64)
    };
    wiki.documents = [];
    wiki.concepts = [{
      conceptId: 'concept-z',
      conceptType: 'Guide',
      title: 'Selected concept',
      description: '',
      language: 'en',
      tags: [],
      summary: 'Selected summary.',
      sourceRevision: 1,
      lifecycle: 'stable',
      assurance: null
    }];
    wiki.page = {
      descriptor: requestedPage,
      blocks: [{ kind: 'paragraph', text: 'The selected page arrived before the outline.' }],
      metadata: [],
      backlinks: [],
      truncated: false
    };

    render(SharedWikiViewer, {
      source: 'public',
      sourceName: 'Public network',
      browse: wiki,
      loading: false,
      structureLoading: false,
      pageLoading: false,
      initialConceptId: 'concept-z',
      t: translate,
      metadata: () => 'Unverified',
      onback: vi.fn(),
      onopenpage: vi.fn()
    });

    expect(await screen.findByText('The selected page arrived before the outline.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /guides\/selected\.md/ })).toHaveAttribute('aria-current', 'page');
  });

  it('treats publisher and Wiki IDs as part of the page identity', async () => {
    const firstWiki = publicWiki();
    const { rerender } = render(SharedWikiViewer, {
      source: 'public',
      sourceName: 'Public network',
      browse: firstWiki,
      loading: false,
      structureLoading: false,
      pageLoading: false,
      t: translate,
      metadata: () => 'Unverified',
      onback: vi.fn(),
      onopenpage: vi.fn()
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
      structureLoading: false,
      pageLoading: false,
      t: translate,
      metadata: () => 'Unverified',
      onback: vi.fn(),
      onopenpage: vi.fn()
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
      structureLoading: false,
      pageLoading: false,
      t: translate,
      metadata: () => 'Unverified',
      onback: vi.fn(),
      onopenpage: vi.fn(),
      onblock
    });

    await fireEvent.click(screen.getByRole('button', { name: 'Block publisher' }));
    expect(onblock).toHaveBeenCalledWith('publisher-a');
  });
});

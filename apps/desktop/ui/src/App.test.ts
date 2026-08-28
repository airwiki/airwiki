import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { allowPeerPairingAgain, approveProjectMemoryRequest, approveReview, browseNearbyWiki, browsePublicWiki, cancelModelInstall, checkUpdates, configureFirewall, connect, createProjectMemory, detachProjectMemory, explorePublicWikis, installModels, loadReviewEvidence, loadWikiBundle, loadWikiPage, manageIntegration, openSystemDestination, pickOkfImport, pickWikiFolder, prepareGuidedWikiRepair, quitCompletely, refreshApplicationAccess, refreshConnectivity, refreshWikiHealth, rejectProjectMemoryRequest, rejectReview, rescanWiki, searchKnowledge, setApplicationWikiRole, setWikiGrant, updatePreferences, updateWikiPolicy, validateOkfImport, verifyWikiConcept } from './api';
import { setModelProfile } from './api';
import type { AppSnapshot, SearchCoverage, SearchHitSummary, SearchStatus, UiEventEnvelope } from './generated/ui-contract';
import { readySnapshot } from './test/fixtures';

let snapshot = readySnapshot();
let snapshotListener: ((event: UiEventEnvelope) => void) | null = null;
const tauriListeners = new Map<string, (event: unknown) => void>();
const legacyRemoteWorkspace = {
  workspaceSupported: false,
  workspaceFingerprint: null,
  reservedPages: [],
  documents: [],
  links: [],
  nextGraphCursor: null,
  page: null
};

function publishedRemoteWorkspace(
  conceptId: string,
  title: string,
  body = 'Contenido OKF completo publicado por el propietario.'
) {
  const conceptPage = { kind: 'concept' as const, conceptId };
  const conceptFingerprint = '3'.repeat(64);
  return {
    workspaceSupported: true,
    workspaceFingerprint: '0'.repeat(64),
    reservedPages: [{
      page: { kind: 'index' as const }, logicalPath: 'index.md', title: 'Index',
      fingerprint: '1'.repeat(64)
    }, {
      page: { kind: 'log' as const }, logicalPath: 'log.md', title: 'Log',
      fingerprint: '2'.repeat(64)
    }],
    documents: [{
      page: conceptPage, logicalPath: `guides/${conceptId}.md`, title,
      fingerprint: conceptFingerprint
    }],
    links: [{ source: { kind: 'index' as const }, target: conceptPage, label: title }],
    nextGraphCursor: null,
    page: {
      descriptor: {
        page: conceptPage, logicalPath: `guides/${conceptId}.md`, title,
        fingerprint: conceptFingerprint
      },
      blocks: [
        { kind: 'heading' as const, level: 1, text: title },
        { kind: 'paragraph' as const, text: body }
      ],
      metadata: [['type', 'Guide']] as Array<[string, string]>,
      backlinks: [{ kind: 'index' as const }],
      truncated: false
    }
  };
}
function activateLocalSearch() {
  snapshot.model = {
    stateSequence: 1, profile: 'automatic', recommendedModelId: 'synthetic-model',
    displayName: 'Synthetic local model', recommendationReason: null, active: true, activeModelId: 'synthetic-model',
    installed: true, degraded: false, issues: [], pendingModelId: null,
    downloadBytes: 0, requiredFreeBytes: 0, fitsAvailableDisk: true,
    licenseAccepted: true, license: null, licenseUrl: null, revision: 'fixture'
  };
}

type SearchFixtureHit = SearchHitSummary & {
  nodeId: string;
  route: 'deviceNetwork' | 'publicNetwork';
};

function searchSummary(
  requestId: string,
  coverage: SearchCoverage,
  hits: SearchFixtureHit[],
  status: SearchStatus = 'complete'
): NonNullable<AppSnapshot['search']> {
  return {
    requestId,
    status,
    coverage,
    results: hits.map((hit) => {
      const { nodeId, route, ...match } = hit;
      const local = route === 'deviceNetwork' && nodeId === snapshot.nodeId;
      const peer = snapshot.peers.find((candidate) => candidate.peerId === nodeId);
      return {
        wikiId: hit.wikiId,
        wikiName: snapshot.wikis.find((wiki) => wiki.id === hit.wikiId)?.name ?? hit.title,
        description: route === 'publicNetwork' ? 'Public fixture Wiki' : null,
        languages: route === 'publicNetwork' ? ['en'] : [],
        conceptCount: 1,
        okfCompatibility: { kind: 'declaredV02' },
        bestRank: hit.rank,
        totalMatches: 1,
        matches: [match],
        source: route === 'publicNetwork'
          ? {
            kind: 'public', publisherId: nodeId,
            publisherLabel: nodeId.length > 16 ? `${nodeId.slice(0, 8)}…${nodeId.slice(-6)}` : nodeId
          }
          : local
            ? { kind: 'local', private: true, health: 'ready' }
            : {
              kind: 'nearby', peerId: nodeId, deviceName: peer?.deviceName ?? null,
              platform: peer?.platform ?? null, accessGranted: true, available: peer?.activity !== 'notObserved'
            }
      };
    })
  };
}

async function submitVisibleSearch(query = 'fixture search') {
  const form = await screen.findByRole('search');
  const input = form.querySelector('input');
  expect(input).not.toBeNull();
  await fireEvent.input(input!, { target: { value: query } });
  await fireEvent.submit(form);
  await waitFor(() => expect(searchKnowledge).toHaveBeenCalled());
}

async function openSettingsSection(section: 'general' | 'connections' | 'apps') {
  await waitFor(() => {
    expect(
      screen.queryByRole('button', { name: /^Configuración\./ })
      ?? screen.queryByRole('navigation', { name: 'Configuración' })
    ).not.toBeNull();
  });
  const settingsButton = screen.queryByRole('button', { name: /^Configuración\./ });
  if (settingsButton) await fireEvent.click(settingsButton);
  const name = section === 'general' ? /^General/ : section === 'connections' ? /^Conexiones/ : /^Apps de IA/;
  const link = screen.getByRole('link', { name });
  if (link.getAttribute('aria-current') !== 'page') await fireEvent.click(link);
  return screen.findByRole('heading', { name: section === 'general' ? 'General' : section === 'connections' ? 'Conexiones' : 'Apps de IA', level: 1 });
}

async function openFirstSearchMatch(scope: ParentNode = document) {
  const button = scope.querySelector<HTMLButtonElement>('.wiki-search-matches button');
  expect(button).not.toBeNull();
  await fireEvent.click(button!);
}
const accessibilityCases = (['es', 'en'] as const).flatMap((locale) =>
  (['light', 'dark'] as const).flatMap((theme) =>
    (['library', 'search', 'system/models', 'settings/general', 'settings/connections', 'settings/apps'] as const)
      .map((route) => [locale, theme, route] as const)
  )
);

vi.mock('./api', async (importOriginal) => {
  const original = await importOriginal() as typeof import('./api');
  return {
    ...original,
    connect: vi.fn(async (onEvent: (event: UiEventEnvelope) => void) => {
      snapshotListener = onEvent;
      return snapshot;
    }),
    refreshAutostart: vi.fn(async () => undefined),
    refreshConnectivity: vi.fn(async () => undefined),
    refreshWikiHealth: vi.fn(async () => undefined),
    addWiki: vi.fn(async () => undefined),
    prepareGuidedWikiRepair: vi.fn(async () => 'repair-request'),
    updatePreferences: vi.fn(async () => undefined),
    configureFirewall: vi.fn(async () => undefined),
    checkUpdates: vi.fn(async () => undefined),
    installModels: vi.fn(async () => undefined),
    cancelModelInstall: vi.fn(async () => undefined),
    setModelProfile: vi.fn(async () => undefined),
    openSystemDestination: vi.fn(async () => undefined),
    manageIntegration: vi.fn(async () => undefined),
    allowPeerPairingAgain: vi.fn(async () => undefined),
    explorePublicWikis: vi.fn(async () => 'public-catalog-request'),
    browsePublicWiki: vi.fn(async () => 'public-browse-request'),
    browseNearbyWiki: vi.fn(async () => 'nearby-browse-request'),
    searchKnowledge: vi.fn(async () => snapshot.search?.requestId ?? 'search-request'),
    setWikiGrant: vi.fn(async () => undefined),
    updateWikiPolicy: vi.fn(async () => undefined),
    pickOkfImport: vi.fn(async () => null),
    pickWikiFolder: vi.fn(async () => null),
    validateOkfImport: vi.fn(),
    createProjectMemory: vi.fn(async () => undefined),
    refreshApplicationAccess: vi.fn(async () => undefined),
    setApplicationWikiRole: vi.fn(async () => undefined),
    refreshComputations: vi.fn(async () => undefined),
    approveProjectMemoryRequest: vi.fn(async () => undefined),
    rejectProjectMemoryRequest: vi.fn(async () => undefined),
    detachProjectMemory: vi.fn(async () => undefined),
    loadWikiBundle: vi.fn(async () => undefined),
    loadWikiPage: vi.fn(async () => undefined),
    loadReviewEvidence: vi.fn(async () => 'review-evidence-request'),
    verifyWikiConcept: vi.fn(async () => undefined),
    approveReview: vi.fn(async () => undefined),
    rejectReview: vi.fn(async () => undefined),
    rescanWiki: vi.fn(async () => undefined),
    quitCompletely: vi.fn(async () => undefined)
  };
});

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (event: unknown) => void) => {
    tauriListeners.set(event, handler);
    return () => { tauriListeners.delete(event); };
  })
}));

describe('AirWiki wiki workspace', () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    delete document.documentElement.dataset.theme;
    delete document.documentElement.dataset.platform;
    document.documentElement.style.colorScheme = '';
    Reflect.deleteProperty(window, '__TAURI_INTERNALS__');
  });

  it('uses the transparent AirWiki mark in the persistent app header', async () => {
    const { container } = render(App);

    await screen.findByRole('button', { name: 'Biblioteca' });
    const logo = container.querySelector<HTMLImageElement>('.top-brand-logo');
    expect(logo).not.toBeNull();
    expect(logo?.getAttribute('src')).toContain('airwiki-mark-transparent');
    expect(logo).toHaveAttribute('alt', '');
    expect(container.querySelector('.top-brand .brand-mark')).not.toBeInTheDocument();
  });

  beforeEach(() => {
    window.location.hash = '';
    snapshot = readySnapshot();
    snapshotListener = null;
    tauriListeners.clear();
  });

  it('renders one wiki workspace with global search and no redundant sidebar', async () => {
    render(App);

    expect((await screen.findAllByText('Atlas')).length).toBeGreaterThan(0);
    expect(screen.getByRole('heading', { name: 'Tus wikis' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^Configuración\./ })).toBeInTheDocument();
    const search = screen.getByRole('search');
    expect(search).toBeInTheDocument();
    expect(search.querySelector('input')).toHaveAttribute('autocomplete', 'off');
    expect(screen.getByRole('button', { name: 'Nueva wiki' })).toBeInTheDocument();
    expect(screen.getByRole('list', { name: 'Tus wikis' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Atlas 2 de 2 revisados/ })).toBeInTheDocument();
    expect(screen.queryByRole('complementary')).not.toBeInTheDocument();
  });

  it('uses the same attention classification for the Wiki filters and rows', async () => {
    snapshot.wikis[0].staleConceptCount = 1;
    const { container } = render(App);

    expect(await screen.findByRole('button', { name: /Necesitan atención.*1/ })).toBeInTheDocument();
    expect(container.querySelector('.wiki-row-status.attention')).not.toBeNull();
  });

  it('does not call a Wiki ready when content was detected but nothing is searchable', async () => {
    snapshot.wikis[0].publishedCount = 0;
    snapshot.wikis[0].needsReviewCount = 0;
    const { container } = render(App);

    expect(await screen.findByRole('button', { name: /Necesitan atención.*1/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Atlas 0 de 0 revisados.*Revisar indexación.*Se detectó contenido/ })).toBeInTheDocument();
    expect(container.querySelector('.wiki-row-status.attention')).not.toBeNull();
  });

  it('filters the Wiki list by attention and real access without mixing it with knowledge search', async () => {
    snapshot.wikis[0].needsReviewCount = 1;
    snapshot.wikis.push({
      ...snapshot.wikis[0],
      id: '20000000-0000-4000-8000-000000000002',
      name: 'Equipo',
      needsReviewCount: 0,
      localOnly: false,
      peerShareable: true
    });
    snapshot.peers = [{
      peerId: 'peer-a',
      deviceName: 'MacBook',
      platform: 'macOs',
      address: '/ip4/192.0.2.1/tcp/4242',
      trust: 'trusted',
      activity: 'notObserved',
      sasWords: null,
      grantedWikiIds: ['20000000-0000-4000-8000-000000000002']
    }];
    render(App);

    const filters = within(await screen.findByRole('group', { name: 'Mostrar' }));
    expect(await filters.findByRole('button', { name: /Todas.*2/ })).toBeInTheDocument();
    expect(filters.getByRole('button', { name: /Necesitan atención.*1/ })).toBeInTheDocument();
    expect(filters.getByRole('button', { name: /Solo tú.*1/ })).toBeInTheDocument();
    expect(filters.getByRole('button', { name: /Compartidas.*1/ })).toBeInTheDocument();

    await fireEvent.click(filters.getByRole('button', { name: /Solo tú.*1/ }));
    expect(screen.getByRole('button', { name: /^Atlas / })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Equipo / })).not.toBeInTheDocument();

    await fireEvent.click(filters.getByRole('button', { name: /Compartidas.*1/ }));
    expect(screen.getByRole('button', { name: /^Equipo / })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Atlas / })).not.toBeInTheDocument();
    expect(screen.getByRole('search')).toBeInTheDocument();
  });

  it('keeps AI connections separate from the network sharing filter', async () => {
    const wiki = snapshot.wikis[0];
    wiki.origin = 'aiMemory';
    wiki.memoryKind = 'personal';
    wiki.indexingMode = 'notApplicable';
    wiki.localOnly = false;
    wiki.allowExternalAi = true;
    snapshot.applicationAccess = [{
      appId: 'codex-desktop',
      clientName: 'codex',
      displayName: 'Codex',
      producer: 'OpenAI',
      active: true,
      ownedWikiCount: 1,
      managedBytes: 0,
      grants: [{ wikiId: wiki.id, role: 'owner' }]
    }];
    render(App);

    const filters = within(await screen.findByRole('group', { name: 'Mostrar' }));
    const privateWikis = filters.getByRole('button', { name: /Solo tú.*1/ });
    expect(filters.getByRole('button', { name: /Compartidas.*0/ })).toBeInTheDocument();
    await fireEvent.click(privateWikis);
    expect(screen.getByRole('button', { name: /Atlas 2 de 2 revisados.*Acceso en 1 app/ })).toBeInTheDocument();
  });

  it('keeps the first-Wiki action while making public exploration an explicit choice', async () => {
    snapshot.wikis = [];
    render(App);

    await screen.findByRole('button', { name: 'Elegir cómo crearla' });
    const publicScope = screen.getByRole('button', { name: 'Públicas' });
    expect(explorePublicWikis).not.toHaveBeenCalled();
    expect(screen.queryByText('Nombre')).not.toBeInTheDocument();

    await fireEvent.click(publicScope);
    expect(await screen.findByRole('heading', { name: 'Explorar wikis públicas' })).toBeInTheDocument();
    expect(explorePublicWikis).toHaveBeenCalledTimes(1);

    await fireEvent.click(screen.getByRole('button', { name: /En este dispositivo.*0/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir cómo crearla' }));
    expect(screen.getByRole('dialog', { name: '¿De dónde viene esta wiki?' })).toBeInTheDocument();
  });

  it('explains when configured public indexes need an update', async () => {
    snapshot.publicCatalog = {
      requestId: 'catalog-upgrade-required',
      status: 'upgradeRequired',
      wikis: []
    };
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Públicas' }));

    expect(screen.getByRole('alert')).toHaveTextContent('El catálogo público necesita una actualización');
    expect(screen.getByRole('button', { name: 'Reintentar' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Revisar conexiones' })).not.toBeInTheDocument();
  });

  it('routes a missing public-index configuration to Connections', async () => {
    snapshot.publicCatalog = {
      requestId: 'catalog-not-configured',
      status: 'notConfigured',
      wikis: []
    };
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Públicas' }));

    expect(screen.getByRole('alert')).toHaveTextContent('No hay un índice público configurado');
    expect(screen.getByRole('button', { name: 'Revisar conexiones' })).toBeInTheDocument();
  });

  it('opens a public Wiki from the catalog without exposing its publisher identity', async () => {
    snapshot.publicCatalog = {
      requestId: 'catalog-ready',
      status: 'complete',
      wikis: [{
        publisherId: '12D3KooWPrivateTransportIdentity',
        wikiId: '30000000-0000-4000-8000-000000000003',
        name: 'Huerta comunitaria',
        description: 'Guías públicas para cultivar en comunidad',
        languages: ['es'],
        conceptCount: 14,
        okfCompatibility: { kind: 'declaredV02' }
      }]
    };
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Públicas' }));
    const row = await screen.findByRole('button', { name: /Huerta comunitaria.*14 conceptos.*Red pública/ });
    expect(row).not.toHaveTextContent('12D3KooWPrivateTransportIdentity');
    await fireEvent.click(row);

    expect(browsePublicWiki).toHaveBeenCalledWith(
      '12D3KooWPrivateTransportIdentity',
      '30000000-0000-4000-8000-000000000003',
      { graphCursor: 0 }
    );
    expect(window.location.hash).toBe('#library/shared');
  });

  it('reports only search sources that are currently authorized and available', async () => {
    activateLocalSearch();
    render(App);

    expect(await screen.findByText('Este equipo')).toBeInTheDocument();
    expect(screen.queryByText(/cercano/)).not.toBeInTheDocument();

    snapshot = { ...snapshot, sequence: snapshot.sequence + 1, peers: [{
      peerId: '12D3KooSyntheticNearbyNode', deviceName: 'Nearby Mac', platform: 'macOs', address: '',
      trust: 'trusted', activity: 'connected', sasWords: null, grantedWikiIds: [snapshot.wikis[0].id]
    }] };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: null, kind: 'stateChanged', snapshot });
    });

    expect(await screen.findByText('Este equipo + 1 cercano')).toBeInTheDocument();
  });

  it('returns from a wiki detail to the top-level workspace through the AirWiki brand', async () => {
    const { container } = render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    const wikiHeading = await screen.findByRole('heading', { name: 'Atlas' });
    await waitFor(() => expect(wikiHeading).toHaveFocus());
    expect(container.querySelector('.drive-page')).toHaveClass('wiki-open');
    expect(wikiHeading.closest('.drive-route')).toHaveClass('wiki-route');
    expect(wikiHeading.closest('.wiki-heading')?.nextElementSibling).toHaveClass('wiki-detail-body');
    expect(screen.getByRole('button', { name: 'Lista' }).closest('.content-tabs-bar')).toHaveClass('wiki-content-sticky');
    expect(screen.getByRole('region', { name: 'Estado de Atlas mientras exploras su contenido' })).toBeInTheDocument();
    expect(container.querySelector('.wiki-journey')).toBeNull();
    expect(container.querySelector('.wiki-detail-body > .wiki-toolbar')).not.toBeInTheDocument();
    await fireEvent.click(screen.getAllByRole('button', { name: 'Biblioteca' })[0]);

    const workspaceHeading = await screen.findByRole('heading', { name: 'Tus wikis' });
    await waitFor(() => expect(workspaceHeading).toHaveFocus());
    expect(container.querySelector('.drive-page')).not.toHaveClass('wiki-open');
    expect(screen.queryByRole('heading', { name: 'Atlas' })).not.toBeInTheDocument();
  });

  it('preserves the wiki workspace geometry while its published bundle loads', async () => {
    const { container } = render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));

    expect(await screen.findByRole('status')).toHaveTextContent('El bundle se está actualizando');
    expect(container.querySelector('.loading-skeleton.workspace')).toBeInTheDocument();
    expect(container.querySelector('.shimmer-text.active')).toBeInTheDocument();
  });

  it('replaces a failed wiki load with a recoverable state instead of an endless skeleton', async () => {
    vi.mocked(loadWikiBundle).mockRejectedValueOnce(new Error('synthetic load failure'));
    const { container } = render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));

    expect(await screen.findByRole('alert')).toHaveTextContent('No se pudo abrir esta wiki');
    expect(screen.getByRole('button', { name: 'Volver a intentar' })).toBeInTheDocument();
    expect(container.querySelector('.loading-skeleton.workspace')).not.toBeInTheDocument();
  });

  it('does not let a stale connect response replace a newer channel snapshot', async () => {
    const starting = readySnapshot();
    starting.sequence = 0;
    starting.phase = 'starting';
    starting.preferences = null;
    const latest = readySnapshot();
    latest.sequence = 2;
    vi.mocked(connect).mockImplementationOnce(async (onEvent) => {
      onEvent({
        schemaVersion: latest.schemaVersion,
        sequence: latest.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot: latest
      });
      return starting;
    });

    render(App);

    expect(await screen.findByRole('heading', { name: 'Tus wikis' })).toBeInTheDocument();
    expect(screen.queryByText('Trabajando')).not.toBeInTheDocument();
  });

  it('replaces an unrecoverable startup wait with a safe localized exit', async () => {
    snapshot.phase = 'failed';
    snapshot.preferences = null;

    const { container } = render(App);

    expect(await screen.findByRole('heading', { name: 'AirWiki could not start' })).toBeInTheDocument();
    expect(screen.queryByText('Working')).not.toBeInTheDocument();
    const quit = screen.getByRole('button', { name: 'Quit completely' });
    await fireEvent.click(quit);
    expect(quitCompletely).toHaveBeenCalledOnce();
    const results = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(results.violations.filter((violation) => ['critical', 'serious'].includes(violation.impact ?? ''))).toHaveLength(0);
  });

  it('groups technical services into clear user-facing system states', async () => {
    const { container } = render(App);

    const settings = await screen.findByRole('button', { name: /Conocimiento local: Configura la búsqueda local.*Conexiones: Solo este dispositivo.*Apps de IA: Disponible/ });
    expect(settings).toBeInTheDocument();
    expect(container.querySelectorAll('.system-status-button .status-segment')).toHaveLength(3);
    expect(screen.queryByRole('button', { name: /MCP:/ })).not.toBeInTheDocument();
  });

  it('refreshes external app status on startup without opening Settings', async () => {
    render(App);

    expect(await screen.findByRole('heading', { name: 'Tus wikis' })).toBeInTheDocument();
    await waitFor(() => expect(manageIntegration).toHaveBeenCalledWith(expect.any(String), { kind: 'refresh' }));
    expect(window.location.hash).toBe('#library');
    expect(refreshConnectivity).not.toHaveBeenCalled();
  });

  it('refreshes external system status once when the window returns to the foreground', async () => {
    render(App);
    await waitFor(() => expect(manageIntegration).toHaveBeenCalledTimes(1));
    const startupRequestId = vi.mocked(manageIntegration).mock.calls[0]?.[0];
    expect(startupRequestId).toEqual(expect.any(String));

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      integrationRequestId: null,
      integrationCompletedRequestId: startupRequestId ?? null
    };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: startupRequestId ?? null,
        kind: 'stateChanged',
        snapshot
      });
    });

    window.dispatchEvent(new Event('blur'));
    window.dispatchEvent(new Event('focus'));

    await waitFor(() => expect(manageIntegration).toHaveBeenCalledTimes(2));
    expect(refreshConnectivity).toHaveBeenCalledTimes(1);

    window.dispatchEvent(new Event('focus'));
    expect(manageIntegration).toHaveBeenCalledTimes(2);
    expect(refreshConnectivity).toHaveBeenCalledTimes(1);
  });

  it('summarizes ready local knowledge and both network scopes without losing their state', async () => {
    activateLocalSearch();
    snapshot.lanRuntime = { listener: 'listening', discovery: 'active', addressCount: 1 };
    snapshot.wikis[0].internetPublic = true;
    snapshot.wikis[0].publicAnnouncement = { status: 'advertised', acceptedIndexes: 1 };
    render(App);

    expect(await screen.findByRole('button', { name: /Conocimiento local: Listo para buscar.*Conexiones: Cercana y pública/ })).toBeInTheDocument();
  });

  it('keeps the General status aligned with ready local AI when a Wiki health check fails', async () => {
    activateLocalSearch();
    snapshot.wikiHealth = { generation: 2, status: 'failed', errorCount: 1, warningCount: 0, updatingCount: 0, attentionWikiId: null, checked: true };
    render(App);

    expect(await screen.findByRole('button', { name: /Conocimiento local: Listo para buscar/ })).toBeInTheDocument();
    expect(screen.getByRole('alert')).toHaveTextContent('AirWiki no pudo terminar de comprobar tus wikis');
    await openSettingsSection('general');
    expect(screen.getByRole('link', { name: /General.*Listo para buscar/ })).toBeInTheDocument();
    expect(screen.getAllByText('Listo para buscar')).toHaveLength(2);
  });

  it('distinguishes enabled public permission from confirmed Internet availability', async () => {
    snapshot.wikis[0].internetPublic = true;
    snapshot.wikis[0].publicAnnouncement = { status: 'offline' };
    render(App);

    expect(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados.*Acceso público habilitado/ })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Atlas 2 de 2 revisados.*Pública en internet/ })).not.toBeInTheDocument();
  });

  it('does not ask for action when the active model already uses a safe hardware fallback', async () => {
    activateLocalSearch();
    snapshot.model!.degraded = true;
    render(App);

    expect(await screen.findByRole('button', { name: /Conocimiento local: Listo para buscar/ })).toBeInTheDocument();
  });

  it('explains a global Wiki health failure and offers a direct retry', async () => {
    activateLocalSearch();
    snapshot.wikiHealth = { generation: 2, status: 'failed', errorCount: 1, warningCount: 0, updatingCount: 0, attentionWikiId: null, checked: true };
    render(App);

    expect(await screen.findByRole('alert')).toHaveTextContent('AirWiki no pudo terminar de comprobar tus wikis');
    const retry = screen.getByRole('button', { name: 'Comprobar ahora' });
    await fireEvent.click(retry);
    expect(refreshWikiHealth).toHaveBeenCalledOnce();
  });

  it('keeps connections pending until nearby discovery is active', async () => {
    snapshot.lanRuntime = { listener: 'listening', discovery: 'starting', addressCount: 1 };
    render(App);

    expect(await screen.findByRole('button', { name: /Conexiones: Trabajando/ })).toBeInTheDocument();
  });

  it('surfaces a nearby discovery failure as an actionable connection state', async () => {
    snapshot.lanRuntime = { listener: 'listening', discovery: 'failed', addressCount: 1 };
    render(App);

    expect(await screen.findByRole('button', { name: /Conexiones: Falló/ })).toBeInTheDocument();
  });

  it('gives advanced connection sections distinct names', async () => {
    render(App);
    await openSettingsSection('connections');

    expect(screen.getByText('Detalles de red privada')).toBeInTheDocument();
    expect(screen.getByText('Federación pública avanzada')).toBeInTheDocument();
    expect(screen.getAllByText('Detalles avanzados')).toHaveLength(1);
    expect(window.location.hash).toBe('#settings/connections');
  });

  it('keeps Connections focused when it opens from public sharing', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Compartir' }));
    await fireEvent.click(screen.getByRole('switch', { name: 'Equipos cercanos' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Conexiones' }));

    expect(await screen.findByRole('heading', { name: 'Conexiones', level: 1 })).toBeInTheDocument();
    expect(window.location.hash).toBe('#settings/connections');
    expect(manageIntegration).toHaveBeenCalledTimes(1);
  });

  it('shows a copyable generic MCP setup without exposing a capability', async () => {
    snapshot.integrations = {
      externalAiWikiCount: 0,
      integrations: [{
        client: 'genericMcp', status: 'configured', detectedVersion: null,
        activityRecent: false, restartRequired: false,
        mcpSetup: { command: '/synthetic/managed/airwiki-mcp-bridge', args: ['--client', 'generic-mcp'] },
        workflowGuide: { kind: 'mcpInstructions', status: 'builtIn', version: '1', restartRequired: true }
      }]
    };
    render(App);
    await openSettingsSection('apps');

    expect(await screen.findByRole('heading', { name: 'Aplicaciones de chat' })).toBeInTheDocument();
    expect(screen.getByText('Cliente MCP genérico')).toBeInTheDocument();
    const setup = screen.getByText(/synthetic\/managed\/airwiki-mcp-bridge/);
    expect(setup).toHaveTextContent('"generic-mcp"');
    expect(setup).not.toHaveTextContent('capability');
  });

  it('refreshes integrations when an integration deep link opens connections', async () => {
    window.location.hash = '#system/integrations';
    render(App);

    expect(await screen.findByRole('heading', { name: 'Apps de IA', level: 1 })).toBeInTheDocument();
    await waitFor(() => expect(manageIntegration).toHaveBeenCalledWith(expect.any(String), { kind: 'refresh' }));
    expect(window.location.hash).toBe('#settings/apps');
  });

  it('uses durable snapshot state to finish a coalesced integration request', async () => {
    snapshot.integrations = {
      externalAiWikiCount: 0,
      integrations: [{
        client: 'genericMcp', status: 'available', detectedVersion: null,
        activityRecent: false, restartRequired: false, mcpSetup: null,
        workflowGuide: { kind: 'mcpInstructions', status: 'builtIn', version: '1', restartRequired: true }
      }]
    };
    render(App);
    await openSettingsSection('apps');
    await waitFor(() => expect(manageIntegration).toHaveBeenCalledWith(expect.any(String), { kind: 'refresh' }));
    const refreshRequestId = vi.mocked(manageIntegration).mock.calls[0]?.[0];
    expect(refreshRequestId).toEqual(expect.any(String));

    await fireEvent.click(screen.getByRole('button', { name: 'Conectar' }));
    const genericMcpArticle = (await screen.findByText('Cliente MCP genérico')).closest('article');
    expect(genericMcpArticle).not.toBeNull();
    const connectButton = within(genericMcpArticle as HTMLElement).getByRole('button', { name: 'Conectar' });
    expect(connectButton).toBeDisabled();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      integrationRequestId: null,
      integrationCompletedRequestId: null
    };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot
      });
    });
    expect(connectButton).toBeDisabled();

    snapshot = { ...snapshot, sequence: snapshot.sequence + 1, integrationRequestId: refreshRequestId ?? null };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot
      });
    });
    expect(connectButton).toBeDisabled();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      integrationRequestId: null,
      integrationCompletedRequestId: refreshRequestId ?? null
    };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot
      });
    });
    expect(connectButton).toBeEnabled();
  });

  it('releases integration controls when dispatch fails after an active snapshot', async () => {
    let rejectIntegration: ((error: Error) => void) | null = null;
    vi.mocked(manageIntegration).mockImplementationOnce(() => new Promise<void>((_resolve, reject) => {
      rejectIntegration = reject;
    }));
    snapshot.integrations = {
      externalAiWikiCount: 0,
      integrations: [{
        client: 'genericMcp', status: 'available', detectedVersion: null,
        activityRecent: false, restartRequired: false, mcpSetup: null,
        workflowGuide: { kind: 'mcpInstructions', status: 'builtIn', version: '1', restartRequired: true }
      }]
    };
    render(App);
    await openSettingsSection('apps');
    await waitFor(() => expect(manageIntegration).toHaveBeenCalledWith(expect.any(String), { kind: 'refresh' }));
    const refreshRequestId = vi.mocked(manageIntegration).mock.calls[0]?.[0];
    expect(refreshRequestId).toEqual(expect.any(String));

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      integrationRequestId: refreshRequestId ?? null
    };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot
      });
    });
    const genericMcpArticle = (await screen.findByText('Cliente MCP genérico')).closest('article');
    expect(genericMcpArticle).not.toBeNull();
    const connectButton = within(genericMcpArticle as HTMLElement).getByRole('button', { name: 'Conectar' });
    expect(connectButton).toBeDisabled();

    if (!rejectIntegration) throw new Error('integration rejection was not captured');
    await act(async () => {
      rejectIntegration?.(new Error('synthetic dispatch failure'));
      await Promise.resolve();
    });
    expect(connectButton).toBeEnabled();
  });

  it('opens device preferences from disabled local-network guidance', async () => {
    render(App);
    await openSettingsSection('connections');

    expect(screen.getByRole('combobox', { name: 'Acceso desde dispositivos cercanos' })).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: 'Conexiones' })).not.toBeInTheDocument();
    expect(window.location.hash).toBe('#settings/connections');
  });

  it('offers the closed Windows firewall action only when the helper is verified', async () => {
    snapshot.preferences!.lanPreference = 'enabled';
    snapshot.connectivity = { systemPermission: 'notApplicable', networkProfile: 'private', firewall: 'rulesMissing', firewallHelper: 'verified' };
    render(App);
    await openSettingsSection('connections');

    await fireEvent.click(screen.getByRole('button', { name: 'Configurar firewall…' }));
    expect(configureFirewall).toHaveBeenCalledWith(expect.any(String), true);

    cleanup();
    snapshot.connectivity.networkProfile = 'public';
    render(App);
    await openSettingsSection('connections');
    await fireEvent.click(screen.getByRole('button', { name: 'Abrir configuración de red' }));
    expect(openSystemDestination).toHaveBeenCalledWith(expect.any(String), 'networkSettings');
  });

  it('explains a rejected pairing and requires an explicit safe retry', async () => {
    snapshot.preferences!.lanPreference = 'enabled';
    snapshot.peers = [{
      peerId: '12D3KooBlockedSyntheticPeer', deviceName: 'Office PC', platform: 'windows', address: '',
      trust: 'blocked', activity: 'discovered', sasWords: null, grantedWikiIds: []
    }];
    render(App);
    await openSettingsSection('connections');

    expect(screen.getByText('Verificación bloqueada')).toBeInTheDocument();
    expect(screen.getByText(/Los códigos no coincidieron/)).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Permitir volver a verificar' }));

    expect(allowPeerPairingAgain).toHaveBeenCalledWith('12D3KooBlockedSyntheticPeer');
  });

  it('keeps Settings as a full-screen route with focused section headings', async () => {
    const { container } = render(App);
    const heading = await openSettingsSection('connections');

    const shell = container.querySelector('.drive-shell');
    expect((shell as HTMLElement & { inert: boolean }).inert).toBe(false);
    expect(shell).not.toHaveAttribute('aria-hidden');
    await waitFor(() => expect(heading).toHaveFocus());
    expect(screen.getByRole('button', { name: 'Volver' })).toBeInTheDocument();
    expect(screen.queryByRole('search')).not.toBeInTheDocument();
  });

  it('makes the wiki source chooser modal and restores focus when Escape closes it', async () => {
    const { container } = render(App);
    const newWikiButton = await screen.findByRole('button', { name: 'Nueva wiki' });
    newWikiButton.focus();
    await fireEvent.click(newWikiButton);

    const shell = container.querySelector('.drive-shell');
    const firstChoice = screen.getByRole('button', { name: /Crear memoria de proyecto/ });
    expect((shell as HTMLElement & { inert: boolean }).inert).toBe(true);
    await waitFor(() => expect(firstChoice).toHaveFocus());
    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(screen.queryByRole('dialog', { name: '¿De dónde viene esta wiki?' })).not.toBeInTheDocument();
    expect((shell as HTMLElement & { inert: boolean }).inert).toBe(false);
    await waitFor(() => expect(newWikiButton).toHaveFocus());
  });

  it('keeps the OKF import confirmation modal and focuses its name field', async () => {
    vi.mocked(pickOkfImport).mockResolvedValue({ token: 'okf-selection', displayName: 'Portable OKF' });
    vi.mocked(validateOkfImport).mockResolvedValue({
      entryCount: 2,
      conceptCount: 1,
      uncompressedBytes: 1024,
      declaredOkfVersion: '0.2',
      compatibility: { kind: 'declaredV02' },
      warningCount: 0,
      warnings: [],
      restrictions: []
    });
    const { container } = render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Nueva wiki' }));
    await fireEvent.click(screen.getByRole('button', { name: /Importar carpeta OKF/ }));

    const dialog = await screen.findByRole('dialog', { name: 'Revisa el bundle antes de importarlo' });
    const nameField = within(dialog).getByRole('textbox', { name: 'Nombre de la wiki' });
    const shell = container.querySelector('.drive-shell');
    expect((shell as HTMLElement & { inert: boolean }).inert).toBe(true);
    await waitFor(() => expect(nameField).toHaveFocus());
    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(screen.queryByRole('dialog', { name: 'Revisa el bundle antes de importarlo' })).not.toBeInTheDocument();
    expect((shell as HTMLElement & { inert: boolean }).inert).toBe(false);
  });

  it('initializes project memory only from the explicit folder-and-name flow', async () => {
    vi.mocked(pickWikiFolder).mockResolvedValue({ token: 'project-folder-token', displayName: 'Atlas repo' });
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Nueva wiki' }));
    await fireEvent.click(screen.getByRole('button', { name: /Crear memoria de proyecto/ }));

    const dialog = await screen.findByRole('dialog', { name: 'Crear memoria de proyecto' });
    const name = within(dialog).getByRole('textbox', { name: 'Nombre de la wiki' });
    await fireEvent.input(name, { target: { value: 'Atlas — memoria' } });
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Crear .airwiki' }));

    await waitFor(() => {
      expect(createProjectMemory).toHaveBeenCalledWith('Atlas — memoria', 'project-folder-token');
    });
  });

  it('shows project attachment requests and dispatches the selected decision', async () => {
    snapshot.projectMemoryRequests = [{
      requestId: '20000000-0000-4000-8000-000000000001',
      applicationName: 'Codex', kind: 'attach', folderName: 'atlas-clone',
      requestedName: null, expiresAt: '2026-08-24T00:00:00Z'
    }];
    render(App);
    await openSettingsSection('apps');

    const section = (await screen.findByRole('heading', { name: 'Solicitudes de memoria de proyecto' }))
      .closest('section');
    expect(section).not.toBeNull();
    expect(screen.queryByRole('list', { name: 'Tus wikis' })).not.toBeInTheDocument();
    expect(within(section!).getByText('Codex quiere usar la memoria del proyecto')).toBeInTheDocument();
    await fireEvent.click(within(section!).getByRole('button', { name: 'Aprobar' }));
    expect(approveProjectMemoryRequest).toHaveBeenCalledWith(snapshot.projectMemoryRequests[0].requestId);

    await fireEvent.click(within(section!).getByRole('button', { name: 'Rechazar' }));
    expect(rejectProjectMemoryRequest).toHaveBeenCalledWith(snapshot.projectMemoryRequests[0].requestId);
  });

  it('announces new approvals once without treating dismissal as rejection', async () => {
    render(App);
    await screen.findByRole('heading', { name: 'Tus wikis' });
    const firstRequest = {
      requestId: '20000000-0000-4000-8000-000000000001',
      applicationName: 'Codex', kind: 'attach' as const, folderName: 'atlas-clone',
      requestedName: null, expiresAt: '2026-08-24T00:00:00Z'
    };
    snapshot = { ...snapshot, sequence: snapshot.sequence + 1, projectMemoryRequests: [firstRequest] };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: null, kind: 'stateChanged', snapshot });
    });

    expect(await screen.findByText('Hay 1 aprobación pendiente')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Descartar aviso' }));
    expect(rejectProjectMemoryRequest).not.toHaveBeenCalled();
    expect(screen.queryByText('Hay 1 aprobación pendiente')).not.toBeInTheDocument();

    const secondRequest = { ...firstRequest, requestId: '20000000-0000-4000-8000-000000000002', folderName: 'atlas-worktree' };
    snapshot = { ...snapshot, sequence: snapshot.sequence + 1, projectMemoryRequests: [firstRequest, secondRequest] };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: null, kind: 'stateChanged', snapshot });
    });
    await fireEvent.click(await screen.findByRole('button', { name: 'Revisar' }));
    expect(await screen.findByRole('heading', { name: 'Apps de IA', level: 1 })).toBeInTheDocument();
    expect(window.location.hash).toBe('#settings/apps');
  });

  it('keeps healthy project-memory status and detach inside Details', async () => {
    snapshot.wikis[0].origin = 'aiMemory';
    snapshot.wikis[0].memoryKind = 'project';
    snapshot.wikis[0].projectMemoryHealth = 'active';
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));

    expect(screen.queryByText('Vinculada y disponible')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Desvincular' })).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Detalles' }));
    const dialog = screen.getByRole('dialog', { name: 'Atlas' });
    expect(within(dialog).getByText('Memoria portátil del proyecto')).toBeInTheDocument();
    expect(within(dialog).getByText('Vinculada y disponible')).toBeInTheDocument();
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Desvincular' }));
    expect(detachProjectMemory).toHaveBeenCalledWith(snapshot.wikis[0].id);
  });

  it('uses the project-memory banner only for problems and opens recovery details', async () => {
    snapshot.wikis[0].origin = 'aiMemory';
    snapshot.wikis[0].memoryKind = 'project';
    snapshot.wikis[0].projectMemoryHealth = 'invalid';
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));

    const warning = screen.getByRole('region', { name: 'La memoria del proyecto no está disponible' });
    expect(within(warning).getByText(/Los archivos .airwiki no son válidos/)).toBeInTheDocument();
    expect(within(warning).queryByRole('button', { name: 'Desvincular' })).not.toBeInTheDocument();
    await fireEvent.click(within(warning).getByRole('button', { name: 'Detalles' }));
    expect(within(screen.getByRole('dialog', { name: 'Atlas' })).getByRole('button', { name: 'Desvincular' })).toBeInTheDocument();
  });

  it('moves focus to the close confirmation above Settings', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
    render(App);
    await openSettingsSection('connections');
    await waitFor(() => expect(tauriListeners.has('close-choice-required')).toBe(true));

    await act(() => {
      tauriListeners.get('close-choice-required')?.({ payload: null });
    });

    const hideButton = await screen.findByRole('button', { name: 'Ocultar en bandeja' });
    await waitFor(() => expect(hideButton).toHaveFocus());
    expect(screen.getByRole('heading', { name: 'Conexiones', level: 1, hidden: true })).toBeInTheDocument();
    expect(hideButton.closest('.close-confirmation-backdrop')).not.toBeNull();

    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: '¿Qué debe pasar al cerrar?' })).not.toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Conexiones', level: 1 })).toBeInTheDocument();
  });

  it('restores focus after cancelling a standalone close confirmation', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
    render(App);
    const settingsButton = await screen.findByRole('button', { name: /^Configuración\./ });
    settingsButton.focus();
    await waitFor(() => expect(tauriListeners.has('close-choice-required')).toBe(true));

    await act(() => {
      tauriListeners.get('close-choice-required')?.({ payload: null });
    });
    await fireEvent.click(await screen.findByRole('button', { name: 'Cancelar' }));

    await waitFor(() => expect(settingsButton).toHaveFocus());
  });

  it('opens a wiki as an independent page and requests its OKF bundle', async () => {
    render(App);
    const wikiButton = await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ });
    expect(wikiButton).not.toBeNull();
    await fireEvent.click(wikiButton);

    expect(loadWikiBundle).toHaveBeenCalledWith(snapshot.wikis[0].id);
    expect(window.location.hash).toBe('#library/wiki');
    expect(await screen.findByRole('button', { name: /^Todo/ })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('button', { name: /^Borradores/ })).toHaveAttribute('aria-pressed', 'false');
  });

  it('opens Wiki access controls from the status journey', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Compartir' }));

    expect(screen.getByRole('dialog', { name: 'Atlas' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Conexiones', level: 1 })).not.toBeInTheDocument();
  });

  it('lists known private devices with current availability and platform', async () => {
    snapshot.peers = [{
      peerId: '12D3KooSyntheticMacNode', deviceName: 'Atlas Mac', platform: 'macOs', address: '',
      trust: 'trusted', activity: 'connected', sasWords: null, grantedWikiIds: []
    }, {
      peerId: '12D3KooSyntheticWindowsNode', deviceName: 'RUSTICO', platform: 'windows', address: '',
      trust: 'trusted', activity: 'notObserved', sasWords: null, grantedWikiIds: []
    }];
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Compartir' }));
    await fireEvent.click(screen.getByRole('switch', { name: 'Equipos cercanos' }));

    expect(screen.getByRole('checkbox', { name: 'Atlas Mac, macOS' })).toBeInTheDocument();
    expect(screen.getByText('Conexión activa')).toBeInTheDocument();
    expect(screen.getByRole('checkbox', { name: 'RUSTICO, Windows' })).toBeInTheDocument();
    expect(screen.getByText('Se conectará cuando sea necesario')).toBeInTheDocument();
  });

  it('distinguishes an enabled LAN channel from a granted device', async () => {
    snapshot.wikis[0].localOnly = false;
    snapshot.wikis[0].peerShareable = true;
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));

    expect(screen.getByLabelText('Local: Activa')).toBeInTheDocument();
    expect(screen.getByLabelText('LAN: Habilitada')).toBeInTheDocument();
  });

  it('shows private device identity, platform, activity and Wiki access while sharing', async () => {
    const peerId = '12D3KooSyntheticNearbyNode';
    snapshot.peers = [{
      peerId, deviceName: 'RUSTICO', platform: 'windows', address: '/ip4/192.0.2.1/tcp/4242',
      trust: 'trusted', activity: 'connected', sasWords: null, grantedWikiIds: []
    }];
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Compartir' }));
    await fireEvent.click(screen.getByRole('switch', { name: 'Equipos cercanos' }));

    const networkAccess = screen.getByRole('group', { name: 'Acceso de red' });
    expect(within(networkAccess).getByRole('switch', { name: 'Equipos cercanos' })).toBeInTheDocument();
    expect(within(networkAccess).getByRole('switch', { name: 'Red pública' })).toBeInTheDocument();
    expect(within(networkAccess).queryByText('Red privada (LAN)')).not.toBeInTheDocument();
    expect(screen.getByText('Conexión activa')).toBeInTheDocument();
    expect(screen.getByText(/AirWiki no revela el nombre ni el sistema operativo/)).toBeInTheDocument();
    const deviceGrant = screen.getByRole('checkbox', { name: 'RUSTICO, Windows' });
    expect(deviceGrant.closest('label')?.querySelector('.platform-icon.windows')).not.toBeNull();
    await fireEvent.click(deviceGrant);

    expect(setWikiGrant).toHaveBeenCalledWith(peerId, snapshot.wikis[0].id, true);
  });

  it('groups nearby and public access while preserving independent policies', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Compartir' }));

    const networkAccess = screen.getByRole('group', { name: 'Acceso de red' });
    await fireEvent.click(within(networkAccess).getByRole('switch', { name: 'Equipos cercanos' }));
    await waitFor(() => expect(updateWikiPolicy).toHaveBeenLastCalledWith(snapshot.wikis[0].id, {
      localOnly: false,
      peerShareable: true,
      allowExternalAi: false,
      internetPublic: false
    }));

    await fireEvent.click(within(networkAccess).getByRole('switch', { name: 'Red pública' }));
    await waitFor(() => expect(updateWikiPolicy).toHaveBeenLastCalledWith(snapshot.wikis[0].id, {
      localOnly: false,
      peerShareable: true,
      allowExternalAi: false,
      internetPublic: true
    }));
    expect(within(networkAccess).getByRole('textbox', { name: 'Descripción pública' })).toBeInTheDocument();
  });

  it('keeps Share limited to network channels without a misleading generic save', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Compartir' }));

    const dialog = screen.getByRole('dialog', { name: 'Atlas' });
    expect(within(dialog).queryByRole('button', { name: 'Guardar' })).not.toBeInTheDocument();
    expect(within(dialog).queryByRole('switch', { name: 'Aplicaciones de IA' })).not.toBeInTheDocument();
    expect(within(dialog).getByRole('switch', { name: 'Equipos cercanos' })).toBeInTheDocument();
    expect(within(dialog).getByRole('switch', { name: 'Red pública' })).toBeInTheDocument();
    expect(within(dialog).getByText(/Los cambios de canal se aplican después de confirmarlos/)).toBeInTheDocument();
  });

  it('manages per-application memory roles from the dedicated AI Apps panel', async () => {
    const wiki = snapshot.wikis[0];
    wiki.origin = 'aiMemory';
    wiki.memoryKind = 'personal';
    wiki.indexingMode = 'notApplicable';
    wiki.localOnly = false;
    wiki.allowExternalAi = true;
    snapshot.applicationAccess = [{
      appId: 'owner-application', clientName: 'codex', displayName: 'Codex', producer: 'OpenAI', active: true,
      ownedWikiCount: 1, managedBytes: 0, grants: [{ wikiId: wiki.id, role: 'owner' }]
    }, {
      appId: 'reader-application', clientName: 'claude-code', displayName: 'Claude Code', producer: 'Anthropic', active: true,
      ownedWikiCount: 0, managedBytes: 0, grants: []
    }];
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: /Gestionar apps de IA/ }));

    const dialog = screen.getByRole('dialog', { name: 'Atlas' });
    expect(refreshApplicationAccess).toHaveBeenCalledOnce();
    expect(within(dialog).getByRole('heading', { name: 'Permisos por aplicación' })).toBeInTheDocument();
    expect(within(dialog).getByText(/Esto no la comparte en LAN/)).toBeInTheDocument();
    const ownerRow = within(dialog).getByText('Codex').closest<HTMLElement>('article');
    expect(ownerRow).not.toBeNull();
    expect(within(ownerRow!).getByText('Propietaria')).toBeInTheDocument();
    const role = within(dialog).getByRole('combobox', { name: 'Permiso para Claude Code' });
    expect(role).toHaveValue('none');

    await fireEvent.change(role, { target: { value: 'reader' } });
    await waitFor(() => expect(setApplicationWikiRole).toHaveBeenCalledWith('reader-application', wiki.id, 'reader'));
    expect(refreshApplicationAccess).toHaveBeenCalledTimes(2);

    await fireEvent.click(within(dialog).getByRole('button', { name: 'Administrar aplicaciones' }));
    expect(await screen.findByRole('heading', { name: 'Apps de IA', level: 1 })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Permisos por aplicación' })).not.toBeInTheDocument();
  });

  it('offers search-only permission for a regular Wiki in AI Apps', async () => {
    const wiki = snapshot.wikis[0];
    wiki.localOnly = false;
    wiki.allowExternalAi = true;
    snapshot.applicationAccess = [{
      appId: 'chatgpt-application', clientName: 'chatgpt-desktop', displayName: 'ChatGPT', producer: 'OpenAI', active: true,
      ownedWikiCount: 0, managedBytes: 0, grants: [{ wikiId: wiki.id, role: 'reader' }]
    }];
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: /Gestionar apps de IA/ }));

    const dialog = screen.getByRole('dialog', { name: 'Atlas' });
    const permission = within(dialog).getByRole('combobox', { name: 'Permiso para ChatGPT' });
    expect(permission).toHaveValue('reader');
    expect(within(permission).getByRole('option', { name: 'Puede buscar' })).toBeInTheDocument();
    expect(within(permission).queryByRole('option', { name: 'Puede editar' })).not.toBeInTheDocument();

    await fireEvent.change(permission, { target: { value: 'none' } });
    await waitFor(() => expect(setApplicationWikiRole).toHaveBeenCalledWith('chatgpt-application', wiki.id, null));
  });

  it('restores the sharing switch when native confirmation is cancelled', async () => {
    vi.mocked(updateWikiPolicy).mockRejectedValueOnce({
      code: 'invalidInput', messageKey: 'humanConfirmationRequired', retryable: false
    });
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Compartir' }));

    const publicSwitch = screen.getByRole('switch', { name: 'Red pública' });
    await fireEvent.click(publicSwitch);

    await waitFor(() => expect(publicSwitch).not.toBeChecked());
    expect(document.querySelector('.action-message.error')).toBeNull();
  });

  it('keeps wiki details and sharing as separate actions', async () => {
    const { container } = render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));

    const detailsButton = screen.getByRole('button', { name: 'Detalles' });
    detailsButton.focus();
    await fireEvent.click(detailsButton);
    expect(screen.getByRole('dialog', { name: 'Atlas' })).toHaveTextContent('Estado de la fuente');
    expect(screen.queryByText('Equipos cercanos')).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Cerrar' })).toHaveFocus());
    await fireEvent.keyDown(window, { key: 'Tab', shiftKey: true });
    expect(screen.getByText('Detalles avanzados')).toHaveFocus();
    let results = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(results.violations.filter((violation) => violation.impact === 'critical' || violation.impact === 'serious')).toEqual([]);
    await fireEvent.click(screen.getByRole('button', { name: 'Cerrar' }));
    await waitFor(() => expect(detailsButton).toHaveFocus());

    const accessButton = screen.getByRole('button', { name: 'Compartir' });
    accessButton.focus();
    await fireEvent.click(accessButton);
    expect(screen.getByRole('dialog', { name: 'Atlas' })).toHaveTextContent('Equipos cercanos');
    expect(screen.queryByText('Documentos de origen')).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Cerrar' })).toHaveFocus());
    results = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(results.violations.filter((violation) => violation.impact === 'critical' || violation.impact === 'serious')).toEqual([]);
  });

  it('distinguishes maintenance from an empty source-issue state', async () => {
    snapshot.wikis[0].maintenanceRequired = true;
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(screen.getAllByRole('button', { name: 'Detalles' }).at(-1)!);

    expect(screen.getByText('El contenido publicado necesita una comprobación')).toBeInTheDocument();
    expect(screen.queryByText('No hay problemas con la fuente')).not.toBeInTheDocument();
  });

  it('carries a maintenance reason and its safe next action into the opened wiki', async () => {
    const wiki = snapshot.wikis[0];
    wiki.maintenanceRequired = true;
    snapshot.wikiHealth!.attentionWikiId = wiki.id;
    const { container } = render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));

    await fireEvent.click(screen.getByRole('button', { name: /Hay que comprobar el contenido publicado.*Revisar reparación segura/ }));
    expect(prepareGuidedWikiRepair).toHaveBeenCalledWith(wiki.id);
    const results = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(results.violations.filter((violation) => violation.impact === 'critical' || violation.impact === 'serious')).toEqual([]);
  });

  it('explains skipped source files and translates their safe issue code', async () => {
    const wiki = snapshot.wikis[0];
    wiki.failedCount = 1;
    snapshot.sourceIssues = [{
      wikiId: wiki.id,
      wikiName: wiki.name,
      sourceName: 'manual.pdf',
      code: 'EncryptedPdf'
    }];
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: /1 archivo no pudo incluirse.*Ver archivos y solución/ }));
    expect(screen.getByRole('dialog', { name: wiki.name })).toHaveTextContent('El PDF está cifrado');
    expect(screen.getByRole('dialog', { name: wiki.name })).toHaveTextContent('Guarda una copia sin contraseña en la carpeta de origen');
    expect(screen.queryByText('EncryptedPdf')).not.toBeInTheDocument();
  });

  it('states that pending AI proposals remain unpublished until a person decides', async () => {
    const wiki = snapshot.wikis[0];
    wiki.needsReviewCount = 2;
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 4 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: /Revisa 2 cambios para hacerlos buscables.*Revisar propuestas/ }));
    expect(screen.getByRole('button', { name: /^Borradores/ })).toHaveAttribute('aria-pressed', 'true');
  });

  it('keeps review decisions unavailable while the Wiki update replaces that draft', async () => {
    const wiki = snapshot.wikis[0];
    wiki.needsReviewCount = 1;
    const review = {
      conceptId: 'updating-review', wikiId: wiki.id, sourceRevision: 3, excluded: false,
      sourceName: 'updating.md', wikiName: wiki.name,
      draft: {
        type: 'Reference', title: 'Draft proposal', description: 'Synthetic fixture.',
        language: 'es', tags: [], entities: [], links: [], summary: 'Pending human review.',
        classificationConfidence: 1, classificationExplanation: 'Synthetic fixture.'
      }
    };
    snapshot.reviews = [review];
    snapshot.reanalyzingReviewIds = [review.conceptId];
    snapshot.reviewEvidence = {
      requestId: 'updating-evidence', conceptId: review.conceptId,
      sourceRevision: review.sourceRevision, status: 'ready',
      excerpts: [{ ordinal: 0, headingOrPage: 'Draft', text: 'Current extracted evidence.', truncated: false }],
      totalChunks: 1, nextOrdinal: null
    };
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 3 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: /^Borradores/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'updating.md, Borrador' }));
    const dialog = await screen.findByRole('dialog', { name: 'updating.md' });

    expect(within(dialog).getByText('Volviendo a analizar los borradores actuales')).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Excluir de esta wiki' })).toBeDisabled();
    expect(within(dialog).getByRole('button', { name: 'Aprobar y continuar' })).toBeDisabled();
  });

  it('keeps review evidence progress local and clears it when the matching request completes', async () => {
    const wiki = snapshot.wikis[0];
    wiki.needsReviewCount = 1;
    const review = {
      conceptId: 'draft-review', wikiId: wiki.id, sourceRevision: 3, excluded: false,
      sourceName: 'draft.md', wikiName: wiki.name,
      draft: {
        type: 'Reference', title: 'Draft proposal', description: 'Synthetic fixture.',
        language: 'es', tags: [], entities: [], links: [], summary: 'Pending human review.',
        classificationConfidence: 1, classificationExplanation: 'Synthetic fixture.'
      }
    };
    snapshot.reviews = [review];
    snapshot.reviewEvidence = null;
    vi.mocked(loadReviewEvidence)
      .mockResolvedValueOnce('review-evidence-first')
      .mockResolvedValueOnce('review-evidence-second');
    const { container } = render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 3 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: /^Borradores/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'draft.md, Borrador' }));

    let dialog = await screen.findByRole('dialog', { name: 'draft.md' });
    expect(within(dialog).getByRole('status')).toHaveTextContent('Cargando el texto extraído…');
    expect(container.querySelector('.action-message')).not.toBeInTheDocument();
    expect(loadReviewEvidence).toHaveBeenCalledWith(review);

    await fireEvent.click(within(dialog).getByRole('button', { name: 'Cerrar' }));
    expect(screen.queryByRole('dialog', { name: 'draft.md' })).not.toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'draft.md, Borrador' }));
    dialog = await screen.findByRole('dialog', { name: 'draft.md' });

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      reviewEvidence: {
        requestId: 'review-evidence-first', conceptId: review.conceptId,
        sourceRevision: review.sourceRevision, status: 'ready',
        excerpts: [{ ordinal: 0, headingOrPage: 'Draft', text: 'Current extracted evidence.', truncated: false }],
        totalChunks: 1, nextOrdinal: null
      }
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'review-evidence-first', kind: 'stateChanged', snapshot });
    });
    expect(within(dialog).getByRole('status')).toHaveTextContent('Cargando el texto extraído…');

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      reviewEvidence: { ...snapshot.reviewEvidence!, requestId: 'review-evidence-second' }
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'review-evidence-second', kind: 'stateChanged', snapshot });
    });

    expect(within(dialog).queryByText('Cargando el texto extraído…')).not.toBeInTheDocument();
    expect(within(dialog).getByText('Current extracted evidence.')).toBeInTheDocument();
    expect(container.querySelector('.action-message')).not.toBeInTheDocument();
  });

  it('keeps legacy review cleanup available without offering an impossible publication', async () => {
    const wiki = snapshot.wikis[0];
    wiki.needsReviewCount = 1;
    wiki.okfVersion = '0.1';
    wiki.declaredOkfVersion = '0.1';
    wiki.okfCompatibility = { kind: 'legacyV01' };
    wiki.restrictions = ['legacyReadOnly'];
    const review = {
      conceptId: 'legacy-review', wikiId: wiki.id, sourceRevision: 2, excluded: false,
      sourceName: 'legacy.md', wikiName: wiki.name,
      draft: {
        type: 'Reference', title: 'Legacy proposal', description: 'Synthetic fixture.',
        language: 'es', tags: [], entities: [], links: [], summary: 'Cannot be republished as v0.1.',
        classificationConfidence: 1, classificationExplanation: 'Synthetic fixture.'
      }
    };
    snapshot.reviews = [review];
    snapshot.reviewEvidence = {
      requestId: 'legacy-evidence', conceptId: review.conceptId, sourceRevision: review.sourceRevision,
      status: 'ready', excerpts: [{ ordinal: 0, headingOrPage: 'Legacy', text: 'Synthetic evidence.', truncated: false }],
      totalChunks: 1, nextOrdinal: null
    };
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 3 revisados/ }));
    await fireEvent.click(screen.getByRole('button', { name: /^Borradores/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'legacy.md, Borrador' }));
    const legacyDialog = await screen.findByRole('dialog', { name: 'legacy.md' });

    expect(within(legacyDialog).getByText(/Vuelve a crearla desde la carpeta de origen/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Aprobar y continuar' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Volver a analizar' })).not.toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Excluir de esta wiki' }));
    expect(rejectReview).toHaveBeenCalledWith(review.conceptId, review.sourceRevision);
    expect(approveReview).not.toHaveBeenCalled();
  });

  it('presents restricted OKF wikis as local read-only without impossible actions', async () => {
    const wiki = snapshot.wikis[0];
    wiki.origin = 'folder';
    wiki.indexingMode = 'manual';
    wiki.okfVersion = '0.1';
    wiki.declaredOkfVersion = '0.1';
    wiki.okfCompatibility = { kind: 'legacyV01' };
    wiki.restrictions = ['legacyReadOnly'];
    render(App);

    const legacyRow = await screen.findByRole('button', { name: /Atlas 2 de 2 revisados.*OKF v0\.1 heredado/ });
    await fireEvent.click(legacyRow);

    expect(screen.getByText(/AirWiki detuvo su indexación y uso compartido/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Actualizar' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Compartir' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Gestionar acceso' })).not.toBeInTheDocument();
    await fireEvent.click(screen.getAllByRole('button', { name: 'Detalles' }).at(-1)!);
    expect(screen.queryByRole('checkbox', { name: 'Mantener actualizada automáticamente' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Volver a vincular carpeta' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Eliminar wiki' })).toBeInTheDocument();
  });

  it('updates a folder Wiki from its workspace instead of from one document', async () => {
    const wiki = snapshot.wikis[0];
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    const update = screen.getByRole('button', { name: 'Actualizar desde la carpeta' });
    expect(update).toHaveAttribute(
      'title',
      'Comprueba la carpeta de origen y vuelve a analizar los borradores actuales. El contenido revisado y los documentos excluidos no cambian.'
    );

    await fireEvent.click(update);

    expect(rescanWiki).toHaveBeenCalledWith(wiki.id);
  });

  it('does not offer folder analysis for an imported OKF Wiki', async () => {
    snapshot.wikis[0].origin = 'importedOkf';
    snapshot.wikis[0].indexingMode = 'notApplicable';
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));

    expect(screen.queryByRole('button', { name: 'Actualizar desde la carpeta' })).not.toBeInTheDocument();
  });

  it('keeps guided repair reachable from the unified wiki workspace', async () => {
    const wiki = snapshot.wikis[0];
    wiki.maintenanceRequired = true;
    snapshot.wikiHealth!.attentionWikiId = wiki.id;
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    await fireEvent.click(await screen.findByRole('button', { name: /Hay que comprobar el contenido publicado.*Revisar reparación segura/ }));
    expect(prepareGuidedWikiRepair).toHaveBeenCalledWith(wiki.id);
  });

  it('opens a local search result inside its wiki without placing the query in the URL', async () => {
    const wiki = snapshot.wikis[0];
    const conceptId = 'concept-atlas';
    activateLocalSearch();
    snapshot.search = searchSummary('search-fixture', 'complete', [{ conceptId, wikiId: wiki.id, title: 'Evidencia Atlas', snippet: 'Contenido verificado.', headingOrPage: 'Atlas', logicalResourceUri: 'urn:airwiki:fixture', sourceRevision: 1, sourceSha256: '0123456789abcdef', rank: 1, nodeId: snapshot.nodeId!, route: 'deviceNetwork', assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false }, lifecycle: 'stable' }]);
    snapshot.knowledge = {
      wikiId: wiki.id, wikiName: wiki.name, version: 'search-fixture', status: 'ready', errorCount: 0, warningCount: 0,
      reservedPages: [],
      concepts: [{ conceptId, page: { kind: 'concept', path: 'guides/atlas.md' }, title: 'Evidencia Atlas', description: 'Contenido verificado.', conceptType: 'Reference', tags: [], lifecycle: 'stable', generatedBy: 'airwiki/test', verifiedBy: ['human:test'], sources: [], staleAfter: null, assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false }, warnings: [], executionAvailable: false, fingerprint: 'a'.repeat(64) }],
      links: []
    };
    window.location.hash = '#search';
    render(App);
    await submitVisibleSearch('Evidencia Atlas');
    expect(await screen.findByText(/Revisado por una persona/)).toBeInTheDocument();
    await openFirstSearchMatch();

    expect(loadWikiBundle).toHaveBeenCalledWith(wiki.id);
    expect(loadWikiPage).toHaveBeenCalledWith(wiki.id, { kind: 'concept', path: 'guides/atlas.md' }, 'a'.repeat(64));
    expect(window.location.hash).toBe('#library/wiki');
    expect(window.location.hash).not.toContain('Evidencia');
  });

  it('filters grouped Library results by origin with visible counts', async () => {
    activateLocalSearch();
    const peerId = '12D3KooSyntheticNearbyNode';
    snapshot.peers = [{
      peerId, deviceName: 'Equipo estudio', platform: 'windows', address: '',
      trust: 'trusted', activity: 'connected', sasWords: null, grantedWikiIds: []
    }];
    const common = {
      snippet: 'Coincidencia reutilizable.', headingOrPage: 'Guía', logicalResourceUri: 'urn:airwiki:fixture',
      sourceRevision: 1, sourceSha256: '0123456789abcdef', rank: 1,
      assurance: null, lifecycle: 'stable' as const
    };
    snapshot.search = searchSummary('grouped-search', 'complete', [
      { ...common, conceptId: 'local-concept', wikiId: snapshot.wikis[0].id, title: 'Guía local', nodeId: snapshot.nodeId!, route: 'deviceNetwork' },
      { ...common, conceptId: 'nearby-concept', wikiId: 'nearby-wiki', title: 'Guía cercana', nodeId: peerId, route: 'deviceNetwork' },
      { ...common, conceptId: 'public-concept', wikiId: 'public-wiki', title: 'Guía pública', nodeId: '12D3KooPublicPublisher', route: 'publicNetwork' }
    ]);
    render(App);
    await fireEvent.click(await screen.findByRole('checkbox', { name: 'Buscar también en la red pública' }));
    await submitVisibleSearch('guía');

    expect(screen.getByRole('button', { name: 'Todas 3' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('button', { name: 'Este equipo 1' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cercanas 1' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Públicas 1' })).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Cercanas 1' }));
    expect(screen.getByRole('heading', { name: 'Guía cercana' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Atlas' })).not.toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Guía pública' })).not.toBeInTheDocument();
  });

  it('distinguishes an empty origin filter from an empty search', async () => {
    activateLocalSearch();
    snapshot.search = searchSummary('local-only-search', 'complete', [{
      conceptId: 'local-concept', wikiId: snapshot.wikis[0].id, title: 'Guía local',
      snippet: 'Coincidencia local.', headingOrPage: 'Guía', logicalResourceUri: 'urn:airwiki:local',
      sourceRevision: 1, sourceSha256: 'a'.repeat(64), rank: 1, nodeId: snapshot.nodeId!,
      route: 'deviceNetwork', assurance: null, lifecycle: 'stable'
    }]);
    render(App);
    await submitVisibleSearch('guía local');

    await fireEvent.click(screen.getByRole('button', { name: 'Cercanas 0' }));

    expect(screen.getByText('No hay resultados de este origen')).toBeInTheDocument();
    expect(screen.getByText('Hay coincidencias en otros orígenes. Elige otro filtro para verlas.')).toBeInTheDocument();
    expect(screen.queryByText('No encontramos evidencia coincidente')).not.toBeInTheDocument();
  });

  it('offers only reserved pages that exist in the current OKF bundle', async () => {
    const wiki = snapshot.wikis[0];
    const fingerprint = 'e'.repeat(64);
    snapshot.knowledge = {
      wikiId: wiki.id, wikiName: wiki.name, version: 'minimal-okf', status: 'ready', errorCount: 0, warningCount: 0,
      reservedPages: [],
      concepts: [{ conceptId: 'minimal-concept', page: { kind: 'concept', path: 'guide.md' }, title: 'Minimal guide', description: 'Valid OKF without reserved pages.', conceptType: 'Guide', tags: [], lifecycle: 'stable', generatedBy: null, verifiedBy: [], sources: [], staleAfter: null, assurance: { trust: 'unverified', freshness: 'notDeclared', verificationOutdated: false }, warnings: [], executionAvailable: false, fingerprint }],
      links: []
    };
    window.location.hash = `#wikis/${wiki.id}`;
    render(App);

    expect(screen.queryByText('index.md')).not.toBeInTheDocument();
    expect(screen.queryByText('log.md')).not.toBeInTheDocument();
    await fireEvent.click(await screen.findByRole('button', { name: 'Minimal guide, guide.md, Revisado' }));

    expect(loadWikiPage).toHaveBeenCalledWith(
      wiki.id,
      { kind: 'concept', path: 'guide.md' },
      fingerprint
    );
  });

  it('explains a concurrent Wiki update without mislabeling it as a search error', async () => {
    const wiki = snapshot.wikis[0];
    const fingerprint = 'f'.repeat(64);
    snapshot.knowledge = {
      wikiId: wiki.id, wikiName: wiki.name, version: 'current-revision', status: 'ready', errorCount: 0, warningCount: 0,
      reservedPages: [],
      concepts: [{ conceptId: 'changing-concept', page: { kind: 'concept', path: 'changing.md' }, title: 'Changing guide', description: '', conceptType: 'Guide', tags: [], lifecycle: 'stable', generatedBy: null, verifiedBy: [], sources: [], staleAfter: null, assurance: { trust: 'unverified', freshness: 'notDeclared', verificationOutdated: false }, warnings: [], executionAvailable: false, fingerprint }],
      links: []
    };
    vi.mocked(loadWikiPage).mockRejectedValueOnce({
      code: 'invalidInput',
      messageKey: 'currentKnowledgeSnapshotRequired',
      retryable: false
    });
    window.location.hash = `#wikis/${wiki.id}`;
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Changing guide, changing.md, Revisado' }));

    expect(await screen.findByText('La Wiki se actualizó mientras esta página estaba seleccionada. Elige la página nuevamente para abrir su revisión actual.')).toBeInTheDocument();
    expect(screen.queryByText('Este resultado cambió. Vuelve a buscar antes de abrir su página publicada.')).not.toBeInTheDocument();
  });

  it('opens a trusted LAN result with the same read-only Wiki workspace', async () => {
    const peerId = '12D3KooSyntheticNearbyNode';
    activateLocalSearch();
    snapshot.peers = [{
      peerId, deviceName: 'RUSTICO', platform: 'windows', address: '/ip4/192.0.2.1/tcp/4242',
      trust: 'trusted', activity: 'connected', sasWords: null, grantedWikiIds: []
    }];
    snapshot.search = searchSummary('nearby-search', 'complete', [{ conceptId: 'nearby-concept', wikiId: 'nearby-wiki', title: 'Evidencia cercana', snippet: 'Contenido autorizado.', headingOrPage: 'Responsable', logicalResourceUri: 'urn:airwiki:nearby', sourceRevision: 1, sourceSha256: '0123456789abcdef', rank: 1, nodeId: peerId, route: 'deviceNetwork', assurance: null, lifecycle: null }]);
    window.location.hash = '#search';
    const { container } = render(App);
    await submitVisibleSearch('Evidencia cercana');
    expect(searchKnowledge).toHaveBeenLastCalledWith('Evidencia cercana', false);

    const article = (await screen.findByRole('heading', { name: 'Evidencia cercana' })).closest('article');
    expect(article).not.toBeNull();
    const nearbyResult = within(article as HTMLElement);
    expect(nearbyResult.getByText('RUSTICO')).toBeInTheDocument();
    expect(nearbyResult.getByRole('img', { name: 'Windows' })).toBeInTheDocument();
    expect(nearbyResult.getByText('Responsable')).toBeInTheDocument();
    expect(nearbyResult.getByText('Red privada (LAN)')).toBeInTheDocument();
    expect(nearbyResult.getByText('Acceso concedido')).toBeInTheDocument();
    expect(nearbyResult.queryByText('Red pública')).not.toBeInTheDocument();
    const scrollRegion = container.querySelector<HTMLElement>('.drive-page');
    expect(scrollRegion).not.toBeNull();
    scrollRegion!.scrollTop = 420;
    const scrollTo = vi.spyOn(HTMLElement.prototype, 'scrollTo');
    await openFirstSearchMatch(article as HTMLElement);
    expect(browseNearbyWiki).toHaveBeenCalledWith(peerId, 'nearby-wiki', {
      targetConceptId: 'nearby-concept',
      graphCursor: 0,
      page: {
        page: { kind: 'concept', conceptId: 'nearby-concept' },
        expectedFingerprint: null
      }
    });
    expect(screen.getByText('Abriendo wiki compartida')).toBeInTheDocument();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      nearbyBrowse: {
        ...publishedRemoteWorkspace('nearby-concept', 'Evidencia cercana'),
        requestId: 'nearby-browse-request', status: 'available', peerId, wikiId: 'nearby-wiki',
        wikiName: 'Guía del equipo', okfCompatibility: { kind: 'declaredV02' }, nextCursor: null,
        appendFailed: false,
        concepts: [{ conceptId: 'nearby-concept', conceptType: 'Guide', title: 'Evidencia cercana', description: 'Contenido publicado por el equipo.', language: 'es', tags: ['operaciones'], summary: 'Contenido autorizado.', sourceRevision: 1, lifecycle: 'stable', assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false } }]
      }
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'nearby-browse-request', kind: 'stateChanged', snapshot });
    });

    const sharedWikiHeading = screen.getByRole('heading', { name: 'Guía del equipo' });
    expect(sharedWikiHeading).toBeInTheDocument();
    await waitFor(() => expect(sharedWikiHeading).toHaveFocus());
    expect(container.querySelector('.drive-page')).toHaveClass('shared-wiki-open');
    expect(sharedWikiHeading.closest('.drive-route')).toHaveClass('shared-wiki-route');
    const sharedConcept = screen.getByRole('button', { name: /Evidencia cercana/ });
    const sharedConceptFocus = vi.spyOn(sharedConcept, 'focus');
    await fireEvent.mouseDown(sharedConcept);
    expect(sharedConceptFocus).toHaveBeenCalledWith({ preventScroll: true });
    expect(screen.queryByRole('heading', { name: 'Buscar evidencia' })).not.toBeInTheDocument();
    expect(screen.queryByRole('tab')).not.toBeInTheDocument();
    expect(screen.getByText('Solo lectura')).toBeInTheDocument();
    expect(screen.getAllByText('RUSTICO').length).toBeGreaterThan(0);
    expect(screen.getAllByRole('img', { name: 'Windows' }).length).toBeGreaterThan(0);
    expect(screen.getByText('Contenido OKF completo publicado por el propietario.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /index\.md/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /log\.md/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Lista' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('button', { name: 'Grafo' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Bloquear este publicador' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Actualizar' })).not.toBeInTheDocument();
    const accessibility = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(accessibility.violations.filter((violation) => ['critical', 'serious'].includes(violation.impact ?? ''))).toEqual([]);

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      nearbyBrowse: snapshot.nearbyBrowse ? {
        ...snapshot.nearbyBrowse,
        nextCursor: 'nearby-next-page',
        appendFailed: true
      } : null
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: null, kind: 'stateChanged', snapshot });
    });
    expect(screen.getByText('La wiki cambió durante la carga o se interrumpió la conexión. Vuelve a los resultados e inténtalo de nuevo.')).toBeInTheDocument();
    expect(screen.getByText('Contenido OKF completo publicado por el propietario.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Cargar más' })).not.toBeInTheDocument();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      nearbyBrowse: snapshot.nearbyBrowse ? {
        ...snapshot.nearbyBrowse,
        ...publishedRemoteWorkspace('different-concept', 'Otro contenido', 'No debe abrirse por accidente.'),
        concepts: [{
          conceptId: 'different-concept', conceptType: 'Guide', title: 'Otro contenido',
          description: '', language: 'es', tags: [], summary: 'No debe abrirse por accidente.',
          sourceRevision: 1, lifecycle: 'stable', assurance: null
        }]
      } : null
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: null, kind: 'stateChanged', snapshot });
    });
    expect(screen.getByText('Este resultado ya no está disponible')).toBeInTheDocument();
    expect(screen.queryByText('No debe abrirse por accidente.')).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Volver a los resultados' }));
    expect(screen.getByRole('heading', { name: 'Evidencia cercana' })).toBeInTheDocument();
    await waitFor(() => expect(scrollTo).toHaveBeenLastCalledWith({ top: 420, left: 0, behavior: 'auto' }));
  });

  it('restores the Library scroll position when browser history closes a shared Wiki', async () => {
    activateLocalSearch();
    snapshot.search = searchSummary('history-search', 'complete', [{
      conceptId: 'nearby-concept', wikiId: 'nearby-wiki', title: 'Resultado remoto',
      snippet: 'Contenido autorizado.', headingOrPage: 'Guía', logicalResourceUri: 'urn:airwiki:nearby',
      sourceRevision: 1, sourceSha256: 'b'.repeat(64), rank: 1,
      nodeId: '12D3KooHistoryPeer', route: 'deviceNetwork', assurance: null, lifecycle: 'stable'
    }]);
    const { container } = render(App);
    await submitVisibleSearch('Resultado remoto');
    const article = (await screen.findByRole('heading', { name: 'Resultado remoto' })).closest('article');
    expect(article).not.toBeNull();
    const scrollRegion = container.querySelector<HTMLElement>('.drive-page');
    expect(scrollRegion).not.toBeNull();
    scrollRegion!.scrollTop = 275;
    const scrollTo = vi.spyOn(HTMLElement.prototype, 'scrollTo');
    await openFirstSearchMatch(article as HTMLElement);

    window.history.replaceState(null, '', '#library');
    window.dispatchEvent(new PopStateEvent('popstate'));

    expect(await screen.findByRole('heading', { name: 'Resultado remoto' })).toBeInTheDocument();
    await waitFor(() => expect(scrollTo).toHaveBeenLastCalledWith({ top: 275, left: 0, behavior: 'auto' }));
  });

  it('ignores a stale automatic continuation after another shared Wiki is opened', async () => {
    const peerId = '12D3KooSyntheticNearbyNode';
    activateLocalSearch();
    snapshot.peers = [{
      peerId, deviceName: 'RUSTICO', platform: 'windows', address: '/ip4/192.0.2.1/tcp/4242',
      trust: 'trusted', activity: 'connected', sasWords: null, grantedWikiIds: []
    }];
    snapshot.search = searchSummary('nearby-search', 'complete', [
        { conceptId: 'concept-a', wikiId: 'wiki-a', title: 'Wiki remota A', snippet: 'Primer contenido.', headingOrPage: 'Guía A', logicalResourceUri: 'urn:airwiki:a', sourceRevision: 1, sourceSha256: 'a'.repeat(64), rank: 1, nodeId: peerId, route: 'deviceNetwork', assurance: null, lifecycle: 'stable' },
        { conceptId: 'concept-b', wikiId: 'wiki-b', title: 'Wiki remota B', snippet: 'Segundo contenido.', headingOrPage: 'Guía B', logicalResourceUri: 'urn:airwiki:b', sourceRevision: 1, sourceSha256: 'b'.repeat(64), rank: 2, nodeId: peerId, route: 'deviceNetwork', assurance: null, lifecycle: 'stable' }
      ]);
    window.location.hash = '#search';
    render(App);
    await submitVisibleSearch('Wiki remota');

    const firstResult = (await screen.findByRole('heading', { name: 'Wiki remota A' })).closest('article');
    expect(firstResult).not.toBeNull();
    await openFirstSearchMatch(firstResult as HTMLElement);
    let resolveStaleLoad: (requestId: string) => void = vi.fn();
    const staleLoad = new Promise<string>((resolve) => { resolveStaleLoad = resolve; });
    vi.mocked(browseNearbyWiki).mockImplementationOnce(() => staleLoad);
    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      nearbyBrowse: {
        ...publishedRemoteWorkspace('concept-a', 'Concepto A'),
        requestId: 'nearby-browse-request', status: 'available', peerId, wikiId: 'wiki-a',
        wikiName: 'Wiki remota A', okfCompatibility: { kind: 'declaredV02' },
        nextCursor: 'next-a', appendFailed: false,
        concepts: [{ conceptId: 'concept-a', conceptType: 'Guide', title: 'Concepto A', description: '', language: 'es', tags: [], summary: 'Primer contenido.', sourceRevision: 1, lifecycle: 'stable', assurance: null }]
      }
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'nearby-browse-request', kind: 'stateChanged', snapshot });
    });
    await waitFor(() => expect(browseNearbyWiki).toHaveBeenLastCalledWith(peerId, 'wiki-a', {
      cursor: 'next-a', graphCursor: null
    }));
    expect(screen.queryByRole('button', { name: 'Cargar más' })).not.toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Volver a los resultados' }));

    const secondResult = (await screen.findByRole('heading', { name: 'Wiki remota B' })).closest('article');
    expect(secondResult).not.toBeNull();
    await openFirstSearchMatch(secondResult as HTMLElement);
    await act(async () => {
      resolveStaleLoad('stale-load-more-request');
      await staleLoad;
    });

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      nearbyBrowse: {
        ...publishedRemoteWorkspace('concept-b', 'Concepto B'),
        requestId: 'nearby-browse-request', status: 'available', peerId, wikiId: 'wiki-b',
        wikiName: 'Wiki remota B', okfCompatibility: { kind: 'declaredV02' },
        nextCursor: null, appendFailed: false,
        concepts: [{ conceptId: 'concept-b', conceptType: 'Guide', title: 'Concepto B', description: '', language: 'es', tags: [], summary: 'Segundo contenido.', sourceRevision: 1, lifecycle: 'stable', assurance: null }]
      }
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'nearby-browse-request', kind: 'stateChanged', snapshot });
    });

    expect(screen.getByRole('heading', { name: 'Wiki remota B' })).toBeInTheDocument();
    expect(screen.queryByText('Abriendo wiki compartida')).not.toBeInTheDocument();
  });

  it('does not retain a completed automatic continuation when its event wins the invoke race', async () => {
    const peerId = '12D3KooSyntheticFastNearbyNode';
    activateLocalSearch();
    snapshot.peers = [{
      peerId, deviceName: 'RUSTICO', platform: 'windows', address: '/ip4/192.0.2.1/tcp/4242',
      trust: 'trusted', activity: 'connected', sasWords: null, grantedWikiIds: []
    }];
    snapshot.search = searchSummary('nearby-fast-search', 'complete', [{ conceptId: 'concept-a', wikiId: 'wiki-a', title: 'Wiki remota rápida', snippet: 'Primer contenido.', headingOrPage: 'Guía A', logicalResourceUri: 'urn:airwiki:fast-a', sourceRevision: 1, sourceSha256: 'a'.repeat(64), rank: 1, nodeId: peerId, route: 'deviceNetwork', assurance: null, lifecycle: 'stable' }]);
    window.location.hash = '#search';
    render(App);
    await submitVisibleSearch('Wiki remota rápida');

    const result = (await screen.findByRole('heading', { name: 'Wiki remota rápida' })).closest('article');
    expect(result).not.toBeNull();
    await openFirstSearchMatch(result as HTMLElement);
    let resolveFastLoad: (requestId: string) => void = vi.fn();
    const fastLoad = new Promise<string>((resolve) => { resolveFastLoad = resolve; });
    vi.mocked(browseNearbyWiki).mockImplementationOnce(() => fastLoad);
    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      nearbyBrowse: {
        ...publishedRemoteWorkspace('concept-a', 'Concepto A'),
        requestId: 'nearby-browse-request', status: 'available', peerId, wikiId: 'wiki-a',
        wikiName: 'Wiki remota rápida', okfCompatibility: { kind: 'declaredV02' },
        nextCursor: 'next-a', appendFailed: false,
        concepts: [{ conceptId: 'concept-a', conceptType: 'Guide', title: 'Concepto A', description: '', language: 'es', tags: [], summary: 'Primer contenido.', sourceRevision: 1, lifecycle: 'stable', assurance: null }]
      }
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'nearby-browse-request', kind: 'stateChanged', snapshot });
    });
    await waitFor(() => expect(browseNearbyWiki).toHaveBeenLastCalledWith(peerId, 'wiki-a', {
      cursor: 'next-a', graphCursor: null
    }));
    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      nearbyBrowse: snapshot.nearbyBrowse ? {
        ...snapshot.nearbyBrowse,
        requestId: 'fast-auto-request',
        nextCursor: null
      } : null
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'fast-auto-request', kind: 'stateChanged', snapshot });
    });
    await act(async () => {
      resolveFastLoad('fast-auto-request');
      await fastLoad;
    });

    await submitVisibleSearch('Wiki remota rápida');
    const refreshedResult = (await screen.findByRole('heading', { name: 'Wiki remota rápida' })).closest('article');
    expect(refreshedResult).not.toBeNull();
    expect(refreshedResult?.querySelector('.wiki-search-matches button')).toBeEnabled();
  });

  it('queues a page selection while the remote Wiki structure is still loading', async () => {
    const peerId = '12D3KooSyntheticQueuedPageNode';
    activateLocalSearch();
    snapshot.peers = [{
      peerId, deviceName: 'RUSTICO', platform: 'windows', address: '/ip4/192.0.2.1/tcp/4242',
      trust: 'trusted', activity: 'connected', sasWords: null, grantedWikiIds: []
    }];
    snapshot.search = searchSummary('nearby-queued-search', 'complete', [{ conceptId: 'concept-a', wikiId: 'wiki-a', title: 'Wiki remota navegable', snippet: 'Primer contenido.', headingOrPage: 'Guía A', logicalResourceUri: 'urn:airwiki:queued-a', sourceRevision: 1, sourceSha256: 'a'.repeat(64), rank: 1, nodeId: peerId, route: 'deviceNetwork', assurance: null, lifecycle: 'stable' }]);
    window.location.hash = '#search';
    render(App);
    await submitVisibleSearch('Wiki remota navegable');
    const result = (await screen.findByRole('heading', { name: 'Wiki remota navegable' })).closest('article');
    expect(result).not.toBeNull();
    await openFirstSearchMatch(result as HTMLElement);

    let resolveStructure: (requestId: string) => void = vi.fn();
    const structureRequest = new Promise<string>((resolve) => { resolveStructure = resolve; });
    vi.mocked(browseNearbyWiki)
      .mockImplementationOnce(() => structureRequest)
      .mockResolvedValueOnce('nearby-page-request');
    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      nearbyBrowse: {
        ...publishedRemoteWorkspace('concept-a', 'Concepto A'),
        requestId: 'nearby-browse-request', status: 'available', peerId, wikiId: 'wiki-a',
        wikiName: 'Wiki remota navegable', okfCompatibility: { kind: 'declaredV02' },
        nextCursor: 'next-a', appendFailed: false,
        concepts: [{ conceptId: 'concept-a', conceptType: 'Guide', title: 'Concepto A', description: '', language: 'es', tags: [], summary: 'Primer contenido.', sourceRevision: 1, lifecycle: 'stable', assurance: null }]
      }
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'nearby-browse-request', kind: 'stateChanged', snapshot });
    });
    await waitFor(() => expect(browseNearbyWiki).toHaveBeenLastCalledWith(peerId, 'wiki-a', {
      cursor: 'next-a', graphCursor: null
    }));

    await fireEvent.click(screen.getByRole('button', { name: /index\.md/ }));
    expect(screen.getByText('Abriendo la página publicada…')).toBeInTheDocument();
    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      nearbyBrowse: snapshot.nearbyBrowse ? {
        ...snapshot.nearbyBrowse,
        requestId: 'nearby-structure-request',
        nextCursor: null
      } : null
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'nearby-structure-request', kind: 'stateChanged', snapshot });
      resolveStructure('nearby-structure-request');
      return structureRequest;
    });
    await waitFor(() => expect(browseNearbyWiki).toHaveBeenLastCalledWith(peerId, 'wiki-a', {
      page: { page: { kind: 'index' }, expectedFingerprint: '1'.repeat(64) }
    }));

    const indexDescriptor = snapshot.nearbyBrowse?.reservedPages[0];
    expect(indexDescriptor).toBeDefined();
    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      nearbyBrowse: snapshot.nearbyBrowse && indexDescriptor ? {
        ...snapshot.nearbyBrowse,
        requestId: 'nearby-page-request',
        page: {
          descriptor: indexDescriptor,
          blocks: [{ kind: 'paragraph', text: 'Índice remoto completo.' }],
          metadata: [], backlinks: [], truncated: false
        }
      } : null
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'nearby-page-request', kind: 'stateChanged', snapshot });
    });
    expect(await screen.findByText('Índice remoto completo.')).toBeInTheDocument();
    expect(screen.queryByText('Abriendo la página publicada…')).not.toBeInTheDocument();
  });

  it('keeps a device-network result labeled as nearby when peer details disappear', async () => {
    activateLocalSearch();
    snapshot.peers = [];
    snapshot.search = searchSummary('departed-peer-search', 'partial', [{ conceptId: 'nearby-concept', wikiId: 'nearby-wiki', title: 'Resultado conservado', snippet: 'Contenido autorizado.', headingOrPage: 'Guía', logicalResourceUri: 'urn:airwiki:nearby', sourceRevision: 1, sourceSha256: '0123456789abcdef', rank: 1, nodeId: '12D3KooDepartedPeer', route: 'deviceNetwork', assurance: null, lifecycle: null }]);
    window.location.hash = '#search';
    render(App);
    await submitVisibleSearch('Resultado conservado');

    const article = (await screen.findByRole('heading', { name: 'Resultado conservado' })).closest('article');
    expect(article).not.toBeNull();
    const result = within(article as HTMLElement);
    expect(result.getByText('Red privada (LAN)')).toBeInTheDocument();
    expect(result.getByText('Equipo cercano')).toBeInTheDocument();
    expect(result.queryByText(/12D3Koo/)).not.toBeInTheDocument();
    expect(result.getByRole('img', { name: 'SO aún no disponible' })).toBeInTheDocument();
    expect(result.getByText('Guía')).toBeInTheDocument();
    expect(result.queryByText('Red pública')).not.toBeInTheDocument();

    await openFirstSearchMatch(article as HTMLElement);
    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      nearbyBrowse: {
        ...legacyRemoteWorkspace,
        requestId: 'nearby-browse-request', status: 'available', peerId: '12D3KooDepartedPeer',
        wikiId: 'nearby-wiki', wikiName: 'Wiki conservada',
        okfCompatibility: { kind: 'declaredV02' }, nextCursor: null, appendFailed: false,
        concepts: [{ conceptId: 'nearby-concept', conceptType: 'Guide', title: 'Resultado conservado', description: '', language: 'es', tags: [], summary: 'Contenido autorizado.', sourceRevision: 1, lifecycle: 'stable', assurance: null }]
      }
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'nearby-browse-request', kind: 'stateChanged', snapshot });
    });
    expect(await screen.findByRole('heading', { name: 'Wiki conservada' })).toBeInTheDocument();
    expect(screen.getAllByText('Equipo cercano').length).toBeGreaterThan(0);
    expect(screen.getAllByRole('img', { name: 'SO aún no disponible' }).length).toBeGreaterThan(0);
    expect(screen.queryByText('Red pública')).not.toBeInTheDocument();
  });

  it('never shows a completed empty search together with a stale progress message', async () => {
    activateLocalSearch();
    snapshot.search = searchSummary('empty-search', 'complete', []);
    window.location.hash = '#search';
    render(App);
    await submitVisibleSearch('sin coincidencias');

    expect(await screen.findByText('No encontramos evidencia coincidente')).toBeInTheDocument();
    expect(screen.getByText('Buscamos en este equipo y en los equipos autorizados disponibles. Prueba formular una pregunta completa sobre la evidencia que necesitas.')).toBeInTheDocument();
    expect(screen.queryByText('Consultando los equipos disponibles…')).not.toBeInTheDocument();
  });

  it('shows an accessible spinner while a search is starting', async () => {
    activateLocalSearch();
    snapshot.search = null;
    window.location.hash = '#search';
    render(App);

    await submitVisibleSearch('estado de carga');

    expect(screen.getByRole('button', { name: 'Consultando los equipos disponibles…' })).toBeDisabled();
    expect(screen.getByRole('status')).toHaveTextContent('Consultando los equipos disponibles…');
    expect(document.querySelector('.spinner')).toBeInTheDocument();
    expect(document.querySelector('.loading-skeleton.results')).toBeInTheDocument();
    expect(document.querySelector('.shimmer-text.active')).toBeInTheDocument();
  });

  it('names every source checked by a completed public search with no matches', async () => {
    activateLocalSearch();
    snapshot.search = searchSummary('empty-public-search', 'complete', []);
    window.location.hash = '#search';
    render(App);
    await fireEvent.click(await screen.findByRole('checkbox', { name: 'Buscar también en la red pública' }));
    await submitVisibleSearch('sin coincidencias públicas');

    expect(screen.getByText('Buscamos en este equipo, en los equipos autorizados disponibles y en la red pública. Prueba formular una pregunta completa sobre la evidencia que necesitas.')).toBeInTheDocument();
  });

  it('does not present an unavailable public search as a conclusive empty result', async () => {
    activateLocalSearch();
    snapshot.search = searchSummary('offline-public-search', 'publicNetworkOffline', []);
    window.location.hash = '#search';
    render(App);
    await fireEvent.click(await screen.findByRole('checkbox', { name: 'Buscar también en la red pública' }));
    await submitVisibleSearch('red pública offline');

    expect(await screen.findByText('No se pudieron consultar todas las fuentes')).toBeInTheDocument();
    expect(screen.getByText('La red pública está offline. La búsqueda local y en equipos emparejados sigue disponible.')).toBeInTheDocument();
    expect(screen.queryByText('No encontramos evidencia coincidente')).not.toBeInTheDocument();
  });

  it('shows public v2 assurance and labels metadata from an older concept as unavailable', async () => {
    activateLocalSearch();
    snapshot.search = searchSummary('public-assurance-search', 'complete', [{ conceptId: 'public-v2', wikiId: 'public-wiki', title: 'Concepto v2', snippet: 'Resumen público', headingOrPage: 'Guía', logicalResourceUri: 'urn:airwiki:public', sourceRevision: 1, sourceSha256: '0123456789abcdef', rank: 1, nodeId: '12D3KooPublicPublisher', route: 'publicNetwork', assurance: null, lifecycle: 'stable' }]);
    window.location.hash = '#search';
    render(App);
    await fireEvent.click(await screen.findByRole('checkbox', { name: 'Buscar también en la red pública' }));
    await submitVisibleSearch('Concepto v2');
    await openFirstSearchMatch();
    const currentWorkspace = publishedRemoteWorkspace('public-v2', 'Concepto v2', 'Contenido v2 completo.');
    const legacyDescriptor = {
      page: { kind: 'concept' as const, conceptId: 'public-v1' },
      logicalPath: 'guides/public-v1.md', title: 'Concepto anterior', fingerprint: '4'.repeat(64)
    };
    snapshot.publicBrowse = {
      ...currentWorkspace,
      documents: [...currentWorkspace.documents, legacyDescriptor],
      requestId: 'public-browse-request', status: 'direct', publisherId: '12D3KooPublicPublisher', wikiId: 'public-wiki',
      wikiName: 'Wiki pública', description: 'Conocimiento compartido', languages: ['es'],
      okfCompatibility: { kind: 'declaredV02' }, nextCursor: null, appendFailed: false,
      concepts: [{
        conceptId: 'public-v2', conceptType: 'Reference', title: 'Concepto v2',
        description: '', language: 'es', tags: [], summary: 'Resumen público', sourceRevision: 1,
        lifecycle: 'stable', assurance: { trust: 'machineConfirmed', freshness: 'stale', verificationOutdated: false }
      }, {
        conceptId: 'public-v1', conceptType: 'Document', title: 'Concepto anterior',
        description: '', language: 'es', tags: [], summary: 'Resumen anterior', sourceRevision: 1,
        lifecycle: null, assurance: null
      }]
    };
    snapshot = { ...snapshot, sequence: snapshot.sequence + 1 };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'public-browse-request', kind: 'stateChanged', snapshot });
    });

    expect(await screen.findByText('OKF v0.2')).toBeInTheDocument();
    expect(await screen.findByText('Confirmado por proceso · Necesita revalidación · stable')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: /Concepto anterior/ }));
    await waitFor(() => expect(browsePublicWiki).toHaveBeenLastCalledWith(
      '12D3KooPublicPublisher',
      'public-wiki',
      { page: { page: legacyDescriptor.page, expectedFingerprint: legacyDescriptor.fingerprint } }
    ));
    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      publicBrowse: snapshot.publicBrowse ? {
        ...snapshot.publicBrowse,
        page: {
          descriptor: legacyDescriptor,
          blocks: [{ kind: 'paragraph', text: 'Contenido anterior completo.' }],
          metadata: [], backlinks: [], truncated: false
        }
      } : null
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'public-browse-request', kind: 'stateChanged', snapshot });
    });
    expect(screen.getByText('Metadata de confianza no disponible (nodo anterior)')).toBeInTheDocument();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      publicBrowse: snapshot.publicBrowse ? { ...snapshot.publicBrowse, okfCompatibility: null } : null
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'public-browse-request', kind: 'stateChanged', snapshot });
    });
    expect(screen.getByText('Compatibilidad OKF no informada (nodo anterior)')).toBeInTheDocument();
  });

  it('opens a public search result in a dedicated viewer and returns to the results', async () => {
    activateLocalSearch();
    snapshot.peers = [{
      peerId: '12D3KooPublicPublisher', deviceName: 'Known publisher', platform: 'windows',
      address: '/ip4/192.0.2.9/tcp/4242', trust: 'trusted', activity: 'connected',
      sasWords: null, grantedWikiIds: []
    }];
    snapshot.search = searchSummary('public-search', 'complete', [{ conceptId: 'public-concept', wikiId: 'public-wiki', title: 'Resultado público', snippet: 'Evidencia pública.', headingOrPage: 'Guía', logicalResourceUri: 'urn:airwiki:public', sourceRevision: 1, sourceSha256: '0123456789abcdef', rank: 1, nodeId: '12D3KooPublicPublisher', route: 'publicNetwork', assurance: null, lifecycle: 'stable' }]);
    window.location.hash = '#search';
    const { container } = render(App);
    await fireEvent.click(await screen.findByRole('checkbox', { name: 'Buscar también en la red pública' }));
    await submitVisibleSearch('Resultado público');

    expect(screen.getAllByText('Red pública').length).toBeGreaterThan(0);
    expect(screen.getByText('Publicador público 12D3KooP…lisher')).toBeInTheDocument();
    expect(screen.queryByText('Known publisher · Windows')).not.toBeInTheDocument();
    await openFirstSearchMatch();
    expect(browsePublicWiki).toHaveBeenCalledWith('12D3KooPublicPublisher', 'public-wiki', {
      targetConceptId: 'public-concept',
      graphCursor: 0,
      page: {
        page: { kind: 'concept', conceptId: 'public-concept' },
        expectedFingerprint: null
      }
    });
    expect(screen.getByText('Abriendo wiki compartida')).toBeInTheDocument();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      publicBrowse: {
        ...publishedRemoteWorkspace('public-concept', 'Concepto público', 'Contenido público OKF completo.'),
        requestId: 'public-browse-request', status: 'direct', publisherId: '12D3KooPublicPublisher', wikiId: 'public-wiki',
        wikiName: 'Wiki pública', description: 'Bundle validado', languages: ['es'], okfCompatibility: { kind: 'declaredV02' }, nextCursor: null,
        appendFailed: false,
        concepts: [{ conceptId: 'public-concept', conceptType: 'Guide', title: 'Concepto público', description: '', language: 'es', tags: [], summary: 'Resumen visible', sourceRevision: 1, lifecycle: 'stable', assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false } }]
      }
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'public-browse-request', kind: 'stateChanged', snapshot });
    });

    expect(screen.getByRole('heading', { name: 'Wiki pública' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Buscar evidencia' })).not.toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Concepto público', level: 2 })).toBeInTheDocument();
    expect(screen.getByText('Contenido público OKF completo.')).toBeInTheDocument();
    expect(screen.getByText('Conexión directa autenticada')).toBeInTheDocument();
    const accessibility = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(accessibility.violations.filter((violation) => ['critical', 'serious'].includes(violation.impact ?? ''))).toEqual([]);
    await fireEvent.click(screen.getByRole('button', { name: 'Volver a los resultados' }));
    expect(screen.getByRole('heading', { name: 'Resultado público' })).toBeInTheDocument();
  });

  it('hides stale search results as soon as the query is edited or cleared', async () => {
    activateLocalSearch();
    snapshot.search = searchSummary('stale-search', 'complete', []);
    window.location.hash = '#search';
    render(App);
    await submitVisibleSearch('consulta anterior');

    expect(await screen.findByText('No encontramos evidencia coincidente')).toBeInTheDocument();
    const form = screen.getByRole('search');
    const input = form.querySelector('input');
    expect(input).not.toBeNull();
    await fireEvent.input(input!, { target: { value: '' } });

    expect(screen.queryByText('No encontramos evidencia coincidente')).not.toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Tus wikis' })).toBeInTheDocument();
    expect(screen.getByRole('list', { name: 'Tus wikis' })).toBeInTheDocument();
  });

  it('searches the latest query automatically after typing pauses', async () => {
    activateLocalSearch();
    render(App);
    const form = await screen.findByRole('search');
    const input = form.querySelector('input');
    expect(input).not.toBeNull();
    vi.useFakeTimers();
    try {
      await fireEvent.input(input!, { target: { value: 'consulta' } });
      await act(async () => { await vi.advanceTimersByTimeAsync(250); });
      await fireEvent.input(input!, { target: { value: 'consulta actualizada' } });
      await act(async () => { await vi.advanceTimersByTimeAsync(399); });
      expect(searchKnowledge).not.toHaveBeenCalled();

      await act(async () => { await vi.advanceTimersByTimeAsync(1); });

      expect(searchKnowledge).toHaveBeenCalledOnce();
      expect(searchKnowledge).toHaveBeenLastCalledWith('consulta actualizada', false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps public consent scoped to the exact query', async () => {
    activateLocalSearch();
    render(App);
    const form = await screen.findByRole('search');
    const input = form.querySelector('input');
    expect(input).not.toBeNull();
    vi.useFakeTimers();
    try {
      await fireEvent.input(input!, { target: { value: 'consulta pública' } });
      const publicSearch = screen.getByRole('checkbox', { name: 'Buscar también en la red pública' });
      await fireEvent.click(publicSearch);
      await act(async () => { await vi.advanceTimersByTimeAsync(400); });
      expect(searchKnowledge).toHaveBeenLastCalledWith('consulta pública', true);

      await fireEvent.input(input!, { target: { value: 'consulta privada revisada' } });
      expect(publicSearch).not.toBeChecked();
      await act(async () => { await vi.advanceTimersByTimeAsync(400); });

      expect(searchKnowledge).toHaveBeenLastCalledWith('consulta privada revisada', false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('searches a pending query when the local model becomes ready', async () => {
    render(App);
    const form = await screen.findByRole('search');
    const input = form.querySelector('input');
    expect(input).not.toBeNull();
    vi.useFakeTimers();
    try {
      await fireEvent.input(input!, { target: { value: 'consulta en espera' } });
      await act(async () => { await vi.advanceTimersByTimeAsync(400); });
      expect(searchKnowledge).not.toHaveBeenCalled();

      activateLocalSearch();
      snapshot = { ...snapshot, sequence: snapshot.sequence + 1 };
      await act(() => {
        snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: null, kind: 'stateChanged', snapshot });
      });
      await act(async () => { await vi.advanceTimersByTimeAsync(400); });

      expect(searchKnowledge).toHaveBeenCalledOnce();
      expect(searchKnowledge).toHaveBeenLastCalledWith('consulta en espera', false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('resumes a pending query after local model setup in Settings', async () => {
    render(App);
    const form = await screen.findByRole('search');
    const input = form.querySelector('input');
    expect(input).not.toBeNull();
    vi.useFakeTimers();
    try {
      await fireEvent.input(input!, { target: { value: 'consulta tras preparar el modelo' } });
      await fireEvent.click(screen.getByRole('button', { name: 'Ver estado de la IA local' }));

      activateLocalSearch();
      snapshot = { ...snapshot, sequence: snapshot.sequence + 1 };
      await act(() => {
        snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: null, kind: 'stateChanged', snapshot });
      });
      await act(async () => { await vi.advanceTimersByTimeAsync(400); });
      expect(searchKnowledge).not.toHaveBeenCalled();

      await fireEvent.click(screen.getByRole('button', { name: 'Volver' }));
      await act(async () => { await vi.advanceTimersByTimeAsync(400); });

      expect(searchKnowledge).toHaveBeenCalledOnce();
      expect(searchKnowledge).toHaveBeenLastCalledWith('consulta tras preparar el modelo', false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('lets Enter search immediately without leaving a duplicate delayed search', async () => {
    activateLocalSearch();
    render(App);
    const form = await screen.findByRole('search');
    const input = form.querySelector('input');
    expect(input).not.toBeNull();
    vi.useFakeTimers();
    try {
      await fireEvent.input(input!, { target: { value: 'consulta inmediata' } });
      await fireEvent.submit(form);
      await act(async () => { await vi.runAllTimersAsync(); });

      expect(searchKnowledge).toHaveBeenCalledOnce();
      expect(searchKnowledge).toHaveBeenLastCalledWith('consulta inmediata', false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('waits for text composition to finish before searching', async () => {
    activateLocalSearch();
    render(App);
    const form = await screen.findByRole('search');
    const input = form.querySelector('input');
    expect(input).not.toBeNull();
    vi.useFakeTimers();
    try {
      await fireEvent.compositionStart(input!);
      await fireEvent.input(input!, { target: { value: '検' } });
      await fireEvent.submit(form);
      await act(async () => { await vi.advanceTimersByTimeAsync(400); });
      expect(searchKnowledge).not.toHaveBeenCalled();

      await fireEvent.compositionEnd(input!, { data: '検索' });
      await fireEvent.input(input!, { target: { value: '検索' } });
      await act(async () => { await vi.advanceTimersByTimeAsync(400); });

      expect(searchKnowledge).toHaveBeenCalledOnce();
      expect(searchKnowledge).toHaveBeenLastCalledWith('検索', false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('hides local-only results when the public search scope changes', async () => {
    activateLocalSearch();
    snapshot.search = searchSummary('local-only-search', 'complete', []);
    window.location.hash = '#search';
    render(App);
    await submitVisibleSearch('consulta local');

    expect(await screen.findByText('No encontramos evidencia coincidente')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Buscar también en la red pública' }));

    expect(screen.queryByText('No encontramos evidencia coincidente')).not.toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Busca en todas tus wikis' })).toBeInTheDocument();
  });

  it('keeps the newer search active when an older dispatch resolves last', async () => {
    activateLocalSearch();
    let resolveFirstSearch!: (requestId: string) => void;
    vi.mocked(searchKnowledge).mockImplementationOnce(() => new Promise((resolve) => {
      resolveFirstSearch = resolve;
    })).mockResolvedValueOnce('newer-search');
    window.location.hash = '#search';
    render(App);
    const form = await screen.findByRole('search');
    const input = form.querySelector('input');
    expect(input).not.toBeNull();
    await fireEvent.input(input!, { target: { value: 'consulta anterior' } });

    const firstSubmission = fireEvent.submit(form);
    await waitFor(() => expect(searchKnowledge).toHaveBeenCalledTimes(1));
    await fireEvent.input(input!, { target: { value: 'consulta nueva' } });
    await fireEvent.submit(form);
    await waitFor(() => expect(searchKnowledge).toHaveBeenCalledTimes(2));
    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      search: searchSummary('newer-search', 'complete', [])
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'newer-search', kind: 'stateChanged', snapshot });
    });
    expect(await screen.findByText('No encontramos evidencia coincidente')).toBeInTheDocument();

    resolveFirstSearch('older-search');
    await firstSubmission;
    expect(screen.getByText('No encontramos evidencia coincidente')).toBeInTheDocument();
  });

  it('keeps the active search locked through unrelated snapshots', async () => {
    activateLocalSearch();
    vi.mocked(searchKnowledge).mockResolvedValueOnce('active-search');
    window.location.hash = '#search';
    render(App);
    const form = await screen.findByRole('search');
    const input = form.querySelector('input');
    expect(input).not.toBeNull();
    await fireEvent.input(input!, { target: { value: 'consulta activa' } });
    await fireEvent.submit(form);
    await waitFor(() => expect(searchKnowledge).toHaveBeenCalledTimes(1));

    snapshot = { ...snapshot, sequence: snapshot.sequence + 1, search: null };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: null, kind: 'stateChanged', snapshot });
    });
    await fireEvent.submit(form);

    expect(searchKnowledge).toHaveBeenCalledTimes(1);
  });

  it('allows an edited query to replace a search with partial results', async () => {
    activateLocalSearch();
    vi.mocked(searchKnowledge)
      .mockResolvedValueOnce('older-search')
      .mockResolvedValueOnce('replacement-search');
    window.location.hash = '#search';
    render(App);
    const form = await screen.findByRole('search');
    const input = form.querySelector('input');
    expect(input).not.toBeNull();
    await fireEvent.input(input!, { target: { value: 'consulta lenta' } });
    await fireEvent.submit(form);
    await waitFor(() => expect(searchKnowledge).toHaveBeenCalledTimes(1));

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      search: searchSummary('older-search', 'partial', [], 'searching')
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: 'older-search', kind: 'stateChanged', snapshot });
    });
    await fireEvent.input(input!, { target: { value: 'consulta nueva' } });
    await fireEvent.submit(form);

    await waitFor(() => expect(searchKnowledge).toHaveBeenCalledTimes(2));
    expect(searchKnowledge).toHaveBeenLastCalledWith('consulta nueva', false);
  });

  it('keeps search visible but explains why it cannot run before local AI is ready', async () => {
    window.location.hash = '#search';
    render(App);
    const form = await screen.findByRole('search');
    const input = form.querySelector('input');
    expect(input).not.toBeNull();
    input!.focus();
    await waitFor(() => expect(input).toHaveFocus());
    await fireEvent.input(input!, { target: { value: 'consulta preparada' } });
    expect(input).toHaveValue('consulta preparada');

    expect(await screen.findByRole('heading', { name: 'Preparando la búsqueda local' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Preparando la búsqueda local' })).toBeDisabled();
    await fireEvent.click(screen.getByRole('button', { name: 'Ver estado de la IA local' }));
    expect(window.location.hash).toBe('#settings/general');
  });

  it('describes a queued model preparation without presenting false download progress', async () => {
    window.location.hash = '#settings/general';
    snapshot.modelInstall = { status: 'queued', downloaded: 0, totalBytes: 0 };
    const { container } = render(App);

    expect(await screen.findByText('Comprobando la IA local de este equipo')).toBeInTheDocument();
    expect(screen.getByText(/Los archivos grandes pueden tardar varios minutos/)).toBeInTheDocument();
    expect(container.querySelector('.model-install-state progress')).not.toBeInTheDocument();
    expect(within(container.querySelector('.model-install-state')!).getByRole('button', { name: 'Cancelar solicitud' })).toBeEnabled();
  });

  it('explains local AI guardrails and lets the user choose another model profile', async () => {
    window.location.hash = '#settings/general';
    snapshot.model = {
      stateSequence: 3,
      profile: 'automatic',
      recommendedModelId: 'gemma-e4b-q4',
      displayName: 'Gemma 4 E4B Q4',
      recommendationReason: null,
      active: true,
      activeModelId: 'gemma-e4b-q4',
      installed: true,
      degraded: false,
      issues: [],
      pendingModelId: null,
      downloadBytes: 3221225472,
      requiredFreeBytes: 4294967296,
      fitsAvailableDisk: true,
      licenseAccepted: true,
      license: 'Gemma',
      licenseUrl: 'https://example.com/license',
      revision: 'synthetic'
    };
    render(App);

    expect(await screen.findByRole('heading', { name: 'IA local de AirWiki' })).toBeInTheDocument();
    expect(screen.getByText('No publica, comparte ni modifica tus documentos fuente.')).toBeInTheDocument();
    expect(screen.getByText('Gemma 4 E4B Q4')).toBeInTheDocument();
    expect(screen.getByText('En uso')).toBeInTheDocument();
    const selector = screen.getByRole('combobox', { name: 'Perfil del modelo' });
    expect(selector).toHaveValue('automatic');

    await fireEvent.change(selector, { target: { value: 'efficient' } });
    await waitFor(() => expect(setModelProfile).toHaveBeenCalledWith('efficient'));
  });

  it('marks an installed selected model as pending restart while the previous model remains active', async () => {
    window.location.hash = '#settings/general';
    activateLocalSearch();
    snapshot.model = {
      ...snapshot.model!,
      recommendedModelId: 'quality-model',
      displayName: 'Synthetic quality model',
      activeModelId: 'synthetic-model',
      pendingModelId: 'quality-model'
    };
    const { container } = render(App);

    await screen.findByRole('heading', { name: 'IA local de AirWiki' });
    const settings = container.querySelector<HTMLElement>('.local-ai-settings');
    expect(settings).not.toBeNull();
    expect(await within(settings!).findAllByText('Reinicio necesario')).toHaveLength(2);
    expect(within(settings!).queryByText('En uso')).not.toBeInTheDocument();
    expect(within(settings!).queryByRole('button', { name: 'Instalar IA local' })).not.toBeInTheDocument();
  });

  it('clears local preparation feedback when a queued request is cancelled', async () => {
    window.location.hash = '#settings/general';
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Instalar IA local' }));
    await waitFor(() => expect(installModels).toHaveBeenCalledOnce());
    expect(screen.queryByText('Preparando la IA local')).not.toBeInTheDocument();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      modelInstall: { status: 'queued', downloaded: 0, totalBytes: 0 }
    };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: null, kind: 'stateChanged', snapshot });
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Cancelar solicitud' }));
    await waitFor(() => expect(cancelModelInstall).toHaveBeenCalledOnce());

    snapshot = { ...snapshot, sequence: snapshot.sequence + 1, modelInstall: null };
    await act(() => {
      snapshotListener?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId: null, kind: 'stateChanged', snapshot });
    });
    expect(screen.queryByText('Comprobando la IA local de este equipo')).not.toBeInTheDocument();
    expect(document.querySelector('.action-message')).not.toBeInTheDocument();
  });

  it('does not report a model failure when the user cancels the native license confirmation', async () => {
    window.location.hash = '#system/models';
    vi.mocked(installModels).mockRejectedValueOnce({
      code: 'invalidInput',
      messageKey: 'humanConfirmationRequired',
      retryable: false
    });
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Instalar IA local' }));

    await waitFor(() => expect(installModels).toHaveBeenCalledOnce());
    expect(screen.queryByText('La IA local necesita atención.')).not.toBeInTheDocument();
  });

  it('keeps graph view selected when opening a graph node', async () => {
    const wiki = snapshot.wikis[0];
    snapshot.knowledge = {
      wikiId: wiki.id, wikiName: wiki.name, version: 'graph-fixture', status: 'ready', errorCount: 0, warningCount: 0,
      reservedPages: [],
      concepts: [{ conceptId: 'concept-atlas', page: { kind: 'concept', path: 'architecture/atlas.md' }, title: 'Atlas concept', description: 'Verified concept', conceptType: 'Reference', tags: [], lifecycle: 'stable', generatedBy: 'airwiki/test', verifiedBy: ['human:test'], sources: [], staleAfter: null, assurance: { trust: 'humanReviewed', freshness: 'notDeclared', verificationOutdated: false }, warnings: [], executionAvailable: false, fingerprint: 'a'.repeat(64) }],
      links: [{ source: { kind: 'index' }, target: { kind: 'concept', path: 'architecture/atlas.md' }, label: 'Verified concept' }]
    };
    render(App);
    const wikiButton = await screen.findByRole('button', { name: /Atlas 2 de 2 revisados/ });
    await fireEvent.click(wikiButton);
    const graphButton = screen.getByRole('button', { name: 'Grafo' });
    await fireEvent.click(graphButton);
    await fireEvent.click(await screen.findByRole('button', { name: 'Atlas concept' }));

    expect(graphButton).toHaveClass('active');
    expect(loadWikiPage).toHaveBeenCalledWith(wiki.id, { kind: 'concept', path: 'architecture/atlas.md' }, 'a'.repeat(64));
    await fireEvent.click(screen.getByRole('button', { name: /^Configuración\./ }));
    expect(await screen.findByRole('button', { name: 'Guardar preferencias' })).toBeDisabled();
  });

  it('offers human verification only for editable managed OKF revisions', async () => {
    const wiki = snapshot.wikis[0];
    wiki.origin = 'importedOkf';
    const fingerprint = 'a'.repeat(64);
    snapshot.knowledge = {
      wikiId: wiki.id, wikiName: wiki.name, version: 'managed-fixture', status: 'ready', errorCount: 0, warningCount: 0,
      reservedPages: [],
      concepts: [{ conceptId: 'managed-concept', page: { kind: 'concept', path: 'memory/decision.md' }, title: 'Decision', description: 'Unverified decision', conceptType: 'Decision', tags: [], lifecycle: 'stable', generatedBy: 'codex/1', verifiedBy: [], sources: [], staleAfter: null, assurance: { trust: 'unverified', freshness: 'notDeclared', verificationOutdated: false }, warnings: [], executionAvailable: false, fingerprint }],
      links: []
    };
    snapshot.knowledgePage = {
      wikiId: wiki.id, page: { kind: 'concept', path: 'memory/decision.md' }, concept: snapshot.knowledge.concepts[0], title: 'Decision', status: 'ready', blocks: [], metadata: [], backlinks: [], truncated: false
    };
    window.location.hash = `#wikis/${wiki.id}`;
    render(App);

    await fireEvent.click(await screen.findByRole('button', { name: 'Marcar como revisado por una persona' }));

    expect(verifyWikiConcept).toHaveBeenCalledWith(wiki.id, 'memory/decision.md', fingerprint);
    expect(loadWikiBundle).toHaveBeenCalledWith(wiki.id);
  });

  it('keeps concept assurance atomic with the loaded page', async () => {
    const wiki = snapshot.wikis[0];
    const first = {
      conceptId: 'first', page: { kind: 'concept' as const, path: 'first.md' }, title: 'First',
      description: '', conceptType: 'Reference', tags: [], lifecycle: 'stable', generatedBy: 'process:first',
      verifiedBy: ['human:first'], sources: [], staleAfter: null,
      assurance: { trust: 'humanReviewed' as const, freshness: 'fresh' as const, verificationOutdated: false },
      warnings: [], executionAvailable: false, fingerprint: 'a'.repeat(64)
    };
    const second = {
      ...first, conceptId: 'second', page: { kind: 'concept' as const, path: 'second.md' }, title: 'Second',
      generatedBy: null, verifiedBy: [], assurance: { trust: 'unverified' as const, freshness: 'notDeclared' as const, verificationOutdated: false },
      fingerprint: 'b'.repeat(64)
    };
    snapshot.knowledge = {
      wikiId: wiki.id, wikiName: wiki.name, version: 'atomic-assurance', status: 'ready',
      reservedPages: [], concepts: [first, second], links: [], errorCount: 0, warningCount: 0
    };
    snapshot.knowledgePage = {
      wikiId: wiki.id, page: first.page, concept: first, title: first.title,
      status: 'ready', blocks: [{ kind: 'paragraph', text: 'First body' }], metadata: [], backlinks: [], truncated: false
    };
    window.location.hash = `#wikis/${wiki.id}`;
    render(App);

    expect(await screen.findByRole('heading', { name: 'First' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'First, first.md, Revisado' })).toHaveAttribute('aria-current', 'page');
    const secondPage = screen.getByRole('button', { name: 'Second, second.md, Revisado' });
    expect(secondPage).not.toHaveAttribute('aria-current');
    const secondPageFocus = vi.spyOn(secondPage, 'focus');
    await fireEvent.mouseDown(secondPage);
    expect(secondPageFocus).toHaveBeenCalledWith({ preventScroll: true });
    expect(screen.getByText('Revisado por una persona')).toBeInTheDocument();
    expect(screen.getByText('process:first')).toBeInTheDocument();

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      knowledgePage: {
        wikiId: wiki.id, page: second.page, concept: second, title: second.title,
        status: 'ready', blocks: [{ kind: 'paragraph', text: 'Second body' }], metadata: [], backlinks: [], truncated: false
      }
    };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot
      });
    });

    expect(screen.getByRole('heading', { name: 'Second' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'First, first.md, Revisado' })).not.toHaveAttribute('aria-current');
    expect(screen.getByRole('button', { name: 'Second, second.md, Revisado' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByText('Reference')).toBeInTheDocument();
    expect(screen.getByText('Sin verificar')).toBeInTheDocument();
    expect(screen.queryByText('Revisado por una persona')).not.toBeInTheDocument();
    expect(screen.queryByText('process:first')).not.toBeInTheDocument();
  });

  it('uses independent Settings sections and returns to the previous Library context', async () => {
    const scrollTo = vi.spyOn(HTMLElement.prototype, 'scrollTo');
    render(App);
    await openSettingsSection('general');
    expect(window.location.hash).toBe('#settings/general');
    await fireEvent.click(screen.getByRole('link', { name: /^Conexiones/ }));
    expect(window.location.hash).toBe('#settings/connections');
    await waitFor(() => expect(scrollTo).toHaveBeenCalledWith({ top: 0, left: 0, behavior: 'auto' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Volver' }));
    expect(window.location.hash).toBe('#library');
    expect(await screen.findByRole('heading', { name: 'Tus wikis' })).toBeInTheDocument();
  });

  it('protects unsaved General preferences before leaving Settings', async () => {
    render(App);
    await openSettingsSection('general');
    await fireEvent.change(screen.getByRole('combobox', { name: 'Al cerrar' }), { target: { value: 'hide_to_tray' } });

    await fireEvent.click(screen.getByRole('button', { name: 'Volver' }));
    const dialog = await screen.findByRole('dialog', { name: '¿Descartar los cambios de General?' });
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Continuar editando' }));
    expect(window.location.hash).toBe('#settings/general');

    await fireEvent.click(screen.getByRole('button', { name: 'Volver' }));
    await fireEvent.click(within(await screen.findByRole('dialog', { name: '¿Descartar los cambios de General?' })).getByRole('button', { name: 'Descartar cambios' }));
    expect(window.location.hash).toBe('#library');
  });

  it('shows sanitized updater failures instead of reverting to idle copy', async () => {
    snapshot.updater = {
      status: 'idle', version: null, releaseNotes: null, issue: 'internal', retryable: true
    };
    window.location.hash = '#system/updates';
    render(App);

    expect(await screen.findByText('No se pudo completar la comprobación de actualización.')).toBeInTheDocument();
    expect(screen.queryByText('Listo para comprobar.')).not.toBeInTheDocument();
  });

  it('keeps updater progress visible until the matching result arrives', async () => {
    window.location.hash = '#system/updates';
    render(App);

    const checkButton = await screen.findByRole('button', { name: 'Comprobar ahora' });
    await fireEvent.click(checkButton);
    const requestId = vi.mocked(checkUpdates).mock.calls[0]?.[0];
    expect(requestId).toEqual(expect.any(String));
    await waitFor(() => {
      expect(screen.getByText(/Comprobando la versión estable/)).toBeInTheDocument();
      expect(checkButton).toBeDisabled();
    });

    snapshot = {
      ...snapshot,
      sequence: snapshot.sequence + 1,
      updater: {
        status: 'idle', version: null, releaseNotes: null, issue: 'invalidManifest', retryable: false
      }
    };
    await act(() => {
      snapshotListener?.({
        schemaVersion: snapshot.schemaVersion,
        sequence: snapshot.sequence,
        requestId: requestId ?? null,
        kind: 'stateChanged',
        snapshot
      });
    });

    expect(screen.getByText('El manifiesto de actualización no es válido.')).toBeInTheDocument();
    expect(checkButton).toBeEnabled();
  });

  it('explains an invalid updater configuration without calling it unsupported', async () => {
    snapshot.updater = {
      status: 'disabled', version: null, releaseNotes: null, issue: 'invalidConfiguration', retryable: false
    };
    window.location.hash = '#system/updates';
    render(App);

    expect(await screen.findByText(/configuración de actualización no válida/)).toBeInTheDocument();
    expect(screen.queryByText('Las actualizaciones no están disponibles en este sistema.')).not.toBeInTheDocument();
  });

  it('lets users change the local-network preference after onboarding', async () => {
    window.location.hash = '#settings/connections';
    render(App);

    const networkPreference = await screen.findByRole('combobox', { name: 'Acceso desde dispositivos cercanos' });
    await fireEvent.change(networkPreference, { target: { value: 'enabled' } });

    expect(updatePreferences).toHaveBeenCalledWith(expect.objectContaining({ lanPreference: 'enabled' }));
    expect(screen.queryByRole('button', { name: 'Guardar preferencias' })).not.toBeInTheDocument();
  });

  it('persists the typed hide-to-tray close behavior and exposes cancel for unsaved previews', async () => {
    window.location.hash = '#system/preferences';
    render(App);

    const closePreference = await screen.findByRole('combobox', { name: 'Al cerrar' });
    const save = screen.getByRole('button', { name: 'Guardar preferencias' });
    const cancel = screen.getByRole('button', { name: 'Cancelar' });
    expect(save).toBeDisabled();
    await fireEvent.change(closePreference, { target: { value: 'hide_to_tray' } });

    await waitFor(() => expect(save).toBeEnabled());
    expect(cancel).toBeEnabled();
    expect(screen.getByText('Cambios sin guardar')).toBeInTheDocument();
    await fireEvent.click(cancel);
    expect(closePreference).toHaveValue('ask');
    expect(save).toBeDisabled();
    await fireEvent.change(closePreference, { target: { value: 'hide_to_tray' } });
    await waitFor(() => expect(save).toBeEnabled());
    await fireEvent.click(save);
    expect(updatePreferences).toHaveBeenCalledWith(expect.objectContaining({ closeBehavior: 'hide_to_tray' }));
  });

  it('preserves unsaved preferences while background snapshots arrive', async () => {
    window.location.hash = '#settings/general';
    render(App);

    const closePreference = await screen.findByRole('combobox', { name: 'Al cerrar' });
    await fireEvent.change(closePreference, { target: { value: 'hide_to_tray' } });
    await fireEvent.click(screen.getByRole('link', { name: /^Conexiones/ }));
    const networkPreference = screen.getByRole('combobox', { name: 'Acceso desde dispositivos cercanos' });
    await fireEvent.change(networkPreference, { target: { value: 'enabled' } });
    const staleSnapshot = structuredClone(snapshot);
    staleSnapshot.sequence += 1;
    await act(() => {
      snapshotListener?.({
        schemaVersion: staleSnapshot.schemaVersion,
        sequence: staleSnapshot.sequence,
        requestId: null,
        kind: 'stateChanged',
        snapshot: staleSnapshot
      });
    });

    expect(updatePreferences).toHaveBeenCalledWith(expect.objectContaining({ lanPreference: 'enabled' }));
    await fireEvent.click(screen.getByRole('link', { name: /^General/ }));
    expect(screen.getByRole('combobox', { name: 'Al cerrar' })).toHaveValue('hide_to_tray');
  });

  it('shows an explicit local-network choice for existing undecided preferences', async () => {
    snapshot.preferences!.lanPreference = 'undecided';
    window.location.hash = '#settings/connections';
    render(App);

    expect(await screen.findByRole('combobox', { name: 'Acceso desde dispositivos cercanos' })).toHaveValue('undecided');
    expect(screen.getByRole('option', { name: 'Preguntar antes de habilitar' })).toBeInTheDocument();
  });

  it('redirects previous top-level routes without retaining the old UI', async () => {
    for (const route of ['#library', '#review', '#home', '#shared/public']) {
      window.location.hash = route;
      render(App);
      expect(await screen.findByRole('heading', { name: 'Tus wikis' })).toBeInTheDocument();
      expect(screen.getByRole('checkbox', { name: 'Buscar también en la red pública' })).not.toBeChecked();
      cleanup();
    }
  });

  it('supports local navigation shortcuts and platform theming', async () => {
    snapshot.platform = 'windows';
    snapshot.preferences!.theme = 'dark';
    render(App);
    await screen.findAllByText('Atlas');
    await fireEvent.keyDown(window, { key: '1', ctrlKey: true });
    expect(window.location.hash).toBe('#library');
    await fireEvent.keyDown(window, { key: ',', ctrlKey: true });
    expect(window.location.hash).toBe('#settings/general');
    expect(document.documentElement.dataset.platform).toBe('windows');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it.each(accessibilityCases)('has no serious or critical accessibility violations in %s/%s/%s', async (locale, theme, route) => {
    snapshot.preferences!.locale = locale;
    snapshot.preferences!.theme = theme;
    window.location.hash = `#${route}`;
    const { container } = render(App);
    await waitFor(() => expect(container.querySelector('.drive-shell')).not.toBeNull());
    const results = await axe.run(container, { rules: { region: { enabled: false } } });
    expect(results.violations.filter((violation) => violation.impact === 'critical' || violation.impact === 'serious')).toEqual([]);
  });
});

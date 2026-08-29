import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ApplicationAccessSummary, PeerSummary, WikiSummary } from './api';
import { message, type MessageArgs } from './i18n';
import { readySnapshot } from './test/fixtures';
import WikiTable from './WikiTable.svelte';

const t = (id: string, args?: MessageArgs) => message('es', id, args);

function setup(
  overrides: Partial<WikiSummary> = {},
  applications: ApplicationAccessSummary[] = [],
  peers: PeerSummary[] = []
) {
  const wiki = { ...readySnapshot().wikis[0], ...overrides };
  const onopen = vi.fn();
  const result = render(WikiTable, {
    wikis: [wiki],
    scans: [],
    sourceIssueCounts: {},
    applications,
    peers,
    t,
    onopen,
    oncreate: vi.fn()
  });
  return { ...result, wiki, onopen };
}

describe('Wiki library shelf', () => {
  afterEach(cleanup);

  it('presents each Wiki as one self-contained, scannable row', () => {
    const { container } = setup();

    expect(screen.getByRole('list', { name: 'Tus wikis' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Atlas 2 de 2 revisados.*Solo tú.*Lista para usar/ })).toBeInTheDocument();
    expect(container.querySelector('.wiki-table-head')).toBeNull();
    expect(Array.from(container.querySelectorAll('.wiki-row-summary > *'), (item) => item.textContent)).toEqual([
      '2 de 2 revisados · 0 borradores · 0 excluidos', '3 elementos detectados'
    ]);
    expect(Array.from(container.querySelectorAll('.wiki-row-exposure-text > span'), (item) => item.textContent)).toEqual([
      'Local', 'LAN Desactivada', 'Internet Desactivada'
    ]);
    expect(screen.queryByText('Abrir Wiki')).not.toBeInTheDocument();
  });

  it('shows enabled LAN and actual public exposure without hiding the recovery state', () => {
    const { container } = setup({
      localOnly: false,
      peerShareable: true,
      internetPublic: true,
      publicAnnouncement: { status: 'advertised', acceptedIndexes: 1 },
      maintenanceRequired: true
    });

    const exposure = Array.from(container.querySelectorAll('.wiki-row-exposure-text > span'));
    expect(exposure[1]).toHaveClass('active');
    expect(exposure[1]).toHaveTextContent('LAN Habilitada');
    expect(exposure[2]).toHaveClass('active');
    expect(exposure[2]).toHaveTextContent('Internet Pública');
    expect(container.querySelector('.wiki-row-status.attention')).toHaveTextContent('Necesita atención');
    expect(container.querySelector('.wiki-row-status.attention')).toHaveTextContent('El contenido publicado necesita una comprobación');
  });

  it('keeps an enabled LAN route private until a trusted peer has a grant', () => {
    const { wiki } = setup({ localOnly: false, peerShareable: true });

    expect(screen.getByRole('button', { name: /Atlas 2 de 2 revisados.*Solo tú/ })).toBeInTheDocument();

    cleanup();
    setup(
      { localOnly: false, peerShareable: true },
      [],
      [{
        peerId: 'peer-a',
        deviceName: 'MacBook',
        platform: 'macOs',
        address: '/ip4/192.0.2.1/tcp/4242',
        trust: 'trusted',
        activity: 'notObserved',
        sasWords: null,
        grantedWikiIds: [wiki.id]
      }]
    );

    const sharedRow = screen.getByRole('button', { name: /Atlas 2 de 2 revisados/ });
    expect(sharedRow).not.toHaveAccessibleName(expect.stringContaining(t('desktop-wiki-private')));
    expect(sharedRow).toHaveAccessibleName(expect.stringContaining(t('desktop-share-nearby')));
  });

  it('keeps public exposure private until its announcement is advertised', () => {
    setup({ localOnly: false, internetPublic: true, publicAnnouncement: { status: 'offline' } });

    expect(screen.getByRole('button', { name: /Atlas 2 de 2 revisados.*Solo tú/ })).toBeInTheDocument();

    cleanup();
    setup({
      localOnly: false,
      internetPublic: true,
      publicAnnouncement: { status: 'advertised', acceptedIndexes: 1 }
    });

    expect(screen.getByRole('button', { name: /Atlas 2 de 2 revisados.*Pública en internet/ })).toBeInTheDocument();
  });

  it('presents AI access separately from network sharing', () => {
    const wiki = readySnapshot().wikis[0];
    const application: ApplicationAccessSummary = {
      appId: 'codex-desktop',
      clientName: 'codex',
      displayName: 'Codex',
      producer: 'OpenAI',
      active: true,
      ownedWikiCount: 1,
      managedBytes: 0,
      grants: [{ wikiId: wiki.id, role: 'owner' }]
    };
    setup({ origin: 'aiMemory', memoryKind: 'personal', localOnly: false, allowExternalAi: true }, [application]);

    const row = screen.getByRole('button', { name: /Atlas 2 de 2 revisados/ });
    expect(row).toHaveAccessibleName(expect.stringContaining(t('desktop-compact-ai-access-count', { count: 1 })));
    expect(row).toHaveAccessibleName(expect.stringContaining(t('desktop-wiki-private')));
  });

  it('opens the Wiki from the full keyboard-sized row target', async () => {
    const { wiki, onopen } = setup();

    await fireEvent.click(screen.getByRole('button', { name: /Atlas 2 de 2 revisados/ }));
    expect(onopen).toHaveBeenCalledWith(wiki.id);
  });
});

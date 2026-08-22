import { mount } from 'svelte';
import App from './App.svelte';
import { installDevelopmentBridge, type DevelopmentBridge, type UiEventEnvelope } from './api';
import './styles.css';
import { readySnapshot } from './test/fixtures';

if (!import.meta.env.DEV) throw new Error('visual fixtures are available only from the development server');

const parameters = new URLSearchParams(window.location.search);
const locale = parameters.get('locale') === 'en' ? 'en' : 'es';
const theme = parameters.get('theme') === 'light' ? 'light' : 'dark';
const platform = parameters.get('platform') === 'windows' ? 'windows' : 'macOs';
const destination = parameters.get('destination') ?? 'home';
const sharedPeerId = '12D3KooSyntheticNearbyNode';
const sharedWikiId = '20000000-0000-4000-8000-000000000001';
const sharedConceptId = '30000000-0000-4000-8000-000000000001';
const snapshot = readySnapshot();
snapshot.platform = platform;
if (snapshot.preferences) {
  snapshot.preferences.locale = locale;
  snapshot.preferences.theme = theme;
  if (parameters.get('onboarding') === '1') snapshot.preferences.completedOnboardingVersion = null;
}
if (parameters.get('maintenance') === '1') {
  snapshot.wikis[0].maintenanceRequired = true;
  if (snapshot.wikiHealth) snapshot.wikiHealth.attentionWikiId = snapshot.wikis[0].id;
}
if (destination === 'review') {
  snapshot.wikis[0].needsReviewCount = 1;
  const review = {
    conceptId: 'synthetic-review', wikiId: snapshot.wikis[0].id, sourceRevision: 4, sourceName: 'operating-guide.md', wikiName: 'Atlas',
    draft: {
      type: 'Procedure' as const, title: 'Safe maintenance window',
      description: 'A verified sequence for routine local maintenance.', language: 'en',
      tags: ['operations', 'local-first'], entities: [], links: [],
      summary: 'Back up local state, verify integrity, apply the change, and confirm recovery.',
      classificationConfidence: 0.92, classificationExplanation: 'The source describes an ordered operational procedure.'
    }
  };
  snapshot.reviews = [review];
  snapshot.reviewEvidence = {
    requestId: 'synthetic-evidence', conceptId: review.conceptId, sourceRevision: review.sourceRevision,
    status: 'ready', excerpts: [
      { ordinal: 0, headingOrPage: 'Preparation', text: 'Create a local backup and verify its checksum before beginning.', truncated: false },
      { ordinal: 1, headingOrPage: 'Recovery', text: 'If validation fails, restore the backup and record the observed state.', truncated: false }
    ], totalChunks: 2, nextOrdinal: null
  };
}
if (destination === 'search') {
  const nearbyPeerId = '12D3KooSyntheticWindowsNode';
  snapshot.model = {
    stateSequence: 3, profile: 'balanced', recommendedModelId: 'synthetic-local-model',
    displayName: 'Local knowledge model', recommendationReason: 'Balanced for this device',
    active: true, installed: true, degraded: false, issues: [], pendingModelId: null,
    downloadBytes: 0, requiredFreeBytes: 0, fitsAvailableDisk: true, licenseAccepted: true,
    license: 'Apache-2.0', licenseUrl: null, revision: 'synthetic'
  };
  snapshot.peers = [{
    peerId: nearbyPeerId, deviceName: 'RUSTICO', platform: 'windows',
    address: '/ip4/192.0.2.1/tcp/4242', trust: 'trusted', activity: 'connected',
    sasWords: null, grantedWikiIds: [snapshot.wikis[0].id]
  }];
  snapshot.search = {
    requestId: 'synthetic-search', status: 'complete', coverage: 'complete', hits: [{
      conceptId: 'synthetic-result', wikiId: snapshot.wikis[0].id,
      title: 'Safe maintenance window', snippet: 'Back up local state and verify integrity before applying a change.',
      headingOrPage: 'Preparation', logicalResourceUri: 'okf://atlas/concepts/safe-maintenance',
      sourceRevision: 4, sourceSha256: 'a'.repeat(64), rank: 0.98, nodeId: snapshot.nodeId ?? 'local', route: 'deviceNetwork',
      assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false }, lifecycle: 'stable'
    }, {
      conceptId: 'synthetic-nearby-result', wikiId: '20000000-0000-4000-8000-000000000002',
      title: 'Windows recovery checklist', snippet: 'Restore the local backup and verify the recovered index.',
      headingOrPage: 'Recovery', logicalResourceUri: 'okf://rustico/concepts/recovery',
      sourceRevision: 2, sourceSha256: 'b'.repeat(64), rank: 0.91, nodeId: nearbyPeerId, route: 'deviceNetwork',
      assurance: { trust: 'machineConfirmed', freshness: 'fresh', verificationOutdated: false }, lifecycle: 'stable'
    }, {
      conceptId: 'synthetic-public-result', wikiId: '20000000-0000-4000-8000-000000000003',
      title: 'Community maintenance guidance', snippet: 'A public procedure for planning a recoverable maintenance window.',
      headingOrPage: 'Community guide', logicalResourceUri: 'okf://public/concepts/maintenance',
      sourceRevision: 7, sourceSha256: 'c'.repeat(64), rank: 0.83, nodeId: '12D3KooSyntheticPublicPublisher', route: 'publicNetwork',
      assurance: { trust: 'unverified', freshness: 'notDeclared', verificationOutdated: false }, lifecycle: 'stable'
    }]
  };
}
if (destination === 'shared') {
  snapshot.model = {
    stateSequence: 3, profile: 'balanced', recommendedModelId: 'synthetic-local-model',
    displayName: 'Local knowledge model', recommendationReason: 'Balanced for this device',
    active: true, installed: true, degraded: false, issues: [], pendingModelId: null,
    downloadBytes: 0, requiredFreeBytes: 0, fitsAvailableDisk: true, licenseAccepted: true,
    license: 'Apache-2.0', licenseUrl: null, revision: 'synthetic'
  };
  snapshot.peers = [{
    peerId: sharedPeerId, deviceName: 'RUSTICO', platform: 'windows', address: '/ip4/192.0.2.1/tcp/4242',
    trust: 'trusted', activity: 'connected', sasWords: null, grantedWikiIds: [sharedWikiId]
  }];
}
if (destination === 'library') {
  snapshot.peers = [{
    peerId: '12D3KooSyntheticMacNode', deviceName: 'Atlas Mac', platform: 'macOs',
    address: '/ip4/192.0.2.2/tcp/4242', trust: 'trusted', activity: 'connected',
    sasWords: null, grantedWikiIds: []
  }, {
    peerId: '12D3KooSyntheticWindowsNode', deviceName: 'RUSTICO', platform: 'windows',
    address: '', trust: 'trusted', activity: 'notObserved', sasWords: null,
    grantedWikiIds: []
  }];
  snapshot.integrations = {
    externalAiWikiCount: 1,
    integrations: [{
      client: 'chatGptDesktop', status: 'configured', detectedVersion: '1.2026.210',
      activityRecent: true, restartRequired: false, mcpSetup: null,
      workflowGuide: { kind: 'nativeSkill', status: 'installed', version: '1', restartRequired: false }
    }, {
      client: 'claudeDesktop', status: 'available', detectedVersion: '0.14.2',
      activityRecent: false, restartRequired: false, mcpSetup: null,
      workflowGuide: { kind: 'mcpInstructions', status: 'builtIn', version: '1', restartRequired: false }
    }, {
      client: 'claudeCode', status: 'configured', detectedVersion: '2.1.227',
      activityRecent: true, restartRequired: false, mcpSetup: null,
      workflowGuide: { kind: 'nativeSkill', status: 'installed', version: '1', restartRequired: false }
    }, {
      client: 'geminiCli', status: 'updateAvailable', detectedVersion: '0.12.0',
      activityRecent: false, restartRequired: true, mcpSetup: null,
      workflowGuide: { kind: 'nativeSkill', status: 'updateAvailable', version: '2', restartRequired: true }
    }, {
      client: 'genericMcp', status: 'configured', detectedVersion: null,
      activityRecent: false, restartRequired: false,
      mcpSetup: { command: '/Applications/AirWiki.app/Contents/MacOS/airwiki-mcp-bridge', args: ['--client', 'generic-mcp'] },
      workflowGuide: { kind: 'mcpInstructions', status: 'builtIn', version: '1', restartRequired: false }
    }]
  };
  snapshot.applicationAccess = [{
    appId: 'codex', displayName: 'Codex', producer: 'codex/1.2', active: true,
    ownedWikiCount: 1, managedBytes: 32768, grants: []
  }];
}
if (destination === 'graph') {
  const wiki = snapshot.wikis[0];
  snapshot.knowledge = {
    wikiId: wiki.id, wikiName: wiki.name, version: 'synthetic-graph', status: 'ready', errorCount: 0, warningCount: 0,
    concepts: [
      { conceptId: 'safe-maintenance', page: { kind: 'concept', path: 'operations/safe-maintenance.md' }, title: 'Safe maintenance', description: 'Verified maintenance guidance.', conceptType: 'Procedure', tags: ['operations'], lifecycle: 'stable', generatedBy: 'airwiki/demo', verifiedBy: ['human:demo'], sources: [], staleAfter: null, assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false }, warnings: [], executionAvailable: false, fingerprint: 'a'.repeat(64) },
      { conceptId: 'recovery', page: { kind: 'concept', path: 'operations/recovery.md' }, title: 'Recovery', description: 'Verified recovery guidance.', conceptType: 'Runbook', tags: ['recovery'], lifecycle: 'stable', generatedBy: 'airwiki/demo', verifiedBy: ['human:demo'], sources: [], staleAfter: null, assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false }, warnings: [], executionAvailable: false, fingerprint: 'b'.repeat(64) }
    ],
    links: [
      { source: { kind: 'index' }, target: { kind: 'concept', path: 'operations/safe-maintenance.md' }, label: 'Maintenance' },
      { source: { kind: 'concept', path: 'operations/safe-maintenance.md' }, target: { kind: 'concept', path: 'operations/recovery.md' }, label: 'Recovery path' },
      { source: { kind: 'log' }, target: { kind: 'concept', path: 'operations/recovery.md' }, label: 'Recorded change' }
    ]
  };
}
if (destination === 'system') {
  snapshot.model = {
    stateSequence: 3, profile: 'balanced', recommendedModelId: 'synthetic-local-model',
    displayName: 'Local knowledge model', recommendationReason: 'Balanced for this device',
    active: true, installed: true, degraded: false, issues: [], pendingModelId: null,
    downloadBytes: 0, requiredFreeBytes: 0, fitsAvailableDisk: true, licenseAccepted: true,
    license: 'Apache-2.0', licenseUrl: null, revision: 'synthetic'
  };
}
window.location.hash = destination === 'review'
  ? `wikis/${snapshot.wikis[0].id}/pending`
  : destination === 'graph' ? `wikis/${snapshot.wikis[0].id}`
  : destination === 'shared' ? 'search'
  : destination === 'library' ? 'wikis' : destination;

let eventSink: ((event: UiEventEnvelope) => void) | null = null;
const bridge: DevelopmentBridge = {
  async connect(onEvent) {
    eventSink = onEvent;
    return snapshot;
  },
  async invoke(_command, arguments_) {
    const requestId = typeof arguments_?.requestId === 'string' ? arguments_.requestId : null;
    if (destination === 'search' && _command === 'search' && requestId && snapshot.search) {
      snapshot.search = { ...snapshot.search, requestId };
    }
    if (destination === 'shared' && _command === 'search' && requestId) {
      snapshot.search = {
        requestId, status: 'complete', coverage: 'complete',
        hits: [{
          conceptId: sharedConceptId, wikiId: sharedWikiId,
          title: 'Ventana de mantenimiento segura',
          snippet: 'Respalda el estado local y verifica su integridad antes de aplicar cambios.',
          headingOrPage: 'Preparación', logicalResourceUri: 'urn:airwiki:shared:maintenance',
          sourceRevision: 4, sourceSha256: 'c'.repeat(64), rank: 1, nodeId: sharedPeerId, route: 'deviceNetwork',
          assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false }, lifecycle: 'stable'
        }]
      };
    }
    if (destination === 'shared' && _command === 'browse_nearby_wiki' && requestId) {
      snapshot.nearbyBrowse = {
        requestId, status: 'available', peerId: sharedPeerId, wikiId: sharedWikiId,
        wikiName: 'Guía operativa compartida', okfCompatibility: { kind: 'declaredV02' }, nextCursor: null,
        workspaceSupported: true,
        workspaceFingerprint: '0'.repeat(64),
        reservedPages: [
          { page: { kind: 'index' }, logicalPath: 'index.md', title: 'Índice', fingerprint: 'a'.repeat(64) },
          { page: { kind: 'log' }, logicalPath: 'log.md', title: 'Historial', fingerprint: 'b'.repeat(64) }
        ],
        documents: [
          { page: { kind: 'concept', conceptId: sharedConceptId }, logicalPath: 'operations/maintenance.md', title: 'Ventana de mantenimiento segura', fingerprint: 'c'.repeat(64) },
          { page: { kind: 'concept', conceptId: '30000000-0000-4000-8000-000000000002' }, logicalPath: 'operations/recovery.md', title: 'Recuperación', fingerprint: 'd'.repeat(64) }
        ],
        links: [
          { source: { kind: 'index' }, target: { kind: 'concept', conceptId: sharedConceptId }, label: 'Mantenimiento' },
          { source: { kind: 'concept', conceptId: sharedConceptId }, target: { kind: 'concept', conceptId: '30000000-0000-4000-8000-000000000002' }, label: 'Recuperación' }
        ],
        nextGraphCursor: null,
        page: {
          descriptor: { page: { kind: 'concept', conceptId: sharedConceptId }, logicalPath: 'operations/maintenance.md', title: 'Ventana de mantenimiento segura', fingerprint: 'c'.repeat(64) },
          blocks: [
            { kind: 'heading', level: 1, text: 'Ventana de mantenimiento segura' },
            { kind: 'paragraph', text: 'Respalda el estado local y verifica su integridad antes de aplicar cambios.' },
            { kind: 'listItem', ordered: true, text: 'Crea un respaldo local verificable.' },
            { kind: 'listItem', ordered: true, text: 'Aplica el cambio y confirma la recuperación.' }
          ],
          metadata: [['type', 'Procedure'], ['status', 'stable']],
          backlinks: [{ kind: 'index' }],
          truncated: false
        },
        appendFailed: false,
        concepts: [{
          conceptId: sharedConceptId, conceptType: 'Procedure', title: 'Ventana de mantenimiento segura',
          description: 'Procedimiento validado para mantener el servicio recuperable.', language: 'es',
          tags: ['operaciones', 'recuperación'], summary: 'Respalda el estado local, verifica su integridad, aplica el cambio y confirma la recuperación.',
          sourceRevision: 4, lifecycle: 'stable',
          assurance: { trust: 'humanReviewed', freshness: 'fresh', verificationOutdated: false }
        }, {
          conceptId: '30000000-0000-4000-8000-000000000002', conceptType: 'Runbook', title: 'Recuperación',
          description: 'Pasos para volver al estado anterior cuando falla la validación.', language: 'es',
          tags: ['recuperación'], summary: 'Restaura el respaldo, valida el resultado y registra el estado observado.',
          sourceRevision: 2, lifecycle: 'stable',
          assurance: { trust: 'machineConfirmed', freshness: 'fresh', verificationOutdated: false }
        }]
      };
    }
    if (_command === 'manage_integration' && requestId) {
      snapshot.integrationRequestId = null;
      snapshot.integrationCompletedRequestId = requestId;
    }
    queueMicrotask(() => {
      snapshot.sequence += 1;
      eventSink?.({ schemaVersion: snapshot.schemaVersion, sequence: snapshot.sequence, requestId, kind: 'stateChanged', snapshot });
    });
    return undefined;
  }
};
installDevelopmentBridge(bridge);

const target = document.getElementById('app');
if (!target) throw new Error('visual fixture root is missing');
mount(App, { target });

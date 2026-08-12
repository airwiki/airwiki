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
  snapshot.search = {
    requestId: 'synthetic-search', status: 'complete', coverage: 'complete', hits: [{
      conceptId: 'synthetic-result', wikiId: snapshot.wikis[0].id,
      title: 'Safe maintenance window', snippet: 'Back up local state and verify integrity before applying a change.',
      headingOrPage: 'Preparation', logicalResourceUri: 'okf://atlas/concepts/safe-maintenance',
      sourceRevision: 4, sourceSha256: 'a'.repeat(64), rank: 0.98, nodeId: snapshot.nodeId ?? 'local'
    }]
  };
}
if (destination === 'graph') {
  const wiki = snapshot.wikis[0];
  snapshot.knowledge = {
    wikiId: wiki.id, wikiName: wiki.name, version: 'synthetic-graph', status: 'ready', errorCount: 0, warningCount: 0,
    concepts: [
      { page: { kind: 'concept', id: 'safe-maintenance' }, title: 'Safe maintenance', description: 'Verified maintenance guidance.', conceptType: 'Procedure', tags: ['operations'] },
      { page: { kind: 'concept', id: 'recovery' }, title: 'Recovery', description: 'Verified recovery guidance.', conceptType: 'Runbook', tags: ['recovery'] }
    ],
    links: [
      { source: { kind: 'index' }, target: { kind: 'concept', id: 'safe-maintenance' }, label: 'Maintenance' },
      { source: { kind: 'concept', id: 'safe-maintenance' }, target: { kind: 'concept', id: 'recovery' }, label: 'Recovery path' },
      { source: { kind: 'log' }, target: { kind: 'concept', id: 'recovery' }, label: 'Recorded change' }
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
  : destination === 'library' ? 'wikis' : destination;

let eventSink: ((event: UiEventEnvelope) => void) | null = null;
const bridge: DevelopmentBridge = {
  async connect(onEvent) {
    eventSink = onEvent;
    return snapshot;
  },
  async invoke(_command, arguments_) {
    const requestId = typeof arguments_?.requestId === 'string' ? arguments_.requestId : null;
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

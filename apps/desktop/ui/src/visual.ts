import { mount } from 'svelte';
import App from './App.svelte';
import { installDevelopmentBridge, type DevelopmentBridge, type UiEventEnvelope } from './api';
import './styles.css';
import { readySnapshot } from './test/fixtures';

if (!import.meta.env.DEV) throw new Error('visual fixtures are available only from the development server');

const parameters = new URLSearchParams(window.location.search);
const locale = parameters.get('locale') === 'en' ? 'en' : 'es';
const theme = parameters.get('theme') === 'light' ? 'light' : 'dark';
const destination = parameters.get('destination') ?? 'library';
const snapshot = readySnapshot();
if (snapshot.preferences) {
  snapshot.preferences.locale = locale;
  snapshot.preferences.theme = theme;
}
if (destination === 'review') {
  const review = {
    conceptId: 'synthetic-review', sourceRevision: 4, sourceName: 'operating-guide.md', collectionName: 'Atlas',
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
      conceptId: 'synthetic-result', collectionId: snapshot.collections[0].id,
      title: 'Safe maintenance window', snippet: 'Back up local state and verify integrity before applying a change.',
      headingOrPage: 'Preparation', logicalResourceUri: 'okf://atlas/concepts/safe-maintenance',
      sourceRevision: 4, sourceSha256: 'a'.repeat(64), rank: 0.98, nodeId: snapshot.nodeId ?? 'local'
    }]
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
window.location.hash = destination === 'system' ? 'system/preferences' : destination;

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

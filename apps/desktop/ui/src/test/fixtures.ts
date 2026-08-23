import type { AppSnapshot } from '../generated/ui-contract';

export function readySnapshot(): AppSnapshot {
  return {
    schemaVersion: 1,
    sequence: 7,
    platform: 'macOs',
    phase: 'ready',
    nodeId: '12D3KooSyntheticLocalNode',
    mcpUrl: 'http://127.0.0.1:43123/mcp',
    blockedPublicPublishers: [],
    hardware: {
      os: 'macOS', architecture: 'arm64', totalMemoryBytes: 17179869184,
      availableMemoryBytes: 8589934592, availableDiskBytes: 21474836480,
      avx2: false, metalAvailable: true, supportedTarget: true, canInstall: true, issues: []
    },
    wikis: [{
      id: '10000000-0000-4000-8000-000000000001', name: 'Atlas', documentCount: 3,
      needsReviewCount: 0, publishedCount: 2, failedCount: 0, localOnly: true,
      peerShareable: false, allowExternalAi: false, internetPublic: false,
      publicDescription: '', publicLanguages: '', publicAnnouncement: { status: 'offline' },
      maintenanceRequired: false, origin: 'folder', indexingMode: 'continuous', okfVersion: '0.2',
      declaredOkfVersion: '0.2', okfCompatibility: { kind: 'declaredV02' }, managedSizeBytes: 0,
      staleConceptCount: 0, outdatedVerificationCount: 0, metadataWarningCount: 0,
      trustSummary: 'humanReviewed', restrictions: []
    }],
    wikiScans: [], reviews: [], reanalyzingReviewIds: [], sourceIssues: [], peers: [],
    model: null, modelInstall: null, search: null, publicBrowse: null, nearbyBrowse: null, reviewEvidence: null,
    knowledge: null, knowledgePage: null,
    preferences: { completedOnboardingVersion: 1, locale: 'es', theme: 'system', lanPreference: 'disabled', closeBehavior: 'ask', automaticUpdateChecks: false },
    autostart: 'disabled',
    wikiHealth: { generation: 1, status: 'ready', errorCount: 0, warningCount: 0, updatingCount: 0, attentionWikiId: null, checked: true },
    guidedRepair: null,
    connectivity: { systemPermission: 'notApplicable', networkProfile: 'notApplicable', firewall: 'notApplicable', firewallHelper: 'notApplicable' },
    lanRuntime: { listener: 'stopped', discovery: 'disabled', addressCount: 0 },
    firewallOperation: null, integrations: { integrations: [], externalAiWikiCount: 0 },
    integrationRequestId: null, integrationCompletedRequestId: null,
    applicationAccess: [], pendingComputations: [], completedComputations: [],
    updater: { status: 'idle', version: null, releaseNotes: null, issue: null, retryable: false },
    notice: null
  };
}

import { Channel, invoke } from '@tauri-apps/api/core';

export interface AppSnapshot {
  schemaVersion: number;
  sequence: number;
  phase: 'starting' | 'ready';
  collections: CollectionSummary[];
  reviews: ReviewSummary[];
  sourceIssues: SourceIssueSummary[];
  peers: PeerSummary[];
  model: ModelSummary | null;
  search: SearchSummary | null;
  reviewEvidence: ReviewEvidenceSummary | null;
  notice: { level: string; message: string } | null;
}

export interface UiEventEnvelope {
  schemaVersion: number;
  sequence: number;
  kind: string;
  snapshot: AppSnapshot;
}

export interface CollectionSummary { id: string; name: string; documentCount: number; needsReviewCount: number; publishedCount: number; failedCount: number; localOnly: boolean; peerShareable: boolean; allowExternalAi: boolean; internetPublic: boolean; }
export type ConceptType = 'Document' | 'Policy' | 'Procedure' | 'Runbook' | 'Reference' | 'Report';
export interface SuggestedEntity { name: string; kind: string; }
export interface SuggestedLink { label: string; target: string; }
export interface EnrichmentDraft {
  type: ConceptType;
  title: string;
  description: string;
  language: string;
  tags: string[];
  entities: SuggestedEntity[];
  links: SuggestedLink[];
  summary: string;
  classification_confidence: number;
  classification_explanation: string;
}
export interface ReviewSummary { conceptId: string; sourceRevision: number; sourceName: string; collectionName: string; draft: EnrichmentDraft; }
export interface ReviewExcerptSummary { ordinal: number; headingOrPage: string; text: string; truncated: boolean; }
export interface ReviewEvidenceSummary { requestId: string; conceptId: string; sourceRevision: number; status: 'ready' | 'stale' | 'missing' | 'failed'; excerpts: ReviewExcerptSummary[]; totalChunks: number; nextOrdinal: number | null; }
export interface SourceIssueSummary { collectionId: string; sourceName: string; collectionName: string; code: string; }
export interface PeerSummary { peerId: string; deviceName: string | null; trust: string; activity: string; }
export interface ModelSummary { displayName: string | null; active: boolean; installed: boolean; degraded: boolean; downloadBytes: number; requiredFreeBytes: number; fitsAvailableDisk: boolean; licenseAccepted: boolean; }
export interface SearchSummary { requestId: string; status: 'searching' | 'complete' | 'failed'; hits: SearchHitSummary[]; coverage: string; }
export interface SearchHitSummary { title: string; snippet: string; headingOrPage: string; logicalResourceUri: string; rank: number; }

export async function connect(onEvent: (event: UiEventEnvelope) => void): Promise<AppSnapshot> {
  const events = new Channel<UiEventEnvelope>();
  events.onmessage = onEvent;
  return invoke<AppSnapshot>('connect', { events });
}

export async function installModels(): Promise<void> {
  return invoke('install_models');
}

export async function cancelModelInstall(): Promise<void> {
  return invoke('cancel_model_install');
}

export interface FolderSelection { token: string; displayPath: string; }

export async function pickCollectionFolder(): Promise<FolderSelection | null> {
  return invoke('pick_collection_folder');
}

export async function addCollection(name: string, folderToken: string): Promise<void> {
  return invoke('add_collection', { name, folderToken });
}

export async function rescanCollection(collectionId: string): Promise<void> {
  return invoke('rescan_collection', { collectionId });
}

export async function searchKnowledge(question: string, publicNetwork: boolean): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('search', { requestId, question, topK: 8, publicNetwork });
  return requestId;
}

export async function loadReviewEvidence(review: ReviewSummary, afterOrdinal: number | null = null): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('load_review_evidence', {
    requestId,
    conceptId: review.conceptId,
    sourceRevision: review.sourceRevision,
    afterOrdinal
  });
  return requestId;
}

export async function approveReview(conceptId: string, sourceRevision: number, draft: EnrichmentDraft): Promise<void> {
  return invoke('approve_review', { conceptId, sourceRevision, draft });
}

export async function rejectReview(conceptId: string): Promise<void> {
  return invoke('reject_review', { conceptId });
}

export async function reanalyzeReview(conceptId: string): Promise<void> {
  return invoke('reanalyze_review', { conceptId });
}

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
  notice: { level: string; message: string } | null;
}

export interface UiEventEnvelope {
  schemaVersion: number;
  sequence: number;
  kind: string;
  snapshot: AppSnapshot;
}

export interface CollectionSummary { id: string; name: string; documentCount: number; needsReviewCount: number; publishedCount: number; failedCount: number; localOnly: boolean; peerShareable: boolean; allowExternalAi: boolean; internetPublic: boolean; }
export interface ReviewSummary { conceptId: string; sourceRevision: number; sourceName: string; collectionName: string; }
export interface SourceIssueSummary { collectionId: string; sourceName: string; collectionName: string; code: string; }
export interface PeerSummary { peerId: string; deviceName: string | null; trust: string; activity: string; }
export interface ModelSummary { displayName: string | null; active: boolean; installed: boolean; degraded: boolean; downloadBytes: number; requiredFreeBytes: number; fitsAvailableDisk: boolean; licenseAccepted: boolean; }

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

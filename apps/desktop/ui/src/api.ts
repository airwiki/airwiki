import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  AppSnapshot,
  CollectionPolicyInput,
  EnrichmentDraft,
  FolderSelection,
  IntegrationActionInput,
  KnowledgePageInput,
  PreferencesInput,
  ReviewSummary,
  SystemDestination,
  UiEventEnvelope
} from './generated/ui-contract';

export type * from './generated/ui-contract';

export async function connect(onEvent: (event: UiEventEnvelope) => void): Promise<AppSnapshot> {
  const events = new Channel<UiEventEnvelope>();
  events.onmessage = onEvent;
  return invoke<AppSnapshot>('connect', { events });
}

export async function installModels(licensesConfirmed: boolean): Promise<void> {
  return invoke('install_models', { licensesConfirmed });
}

export async function cancelModelInstall(): Promise<void> {
  return invoke('cancel_model_install');
}

export async function pickCollectionFolder(): Promise<FolderSelection | null> {
  return invoke('pick_collection_folder');
}

export async function addCollection(name: string, folderToken: string): Promise<void> {
  return invoke('add_collection', { name, folderToken });
}

export async function relinkCollection(collectionId: string, folderToken: string): Promise<void> {
  return invoke('relink_collection', { collectionId, folderToken });
}

export async function updateCollectionPolicy(collectionId: string, policy: CollectionPolicyInput): Promise<void> {
  return invoke('update_collection_policy', { collectionId, policy });
}

export async function pairPeer(peerId: string): Promise<void> {
  return invoke('pair_peer', { peerId });
}

export async function confirmPairing(peerId: string, accepted: boolean): Promise<void> {
  return invoke('confirm_pairing', { peerId, accepted });
}

export async function revokePeer(peerId: string): Promise<void> {
  return invoke('revoke_peer', { peerId });
}

export async function setCollectionGrant(peerId: string, collectionId: string, granted: boolean): Promise<void> {
  return invoke('set_collection_grant', { peerId, collectionId, granted });
}

export async function manageIntegration(requestId: string, action: IntegrationActionInput): Promise<void> {
  return invoke('manage_integration', { requestId, action });
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

export async function loadKnowledgeBundle(collectionId: string): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('load_knowledge_bundle', { requestId, collectionId });
  return requestId;
}

export async function loadKnowledgePage(collectionId: string, page: KnowledgePageInput): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('load_knowledge_page', { requestId, collectionId, page });
  return requestId;
}

export async function updatePreferences(preferences: PreferencesInput): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('update_preferences', { requestId, preferences });
  return requestId;
}

export async function refreshAutostart(requestId: string): Promise<void> {
  return invoke('refresh_autostart', { requestId });
}

export async function setAutostart(requestId: string, enabled: boolean): Promise<void> {
  return invoke('set_autostart', { requestId, enabled });
}

export async function checkUpdates(requestId: string): Promise<void> {
  return invoke('check_updates', { requestId });
}

export async function downloadUpdate(requestId: string): Promise<void> {
  return invoke('download_update', { requestId });
}

export async function installUpdate(requestId: string): Promise<void> {
  return invoke('install_update', { requestId });
}

export async function refreshWikiHealth(requestId: string): Promise<void> {
  return invoke('refresh_wiki_health', { requestId });
}

export async function refreshConnectivity(requestId: string): Promise<void> {
  return invoke('refresh_connectivity', { requestId });
}

export async function configureFirewall(requestId: string, install: boolean): Promise<void> {
  return invoke('configure_firewall', { requestId, install });
}

export async function openSystemDestination(requestId: string, destination: SystemDestination): Promise<void> {
  return invoke('open_system_destination', { requestId, destination });
}

export async function openExternalLink(url: string, confirmed: boolean): Promise<void> {
  return invoke('open_external_link', { url, confirmed });
}

export async function hideToTray(): Promise<void> {
  return invoke('hide_to_tray');
}

export async function quitCompletely(): Promise<void> {
  return invoke('quit_completely');
}

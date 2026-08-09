import { Channel, invoke as tauriInvoke } from '@tauri-apps/api/core';
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

export interface DevelopmentBridge {
  connect(onEvent: (event: UiEventEnvelope) => void): Promise<AppSnapshot>;
  invoke(command: string, arguments_: Record<string, unknown> | undefined): Promise<unknown>;
}

let developmentBridge: DevelopmentBridge | null = null;

export function installDevelopmentBridge(bridge: DevelopmentBridge): void {
  if (!import.meta.env.DEV) throw new Error('the development bridge is unavailable in production');
  developmentBridge = bridge;
}

async function invoke<T>(command: string, arguments_?: Record<string, unknown>): Promise<T> {
  if (developmentBridge) return developmentBridge.invoke(command, arguments_) as Promise<T>;
  return tauriInvoke<T>(command, arguments_);
}

export async function connect(onEvent: (event: UiEventEnvelope) => void): Promise<AppSnapshot> {
  if (developmentBridge) return developmentBridge.connect(onEvent);
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

export async function updatePublicCollectionProfile(
  collectionId: string,
  description: string,
  languages: string[]
): Promise<void> {
  return invoke('update_public_collection_profile', { collectionId, description, languages });
}

export async function addFederationIndex(peerId: string, address: string): Promise<void> {
  return invoke('add_federation_index', { peerId, address });
}

export async function removeFederationIndex(peerId: string): Promise<void> {
  return invoke('remove_federation_index', { peerId });
}

export async function browsePublicCollection(
  publisherId: string,
  collectionId: string,
  cursor: string | null = null
): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('browse_public_collection', { requestId, publisherId, collectionId, cursor });
  return requestId;
}

export async function setPublicPublisherBlocked(publisherId: string, blocked: boolean): Promise<void> {
  return invoke('set_public_publisher_blocked', { publisherId, blocked });
}

export async function dialPeer(address: string): Promise<void> {
  return invoke('dial_peer', { address });
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

export async function prepareGuidedWikiRepair(collectionId: string): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('prepare_guided_wiki_repair', { requestId, collectionId });
  return requestId;
}

export async function executeGuidedWikiRepair(collectionId: string, confirmed: boolean): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('execute_guided_wiki_repair', { requestId, collectionId, confirmed });
  return requestId;
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

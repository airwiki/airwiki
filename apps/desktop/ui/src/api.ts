import { Channel, invoke as tauriInvoke } from '@tauri-apps/api/core';
import type {
  AppSnapshot,
  WikiPolicyInput,
  EnrichmentDraft,
  FolderSelection,
  IntegrationActionInput,
  KnowledgePageInput,
  OkfImportSummary,
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

export async function installModels(): Promise<void> {
  return invoke('install_models');
}

export async function cancelModelInstall(): Promise<void> {
  return invoke('cancel_model_install');
}

export async function pickWikiFolder(): Promise<FolderSelection | null> {
  return invoke('pick_wiki_folder');
}

export async function pickOkfImport(zip: boolean): Promise<FolderSelection | null> {
  return invoke('pick_okf_import', { zip });
}

export async function validateOkfImport(selectionToken: string): Promise<OkfImportSummary> {
  return invoke('validate_okf_import', { selectionToken });
}

export async function importOkf(name: string, selectionToken: string): Promise<void> {
  return invoke('import_okf', { name, selectionToken });
}

export async function setWikiIndexing(wikiId: string, continuous: boolean): Promise<void> {
  return invoke('set_wiki_indexing', { wikiId, continuous });
}

export async function addWiki(
  name: string,
  folderToken: string,
  continuousIndexing = true,
): Promise<void> {
  return invoke('add_wiki', { name, folderToken, continuousIndexing });
}

export async function relinkWiki(wikiId: string, folderToken: string): Promise<void> {
  return invoke('relink_wiki', { wikiId, folderToken });
}

export async function updateWikiPolicy(wikiId: string, policy: WikiPolicyInput): Promise<void> {
  return invoke('update_wiki_policy', { wikiId, policy });
}

export async function updatePublicWikiProfile(
  wikiId: string,
  description: string,
  languages: string[]
): Promise<void> {
  return invoke('update_public_wiki_profile', { wikiId, description, languages });
}

export async function addFederationIndex(peerId: string, address: string): Promise<void> {
  return invoke('add_federation_index', { peerId, address });
}

export async function removeFederationIndex(peerId: string): Promise<void> {
  return invoke('remove_federation_index', { peerId });
}

export async function browsePublicWiki(
  publisherId: string,
  wikiId: string,
  cursor: string | null = null
): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('browse_public_wiki', { requestId, publisherId, wikiId, cursor });
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

export async function allowPeerPairingAgain(peerId: string): Promise<void> {
  return invoke('allow_peer_pairing_again', { peerId });
}

export async function setWikiGrant(peerId: string, wikiId: string, granted: boolean): Promise<void> {
  return invoke('set_wiki_grant', { peerId, wikiId, granted });
}

export async function manageIntegration(requestId: string, action: IntegrationActionInput): Promise<void> {
  return invoke('manage_integration', { requestId, action });
}

export async function rescanWiki(wikiId: string): Promise<void> {
  return invoke('rescan_wiki', { wikiId });
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

export async function loadWikiBundle(wikiId: string): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('load_wiki_bundle', { requestId, wikiId });
  return requestId;
}

export async function loadWikiPage(wikiId: string, page: KnowledgePageInput): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('load_wiki_page', { requestId, wikiId, page });
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

export async function prepareGuidedWikiRepair(wikiId: string): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('prepare_guided_wiki_repair', { requestId, wikiId });
  return requestId;
}

export async function executeGuidedWikiRepair(wikiId: string): Promise<string> {
  const requestId = crypto.randomUUID();
  await invoke('execute_guided_wiki_repair', { requestId, wikiId });
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

export async function openExternalLink(url: string): Promise<void> {
  return invoke('open_external_link', { url });
}

export async function hideToTray(): Promise<void> {
  return invoke('hide_to_tray');
}

export async function quitCompletely(): Promise<void> {
  return invoke('quit_completely');
}

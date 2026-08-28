import type { ApplicationAccessSummary, PeerSummary, WikiSummary } from './api';

export function wikiProjectMemoryBlocked(wiki: WikiSummary): boolean {
  return wiki.memoryKind === 'project' && wiki.projectMemoryHealth !== 'active';
}

export function wikiExternalAccessBlocked(wiki: WikiSummary): boolean {
  return wiki.restrictions.length > 0 || wikiProjectMemoryBlocked(wiki);
}

export function applicationCanAccessWiki(
  application: ApplicationAccessSummary,
  wiki: WikiSummary
): boolean {
  return !wikiExternalAccessBlocked(wiki)
    && wiki.allowExternalAi
    && application.active
    && application.grants.some((grant) => grant.wikiId === wiki.id);
}

export function wikiHasApplicationAccess(
  wiki: WikiSummary,
  applications: ApplicationAccessSummary[]
): boolean {
  return applications.some((application) => applicationCanAccessWiki(application, wiki));
}

export function wikiHasLanAccess(wiki: WikiSummary, peers: PeerSummary[]): boolean {
  return !wikiExternalAccessBlocked(wiki)
    && wiki.peerShareable
    && peers.some((peer) => (
      peer.trust === 'trusted' && peer.grantedWikiIds.includes(wiki.id)
    ));
}

export function wikiHasPublicAccess(wiki: WikiSummary): boolean {
  return !wikiExternalAccessBlocked(wiki)
    && wiki.internetPublic
    && wiki.publicAnnouncement.status === 'advertised';
}

export function wikiIsPrivate(wiki: WikiSummary, peers: PeerSummary[]): boolean {
  return !wikiHasLanAccess(wiki, peers) && !wikiHasPublicAccess(wiki);
}

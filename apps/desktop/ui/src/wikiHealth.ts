import type { WikiSummary } from './api';

export function wikiRequiresAttention(wiki: WikiSummary): boolean {
  return wiki.failedCount > 0
    || wiki.maintenanceRequired
    || wiki.needsReviewCount > 0
    || (wiki.documentCount > 0 && wiki.publishedCount === 0)
    || wiki.staleConceptCount > 0
    || wiki.outdatedVerificationCount > 0
    || wiki.metadataWarningCount > 0
    || wiki.okfCompatibility.kind === 'legacyV01'
    || wiki.okfCompatibility.kind === 'futureRestricted'
    || (wiki.memoryKind === 'project' && wiki.projectMemoryHealth !== 'active');
}

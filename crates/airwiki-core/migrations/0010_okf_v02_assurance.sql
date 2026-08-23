ALTER TABLE collections ADD COLUMN declared_okf_version TEXT;
ALTER TABLE collections ADD COLUMN okf_compatibility TEXT NOT NULL DEFAULT 'legacy_v01'
    CHECK(okf_compatibility IN ('declared_v02','undeclared_v02_compatible','legacy_v01','future_restricted'));
ALTER TABLE collections ADD COLUMN managed_size_bytes INTEGER NOT NULL DEFAULT 0;

ALTER TABLE okf_concept_projection ADD COLUMN stale_after TEXT;
ALTER TABLE okf_concept_projection ADD COLUMN trust_tier TEXT NOT NULL DEFAULT 'unverified'
    CHECK(trust_tier IN ('unverified','machine_confirmed','human_reviewed'));
ALTER TABLE okf_concept_projection ADD COLUMN freshness_state TEXT NOT NULL DEFAULT 'not_declared'
    CHECK(freshness_state IN ('not_declared','fresh','stale','invalid'));
ALTER TABLE okf_concept_projection ADD COLUMN verification_outdated INTEGER NOT NULL DEFAULT 0;
ALTER TABLE okf_concept_projection ADD COLUMN warnings_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE application_capabilities ADD COLUMN producer TEXT NOT NULL DEFAULT 'airwiki/legacy';
ALTER TABLE application_capabilities ADD COLUMN managed_bytes INTEGER NOT NULL DEFAULT 0;

CREATE TABLE managed_bundle_mutations (
    id TEXT PRIMARY KEY NOT NULL,
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    logical_path TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('prepared','file_replaced','committed','recovery_required')),
    previous_fingerprint TEXT,
    replacement_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX managed_bundle_mutations_recovery
    ON managed_bundle_mutations(state, updated_at);

UPDATE collections SET declared_okf_version='0.1',okf_compatibility='legacy_v01'
WHERE okf_version='0.1';
UPDATE collections SET declared_okf_version='0.2',okf_compatibility='declared_v02'
WHERE okf_version='0.2';

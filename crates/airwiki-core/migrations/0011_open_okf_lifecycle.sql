ALTER TABLE okf_concept_projection RENAME TO okf_concept_projection_restricted;

CREATE TABLE okf_concept_projection (
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    concept_id TEXT NOT NULL,
    logical_path TEXT NOT NULL,
    concept_type TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    tags_json TEXT NOT NULL DEFAULT '[]',
    lifecycle_status TEXT NOT NULL,
    generation_json TEXT,
    verifications_json TEXT NOT NULL DEFAULT '[]',
    provenance_json TEXT NOT NULL DEFAULT '[]',
    version TEXT,
    fingerprint TEXT NOT NULL,
    unknown_frontmatter_json TEXT NOT NULL DEFAULT '{}',
    indexed_at TEXT NOT NULL,
    stale_after TEXT,
    trust_tier TEXT NOT NULL DEFAULT 'unverified'
        CHECK(trust_tier IN ('unverified','machine_confirmed','human_reviewed')),
    freshness_state TEXT NOT NULL DEFAULT 'not_declared'
        CHECK(freshness_state IN ('not_declared','fresh','stale','invalid')),
    verification_outdated INTEGER NOT NULL DEFAULT 0,
    warnings_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY(collection_id, logical_path),
    UNIQUE(collection_id, concept_id)
);

INSERT INTO okf_concept_projection(
    collection_id,concept_id,logical_path,concept_type,title,description,tags_json,
    lifecycle_status,generation_json,verifications_json,provenance_json,version,
    fingerprint,unknown_frontmatter_json,indexed_at,stale_after,trust_tier,
    freshness_state,verification_outdated,warnings_json
)
SELECT
    collection_id,concept_id,logical_path,concept_type,title,description,tags_json,
    lifecycle_status,generation_json,verifications_json,provenance_json,version,
    fingerprint,unknown_frontmatter_json,indexed_at,stale_after,trust_tier,
    freshness_state,verification_outdated,warnings_json
FROM okf_concept_projection_restricted;

DROP TABLE okf_concept_projection_restricted;
CREATE INDEX okf_projection_collection_status
    ON okf_concept_projection(collection_id, lifecycle_status);

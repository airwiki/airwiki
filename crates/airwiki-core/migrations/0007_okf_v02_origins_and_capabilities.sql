ALTER TABLE collections ADD COLUMN origin TEXT NOT NULL DEFAULT 'folder'
    CHECK(origin IN ('folder','imported_okf','ai_memory'));
ALTER TABLE collections ADD COLUMN indexing_mode TEXT NOT NULL DEFAULT 'continuous'
    CHECK(indexing_mode IN ('continuous','manual','not_applicable'));
ALTER TABLE collections ADD COLUMN okf_version TEXT NOT NULL DEFAULT '0.1';

CREATE TABLE okf_concept_projection (
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    concept_id TEXT NOT NULL,
    logical_path TEXT NOT NULL,
    concept_type TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    tags_json TEXT NOT NULL DEFAULT '[]',
    lifecycle_status TEXT NOT NULL CHECK(lifecycle_status IN ('draft','stable','deprecated')),
    generation_json TEXT,
    verifications_json TEXT NOT NULL DEFAULT '[]',
    provenance_json TEXT NOT NULL DEFAULT '[]',
    version TEXT,
    fingerprint TEXT NOT NULL,
    unknown_frontmatter_json TEXT NOT NULL DEFAULT '{}',
    indexed_at TEXT NOT NULL,
    PRIMARY KEY(collection_id, logical_path),
    UNIQUE(collection_id, concept_id)
);
CREATE INDEX okf_projection_collection_status
    ON okf_concept_projection(collection_id, lifecycle_status);

CREATE TABLE application_capabilities (
    app_id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    owner_kind TEXT NOT NULL,
    secret_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    last_used_at TEXT,
    revoked_at TEXT
);

CREATE TABLE application_wiki_grants (
    app_id TEXT NOT NULL REFERENCES application_capabilities(app_id) ON DELETE CASCADE,
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK(role IN ('owner','reader','editor')),
    granted_at TEXT NOT NULL,
    confirmed_at TEXT NOT NULL,
    PRIMARY KEY(app_id, collection_id)
);

CREATE TABLE computation_runs (
    id TEXT PRIMARY KEY NOT NULL,
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    logical_path TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    actor_id TEXT,
    state TEXT NOT NULL CHECK(state IN ('awaiting_confirmation','running','completed','rejected','failed','expired')),
    contract_fingerprint TEXT NOT NULL,
    executor_sha256 TEXT NOT NULL,
    attester_sha256 TEXT NOT NULL,
    parameter_schema_json TEXT NOT NULL,
    receipt_sha256 TEXT,
    verdict TEXT,
    requested_at TEXT NOT NULL,
    confirmed_at TEXT,
    completed_at TEXT,
    expires_at TEXT NOT NULL
);
CREATE INDEX computation_runs_expiry ON computation_runs(state, expires_at);

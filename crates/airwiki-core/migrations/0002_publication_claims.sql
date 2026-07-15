CREATE TABLE publication_claims (
    job_id TEXT PRIMARY KEY NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    concept_id TEXT NOT NULL UNIQUE REFERENCES concepts(id) ON DELETE CASCADE,
    collection_id TEXT NOT NULL UNIQUE REFERENCES collections(id) ON DELETE CASCADE,
    source_sha256 TEXT NOT NULL CHECK(length(source_sha256) = 64),
    source_revision INTEGER NOT NULL,
    action TEXT NOT NULL CHECK(action IN ('published', 'replaced')),
    reviewed_at TEXT NOT NULL
);

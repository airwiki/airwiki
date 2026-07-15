CREATE TABLE collections (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    source_folder TEXT NOT NULL UNIQUE,
    wiki_folder TEXT NOT NULL,
    local_only INTEGER NOT NULL DEFAULT 1,
    peer_shareable INTEGER NOT NULL DEFAULT 0,
    allow_external_ai INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE source_documents (
    id TEXT PRIMARY KEY NOT NULL,
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    source_path TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    source_format TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    page_count INTEGER NOT NULL DEFAULT 0,
    character_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    concept_id TEXT,
    last_error TEXT,
    discovered_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    UNIQUE(collection_id, source_path)
);
CREATE INDEX source_documents_collection_hash
    ON source_documents(collection_id, source_sha256);
CREATE INDEX source_documents_status ON source_documents(status);

CREATE TABLE concepts (
    id TEXT PRIMARY KEY NOT NULL,
    source_document_id TEXT NOT NULL UNIQUE REFERENCES source_documents(id) ON DELETE CASCADE,
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    concept_type TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    language TEXT NOT NULL,
    tags_json TEXT NOT NULL,
    entities_json TEXT NOT NULL,
    links_json TEXT NOT NULL,
    summary TEXT NOT NULL,
    classification_confidence REAL NOT NULL,
    classification_explanation TEXT NOT NULL,
    logical_resource_uri TEXT NOT NULL,
    generator_model TEXT NOT NULL,
    status TEXT NOT NULL,
    reviewed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX concepts_collection_status ON concepts(collection_id, status);

CREATE TABLE chunks (
    id TEXT PRIMARY KEY NOT NULL,
    concept_id TEXT NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
    source_document_id TEXT NOT NULL REFERENCES source_documents(id) ON DELETE CASCADE,
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    heading_or_page TEXT NOT NULL,
    text TEXT NOT NULL,
    text_sha256 TEXT NOT NULL,
    embedding BLOB NOT NULL,
    source_revision INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(concept_id, ordinal)
);
CREATE INDEX chunks_collection ON chunks(collection_id);

CREATE VIRTUAL TABLE chunk_fts USING fts5(
    chunk_id UNINDEXED,
    title,
    description,
    tags,
    heading,
    text,
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TABLE jobs (
    id TEXT PRIMARY KEY NOT NULL,
    source_document_id TEXT REFERENCES source_documents(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX jobs_state ON jobs(state, updated_at);

CREATE TABLE peers (
    peer_id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT,
    trusted INTEGER NOT NULL DEFAULT 0,
    blocked INTEGER NOT NULL DEFAULT 0,
    paired_at TEXT,
    last_seen_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE grants (
    peer_id TEXT NOT NULL REFERENCES peers(peer_id) ON DELETE CASCADE,
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    granted_at TEXT NOT NULL,
    PRIMARY KEY(peer_id, collection_id)
);

CREATE TABLE audit_events (
    id TEXT PRIMARY KEY NOT NULL,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT,
    details_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX audit_events_created ON audit_events(created_at);

ALTER TABLE collections ADD COLUMN internet_public INTEGER NOT NULL DEFAULT 0;

CREATE TABLE public_collection_profiles (
    collection_id TEXT PRIMARY KEY NOT NULL
        REFERENCES collections(id) ON DELETE CASCADE,
    description TEXT NOT NULL DEFAULT '',
    languages_json TEXT NOT NULL DEFAULT '[]',
    manifest_sequence INTEGER NOT NULL DEFAULT 0 CHECK(manifest_sequence >= 0),
    enabled_at TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE federation_indexes (
    peer_id TEXT PRIMARY KEY NOT NULL,
    multiaddr TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    source TEXT NOT NULL CHECK(source IN ('bootstrap', 'community')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

ALTER TABLE federation_indexes ADD COLUMN registry_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE federation_indexes ADD COLUMN expires_at TEXT;

CREATE TABLE public_publisher_blocks (
    publisher_id TEXT PRIMARY KEY NOT NULL,
    blocked_at TEXT NOT NULL
);

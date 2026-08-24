ALTER TABLE collections ADD COLUMN memory_scope TEXT
    CHECK(memory_scope IN ('personal','project'));

UPDATE collections SET memory_scope='personal' WHERE origin='ai_memory';

CREATE TABLE project_memory_attachments (
    collection_id TEXT PRIMARY KEY NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL,
    portable_wiki_id TEXT NOT NULL,
    project_root TEXT NOT NULL UNIQUE,
    manifest_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('active','invalid','missing','identity_conflict')),
    last_error_code TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX project_memory_attachment_identity
    ON project_memory_attachments(project_id,portable_wiki_id);

CREATE TABLE project_memory_requests (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL REFERENCES application_capabilities(app_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('initialize','attach')),
    project_root TEXT NOT NULL,
    requested_name TEXT,
    manifest_fingerprint TEXT,
    state TEXT NOT NULL CHECK(state IN ('awaiting_confirmation','approved','rejected','expired')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    CHECK((kind='initialize' AND requested_name IS NOT NULL AND manifest_fingerprint IS NULL) OR
          (kind='attach' AND requested_name IS NULL AND manifest_fingerprint IS NOT NULL))
);
CREATE INDEX project_memory_requests_pending
    ON project_memory_requests(app_id,state,expires_at);
CREATE UNIQUE INDEX project_memory_requests_one_pending_per_root
    ON project_memory_requests(app_id,project_root)
    WHERE state='awaiting_confirmation';

CREATE TRIGGER collections_memory_scope_insert
BEFORE INSERT ON collections
WHEN (NEW.origin='ai_memory' AND NEW.memory_scope IS NULL)
  OR (NEW.origin!='ai_memory' AND NEW.memory_scope IS NOT NULL)
BEGIN
    SELECT RAISE(ABORT, 'collection memory scope is incompatible with its origin');
END;

CREATE TRIGGER collections_memory_scope_update
BEFORE UPDATE OF origin,memory_scope ON collections
WHEN (NEW.origin='ai_memory' AND NEW.memory_scope IS NULL)
  OR (NEW.origin!='ai_memory' AND NEW.memory_scope IS NOT NULL)
  OR (EXISTS(SELECT 1 FROM project_memory_attachments p WHERE p.collection_id=OLD.id)
      AND (NEW.origin!='ai_memory' OR NEW.memory_scope!='project'))
BEGIN
    SELECT RAISE(ABORT, 'collection memory scope is incompatible with its origin');
END;

CREATE TRIGGER project_memory_attachment_insert
BEFORE INSERT ON project_memory_attachments
WHEN NOT EXISTS(
    SELECT 1 FROM collections c
    WHERE c.id=NEW.collection_id AND c.origin='ai_memory' AND c.memory_scope='project'
)
BEGIN
    SELECT RAISE(ABORT, 'project attachment requires a project memory collection');
END;

CREATE TRIGGER project_memory_attachment_update
BEFORE UPDATE OF collection_id ON project_memory_attachments
WHEN NOT EXISTS(
    SELECT 1 FROM collections c
    WHERE c.id=NEW.collection_id AND c.origin='ai_memory' AND c.memory_scope='project'
)
BEGIN
    SELECT RAISE(ABORT, 'project attachment requires a project memory collection');
END;

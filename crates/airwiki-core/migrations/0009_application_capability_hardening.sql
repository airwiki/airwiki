ALTER TABLE application_capabilities ADD COLUMN capability_prefix TEXT NOT NULL DEFAULT '';
ALTER TABLE application_capabilities ADD COLUMN mutations_window_started_at TEXT;
ALTER TABLE application_capabilities ADD COLUMN mutations_in_window INTEGER NOT NULL DEFAULT 0;
ALTER TABLE application_capabilities ADD COLUMN creations_window_started_at TEXT;
ALTER TABLE application_capabilities ADD COLUMN creations_in_window INTEGER NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX application_capability_prefix_unique
    ON application_capabilities(capability_prefix) WHERE capability_prefix!='';

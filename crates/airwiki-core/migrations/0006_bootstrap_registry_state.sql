CREATE TABLE federation_bootstrap_registry_state (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton = 1),
    registry_version INTEGER NOT NULL CHECK(registry_version > 0),
    updated_at TEXT NOT NULL
);

INSERT INTO federation_bootstrap_registry_state(singleton, registry_version, updated_at)
SELECT 1, MAX(registry_version), MAX(updated_at)
FROM federation_indexes
HAVING MAX(registry_version) > 0;

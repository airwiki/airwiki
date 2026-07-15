CREATE TABLE collection_maintenance (
    collection_id TEXT PRIMARY KEY NOT NULL
        REFERENCES collections(id) ON DELETE CASCADE,
    last_started_at TEXT,
    last_finished_at TEXT,
    last_success_at TEXT,
    status TEXT NOT NULL DEFAULT 'never'
        CHECK(status IN ('never', 'success', 'partial', 'failed', 'quarantined')),
    analyzed_count INTEGER NOT NULL DEFAULT 0 CHECK(analyzed_count >= 0),
    unchanged_count INTEGER NOT NULL DEFAULT 0 CHECK(unchanged_count >= 0),
    renamed_count INTEGER NOT NULL DEFAULT 0 CHECK(renamed_count >= 0),
    deleted_count INTEGER NOT NULL DEFAULT 0 CHECK(deleted_count >= 0),
    failed_count INTEGER NOT NULL DEFAULT 0 CHECK(failed_count >= 0),
    issue_code TEXT,
    issue_summary TEXT,
    CHECK(
        (issue_code IS NULL AND issue_summary IS NULL)
        OR (issue_code IS NOT NULL AND issue_summary IS NOT NULL)
    )
);

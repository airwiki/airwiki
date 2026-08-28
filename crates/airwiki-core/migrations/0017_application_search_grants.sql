-- Before per-application search grants, allow_external_ai was the user's
-- collection-wide approval. Preserve that already-approved access for active
-- applications without opening any collection that was previously local-only.
INSERT INTO application_wiki_grants (
    app_id,
    collection_id,
    role,
    granted_at,
    confirmed_at
)
SELECT
    application_capabilities.app_id,
    collections.id,
    'reader',
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
FROM application_capabilities
CROSS JOIN collections
WHERE application_capabilities.revoked_at IS NULL
  AND collections.allow_external_ai = 1
  AND collections.okf_compatibility IN ('declared_v02', 'undeclared_v02_compatible')
  AND (
      collections.origin != 'ai_memory'
      OR collections.memory_scope = 'personal'
      OR EXISTS (
          SELECT 1
          FROM project_memory_attachments
          WHERE project_memory_attachments.collection_id = collections.id
            AND project_memory_attachments.state = 'active'
      )
  )
ON CONFLICT(app_id, collection_id) DO NOTHING;

ALTER TABLE public_collection_profiles ADD COLUMN withdrawal_pending INTEGER NOT NULL DEFAULT 0
    CHECK(withdrawal_pending IN (0,1));

UPDATE federation_indexes
SET enabled = 0
WHERE multiaddr LIKE '/dns%';

UPDATE public_collection_profiles
SET manifest_sequence = manifest_sequence + 1,
    enabled_at = NULL,
    withdrawal_pending = 1
WHERE collection_id IN (
    SELECT id FROM collections
    WHERE internet_public = 0
       OR okf_compatibility NOT IN ('declared_v02','undeclared_v02_compatible')
);

INSERT INTO public_collection_profiles
    (collection_id,description,languages_json,manifest_sequence,enabled_at,updated_at,
     withdrawal_pending)
SELECT id,'','[]',1,NULL,updated_at,1
FROM collections
WHERE internet_public = 1
  AND okf_compatibility NOT IN ('declared_v02','undeclared_v02_compatible')
  AND NOT EXISTS (
      SELECT 1 FROM public_collection_profiles profile WHERE profile.collection_id=collections.id
  );

DELETE FROM grants
WHERE collection_id IN (
    SELECT id FROM collections
    WHERE okf_compatibility NOT IN ('declared_v02','undeclared_v02_compatible')
);

DELETE FROM application_wiki_grants
WHERE collection_id IN (
    SELECT id FROM collections
    WHERE okf_compatibility NOT IN ('declared_v02','undeclared_v02_compatible')
);

UPDATE collections
SET local_only = 1,
    peer_shareable = 0,
    allow_external_ai = 0,
    internet_public = 0,
    indexing_mode = CASE WHEN origin = 'folder' THEN 'manual' ELSE indexing_mode END
WHERE okf_compatibility NOT IN ('declared_v02','undeclared_v02_compatible');

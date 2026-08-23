CREATE VIRTUAL TABLE okf_projection_fts USING fts5(
    collection_id UNINDEXED,
    concept_id UNINDEXED,
    logical_path,
    title,
    description,
    tags,
    text,
    fingerprint UNINDEXED,
    lifecycle_status UNINDEXED,
    tokenize='unicode61 remove_diacritics 2'
);

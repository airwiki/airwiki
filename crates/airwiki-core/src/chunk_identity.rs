use uuid::Uuid;

pub(crate) fn public_chunk_id(source_sha256: &str, ordinal: u32, text_sha256: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("urn:airwiki:chunk:{source_sha256}:{ordinal}:{text_sha256}").as_bytes(),
    )
}

pub(crate) fn stored_chunk_id(
    concept_id: Uuid,
    source_sha256: &str,
    ordinal: u32,
    text_sha256: &str,
) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("urn:airwiki:stored-chunk:{concept_id}:{source_sha256}:{ordinal}:{text_sha256}")
            .as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_identity_is_stable_across_independent_nodes() {
        let source_hash = "a".repeat(64);
        let chunk_hash = "b".repeat(64);

        assert_eq!(
            public_chunk_id(&source_hash, 3, &chunk_hash),
            public_chunk_id(&source_hash, 3, &chunk_hash)
        );
    }

    #[test]
    fn stored_identity_is_scoped_to_its_concept() {
        let source_hash = "a".repeat(64);
        let chunk_hash = "b".repeat(64);

        assert_ne!(
            stored_chunk_id(Uuid::nil(), &source_hash, 3, &chunk_hash),
            stored_chunk_id(Uuid::max(), &source_hash, 3, &chunk_hash)
        );
    }
}

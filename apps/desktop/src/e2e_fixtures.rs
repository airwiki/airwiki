//! Synthetic review material for the isolated debug-only desktop journey.

use std::{ffi::OsStr, path::Path};

use airwiki_core::{
    AppPaths as CoreAppPaths, Database, EMBEDDING_DIMENSIONS, IndexingMode, OkfPublisher,
    StoredChunk, WikiOrigin,
};
use airwiki_types::{CollectionPolicy, ConceptType, EnrichmentDraft};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::paths::AppPaths;

pub(crate) fn seed_review_if_requested(database: &Database, paths: &AppPaths) -> Result<()> {
    if std::env::var_os("AIRWIKI_E2E_REVIEW_FIXTURE").as_deref() != Some(OsStr::new("1")) {
        return Ok(());
    }
    let root = std::env::var_os("AIRWIKI_E2E_DATA_ROOT")
        .context("review fixtures require the isolated E2E data root")?;
    let root = Path::new(&root).canonicalize()?;
    let temporary = std::env::temp_dir().canonicalize()?;
    ensure!(
        root.starts_with(&temporary)
            && root
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with("airwiki-e2e-"))
            && paths.data.canonicalize()? == root.join("data").canonicalize()?,
        "review fixtures require an isolated temporary E2E directory"
    );
    seed_review(database, &paths.data)
}

fn seed_review(database: &Database, data: &Path) -> Result<()> {
    let source_folder = data.join("synthetic-review-source");
    let collections = database.list_collections()?;
    if collections
        .iter()
        .any(|collection| collection.source_folder == source_folder)
    {
        return Ok(());
    }
    ensure!(
        collections.is_empty(),
        "review fixtures require an empty synthetic workspace"
    );
    std::fs::create_dir(&source_folder)?;
    let wiki_folder = CoreAppPaths::at(data).vaults.join("synthetic-review-wiki");
    std::fs::create_dir(&wiki_folder)?;
    let collection = database.create_collection_with_origin(
        "Synthetic review workspace",
        &source_folder,
        &wiki_folder,
        CollectionPolicy::local_only(),
        WikiOrigin::Folder,
        IndexingMode::Manual,
    )?;
    for (file, title, text) in [
        (
            "maintenance.md",
            "Review maintenance",
            "Create a local backup and verify its checksum before beginning.",
        ),
        (
            "recovery.md",
            "Review recovery",
            "If validation fails, restore the backup and check the recovered state.",
        ),
    ] {
        let path = source_folder.join(file);
        std::fs::write(&path, text)?;
        let hash = hex::encode(Sha256::digest(text.as_bytes()));
        let length = u64::try_from(text.len())?;
        let source = database.register_source(collection.id, &path, &hash, "markdown", length)?;
        database.mark_extracted(source.id(), 0, length)?;
        let draft = EnrichmentDraft {
            concept_type: ConceptType::Reference,
            title: title.into(),
            description: "Synthetic review fixture".into(),
            language: "en".into(),
            tags: Vec::new(),
            entities: Vec::new(),
            links: Vec::new(),
            summary: text.into(),
            classification_confidence: 1.0,
            classification_explanation: "Synthetic proposal prepared without an inference runtime"
                .into(),
        };
        let concept = database.save_enrichment(source.id(), draft, "e2e-review", "synthetic")?;
        let source = database
            .source_document(source.id())?
            .context("synthetic source missing")?;
        database.replace_chunks(
            concept.id,
            &[StoredChunk {
                id: Uuid::new_v4(),
                concept_id: concept.id,
                source_document_id: source.id,
                collection_id: collection.id,
                ordinal: 0,
                heading_or_page: "Source evidence".into(),
                text: text.into(),
                text_sha256: hash,
                embedding: vec![0.0; EMBEDDING_DIMENSIONS],
                source_revision: source.revision,
            }],
        )?;
        OkfPublisher::new(&wiki_folder).write_draft(&concept, &source)?;
    }
    Ok(())
}

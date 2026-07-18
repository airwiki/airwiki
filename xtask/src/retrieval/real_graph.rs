//! Real-ranking development replay for the compact OKF concept graph.
//!
//! Unlike the mechanistic fixture in `mini_graph`, this evaluator obtains its
//! candidate order from AirWiki's production BM25/E5/RRF implementation and
//! obtains every edge by inspecting materialized, healthy OKF bundles. It is
//! still a development experiment: it does not change production search.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use airwiki_core::{
    Database, DeterministicEvidenceRelevanceProvider, EMBEDDING_DIMENSIONS, EmbeddingProvider,
    FastEmbedE5Small, HybridSearchEngine, KnowledgeBundleState, KnowledgeLinkDisposition,
    KnowledgePageId, OkfBundleInspector, OkfPublicationMaterializer, PinnedE5Snapshot, StoredChunk,
};
use airwiki_types::{CollectionPolicy, ConceptType, EnrichmentDraft, SearchPurpose, SuggestedLink};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::EvaluationWorkspace;
use super::mini_graph::{
    BASELINE_LIMIT, CONTROL_LIMIT, ExpansionDirection, GraphLinkInput, GraphNode, GraphNodeInput,
    GraphPurpose, LinkDisposition, MiniGraph, NodeState, QueryScope,
};
use crate::{replace_file, workspace_root};

const FIXTURE_PATH: &str = "fixtures/retrieval/mini-graph-real-development-v1.json";
const REPORT_DIRECTORY: &str = "target/evals";
const FIXTURE_SCHEMA_VERSION: u32 = 1;
const REPORT_SCHEMA_VERSION: u32 = 1;
const MIN_DOMAIN_COUNT: usize = 4;
const MIN_CASES_PER_DOMAIN: usize = 3;
const MAX_DISTRACTORS_PER_DOMAIN: usize = 64;
const MAX_DOCUMENTS: usize = 500;
const MAX_RETAINED_PAYLOAD_BYTES: usize = 1024 * 1024;
const MIN_GROUP_GAIN: u32 = 2;
const MIN_IMPROVED_DOMAIN_COUNT: u32 = 3;
const MIN_RESCUE_DOMAIN_COUNT: u32 = 2;
const NODE_ID: &str = "real-graph-development";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayFixture {
    schema_version: u32,
    experiment_id: String,
    domains: Vec<DomainFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DomainFixture {
    id: String,
    collection_name: String,
    language: String,
    distractor_count: usize,
    distractor_title: String,
    distractor_description: String,
    distractor_heading: String,
    distractor_text: String,
    documents: Vec<DocumentFixture>,
    cases: Vec<CaseFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentFixture {
    id: String,
    title: String,
    description: String,
    heading: String,
    text: String,
    links: Vec<DocumentLinkFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentLinkFixture {
    label: String,
    target_document_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFixture {
    id: String,
    question: String,
    required_document_groups: Vec<Vec<String>>,
}

#[derive(Debug)]
struct LoadedFixture {
    fixture: ReplayFixture,
    sha256: String,
}

#[derive(Debug, Clone)]
struct SeedDocument {
    logical_id: String,
    collection_id: Uuid,
    source_id: Uuid,
    concept_id: Uuid,
    draft: EnrichmentDraft,
    heading: String,
    text: String,
    links: Vec<DocumentLinkFixture>,
}

struct ReplayCorpus {
    database: Database,
    engine: HybridSearchEngine,
    graph: MiniGraph,
    sham_graph: MiniGraph,
    logical_by_concept: HashMap<Uuid, String>,
    concept_by_logical: HashMap<String, Uuid>,
    collection_ids: BTreeSet<Uuid>,
    bundle_fingerprints: BTreeMap<Uuid, String>,
    projection_micros: u128,
    _workspace: EvaluationWorkspace,
}

#[derive(Debug, Clone, Serialize)]
struct ArmReport {
    candidate_count: u32,
    found_group_count: u32,
    required_group_count: u32,
    recall: Option<f64>,
    support_candidate_count: u32,
    support_density: Option<f64>,
    candidate_document_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CaseReport {
    id: String,
    domain: String,
    c10: ArmReport,
    c32: ArmReport,
    g1_out: ArmReport,
    g1_bidir: ArmReport,
    g1_sham: ArmReport,
    outgoing_unique_group_count_over_c32: u32,
    bidir_unique_group_count_over_outgoing: u32,
    sham_unique_group_count_over_c32: u32,
    lost_group_count_against_c32: u32,
}

#[derive(Debug, Default, Serialize)]
struct AggregateArm {
    candidate_count: u32,
    found_group_count: u32,
    required_group_count: u32,
    support_candidate_count: u32,
    recall: Option<f64>,
    support_density: Option<f64>,
}

#[derive(Debug, Serialize)]
struct ReplayReport {
    schema_version: u32,
    experiment_id: String,
    fixture_sha256: String,
    embedding_profile: String,
    target_os: String,
    target_arch: String,
    domain_count: u32,
    case_count: u32,
    concept_count: u32,
    edge_count: u32,
    graph_fingerprint: String,
    sham_graph_fingerprint: String,
    retained_payload_bytes: usize,
    projection_micros: u128,
    ranking_and_expansion_micros: u128,
    c10: AggregateArm,
    c32: AggregateArm,
    g1_out: AggregateArm,
    g1_bidir: AggregateArm,
    g1_sham: AggregateArm,
    outgoing_unique_group_count_over_c32: u32,
    bidir_unique_group_count_over_outgoing: u32,
    sham_unique_group_count_over_c32: u32,
    improved_domain_count_over_controls: u32,
    outgoing_rescue_domain_count: u32,
    backlink_rescue_domain_count: u32,
    regressed_case_count_against_c32: u32,
    healthy_fingerprint_gate_passed: bool,
    development_gate_passed: bool,
    production_promotion_ready: bool,
    rejection_reasons: Vec<String>,
    cases: Vec<CaseReport>,
}

#[derive(Debug, Clone, Copy)]
struct DevelopmentGateMetrics {
    healthy_fingerprint: bool,
    c32_found_groups: u32,
    c32_support_density: Option<f64>,
    bidir_found_groups: u32,
    bidir_support_density: Option<f64>,
    sham_found_groups: u32,
    outgoing_unique_groups: u32,
    backlink_unique_groups: u32,
    sham_unique_groups: u32,
    improved_domains: u32,
    outgoing_rescue_domains: u32,
    backlink_rescue_domains: u32,
    regressed_cases: u32,
    retained_payload_bytes: usize,
}

pub(crate) async fn evaluate_real_mini_graph(embedding_snapshot: &Path) -> Result<()> {
    let loaded = load_fixture()?;
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 4);
    let embeddings: Arc<dyn EmbeddingProvider> = Arc::new(FastEmbedE5Small::from_snapshot(
        &PinnedE5Snapshot::open(embedding_snapshot)?,
        threads,
    )?);
    let embedding_profile = embeddings.model_id().to_owned();
    let corpus = build_corpus(&loaded.fixture, Arc::clone(&embeddings)).await?;
    let report = run_replay(&loaded, &corpus, embedding_profile).await?;
    let destination = write_report(&report)?;
    ensure!(
        report.development_gate_passed,
        "real-ranking mini-graph did not pass its development gate; report written to {}",
        destination.display()
    );
    println!(
        "real-ranking mini-graph passed its development gate; report written to {} (production promotion remains disabled)",
        destination.display()
    );
    Ok(())
}

fn load_fixture() -> Result<LoadedFixture> {
    let path = workspace_root().join(FIXTURE_PATH);
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let fixture = serde_json::from_slice::<ReplayFixture>(&bytes)
        .with_context(|| format!("parsing {}", path.display()))?;
    validate_fixture(&fixture)?;
    Ok(LoadedFixture {
        fixture,
        sha256: hex::encode(Sha256::digest(&bytes)),
    })
}

fn validate_fixture(fixture: &ReplayFixture) -> Result<()> {
    ensure!(
        fixture.schema_version == FIXTURE_SCHEMA_VERSION,
        "unsupported real-ranking mini-graph fixture schema"
    );
    validate_identifier(&fixture.experiment_id)?;
    ensure!(
        fixture.domains.len() >= MIN_DOMAIN_COUNT,
        "real-ranking mini-graph fixture needs at least {MIN_DOMAIN_COUNT} domains"
    );
    let mut domain_ids = BTreeSet::new();
    let mut all_document_ids = BTreeSet::new();
    let mut all_case_ids = BTreeSet::new();
    let mut total_documents = 0_usize;
    for domain in &fixture.domains {
        validate_identifier(&domain.id)?;
        ensure!(domain_ids.insert(domain.id.as_str()), "duplicate domain id");
        ensure!(
            !domain.collection_name.trim().is_empty(),
            "empty collection name"
        );
        ensure!(!domain.language.trim().is_empty(), "empty language");
        ensure!(
            domain.distractor_count <= MAX_DISTRACTORS_PER_DOMAIN,
            "domain exceeds distractor limit"
        );
        ensure!(
            domain.cases.len() >= MIN_CASES_PER_DOMAIN,
            "domain needs at least {MIN_CASES_PER_DOMAIN} cases"
        );
        total_documents = total_documents
            .saturating_add(domain.documents.len())
            .saturating_add(domain.distractor_count);
        let domain_documents = domain
            .documents
            .iter()
            .map(|document| document.id.as_str())
            .collect::<BTreeSet<_>>();
        ensure!(
            domain_documents.len() == domain.documents.len(),
            "duplicate document id within domain"
        );
        for document in &domain.documents {
            validate_identifier(&document.id)?;
            ensure!(
                all_document_ids.insert(document.id.as_str()),
                "duplicate document id across domains"
            );
            ensure!(!document.title.trim().is_empty(), "empty document title");
            ensure!(!document.text.trim().is_empty(), "empty document text");
            for link in &document.links {
                ensure!(!link.label.trim().is_empty(), "empty link label");
                ensure!(
                    domain_documents.contains(link.target_document_id.as_str()),
                    "link target must exist in the same domain"
                );
                ensure!(
                    link.target_document_id != document.id,
                    "self links are not valid replay evidence"
                );
            }
        }
        for case in &domain.cases {
            validate_identifier(&case.id)?;
            ensure!(all_case_ids.insert(case.id.as_str()), "duplicate case id");
            ensure!(!case.question.trim().is_empty(), "empty replay question");
            ensure!(
                !case.required_document_groups.is_empty(),
                "replay case has no required group"
            );
            for group in &case.required_document_groups {
                ensure!(!group.is_empty(), "replay case has an empty group");
                ensure!(
                    group
                        .iter()
                        .all(|id| domain_documents.contains(id.as_str())),
                    "replay gold must stay inside its domain"
                );
            }
        }
    }
    ensure!(
        total_documents > CONTROL_LIMIT,
        "real-ranking replay must contain more than {CONTROL_LIMIT} candidates"
    );
    ensure!(
        total_documents <= MAX_DOCUMENTS,
        "real-ranking replay exceeds graph node budget"
    );
    Ok(())
}

fn validate_identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 96
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            }),
        "invalid real-ranking replay identifier"
    );
    Ok(())
}

async fn build_corpus(
    fixture: &ReplayFixture,
    embeddings: Arc<dyn EmbeddingProvider>,
) -> Result<ReplayCorpus> {
    let workspace = EvaluationWorkspace::create()?;
    let database = Database::in_memory()?;
    let mut collection_ids = BTreeSet::new();
    let mut documents = Vec::new();

    for domain in &fixture.domains {
        let root = workspace.path().join(&domain.id);
        let source_folder = root.join("sources");
        let wiki_folder = root.join("wiki");
        std::fs::create_dir_all(&source_folder).context("creating replay source directory")?;
        std::fs::create_dir_all(&wiki_folder).context("creating replay wiki directory")?;
        let collection = database.create_collection(
            &domain.collection_name,
            &source_folder,
            &wiki_folder,
            CollectionPolicy::local_only(),
        )?;
        collection_ids.insert(collection.id);
        for document in &domain.documents {
            documents.push(seed_document_metadata(
                &database,
                collection.id,
                &source_folder,
                &domain.language,
                document.clone(),
            )?);
        }
        for index in 0..domain.distractor_count {
            let ordinal = index.saturating_add(1);
            let document = DocumentFixture {
                id: format!("{}_distractor_{ordinal:02}", domain.id),
                title: format!("{} {ordinal:02}", domain.distractor_title),
                description: domain.distractor_description.clone(),
                heading: domain.distractor_heading.clone(),
                text: domain.distractor_text.clone(),
                links: Vec::new(),
            };
            documents.push(seed_document_metadata(
                &database,
                collection.id,
                &source_folder,
                &domain.language,
                document,
            )?);
        }
    }

    embed_and_store_chunks(&database, &documents, Arc::clone(&embeddings)).await?;
    let concepts = documents
        .iter()
        .map(|document| (document.logical_id.clone(), document.concept_id))
        .collect::<HashMap<_, _>>();
    let resources = documents
        .iter()
        .map(|document| {
            database
                .concept(document.concept_id)?
                .map(|concept| (document.logical_id.clone(), concept.logical_resource_uri))
                .context("seeded replay concept disappeared")
        })
        .collect::<Result<HashMap<_, _>>>()?;
    let materializer = OkfPublicationMaterializer::new(database.clone());
    for document in &mut documents {
        let mut draft = document.draft.clone();
        draft.links = document
            .links
            .iter()
            .map(|link| {
                Ok(SuggestedLink {
                    label: link.label.clone(),
                    target: resources
                        .get(&link.target_document_id)
                        .cloned()
                        .context("replay link target has no materialized resource")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let concept = database.save_enrichment(
            document.source_id,
            draft.clone(),
            NODE_ID,
            "real-ranking-replay",
        )?;
        ensure!(
            concept.id == document.concept_id,
            "re-enrichment changed concept identity"
        );
        let evidence = database
            .review_evidence_page(concept.id, 1, None, None, 1)?
            .context("replay review evidence is missing")?;
        materializer.approve(concept.id, draft.clone(), &evidence.review_version)?;
        document.draft = draft;
    }

    let projection_started = Instant::now();
    let inspector = OkfBundleInspector::new(database.clone());
    let mut bundle_fingerprints = BTreeMap::new();
    let mut node_inputs = Vec::new();
    let mut link_inputs = Vec::new();
    for collection_id in &collection_ids {
        let bundle = inspector.inspect_bundle(*collection_id)?;
        ensure!(
            bundle.state == KnowledgeBundleState::Ready && bundle.health.is_healthy(),
            "real-ranking replay requires a healthy Ready bundle"
        );
        bundle_fingerprints.insert(*collection_id, bundle.fingerprint.clone());
        node_inputs.extend(bundle.concepts.iter().map(|concept| GraphNodeInput {
            node: GraphNode {
                concept_id: concept.id,
                collection_id: *collection_id,
            },
            state: NodeState::Current,
        }));
        for link in &bundle.links {
            if let (
                KnowledgePageId::Concept(source),
                KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(target)),
            ) = (link.source, &link.disposition)
            {
                link_inputs.push(GraphLinkInput {
                    source,
                    target: *target,
                    disposition: LinkDisposition::ReviewedInternal,
                });
            }
        }
    }
    let graph = MiniGraph::build(&node_inputs, &link_inputs)?;
    let sham_links = sham_links(&node_inputs, &link_inputs)?;
    let sham_graph = MiniGraph::build(&node_inputs, &sham_links)?;
    ensure!(
        graph.node_count() == sham_graph.node_count()
            && graph.edge_count() == sham_graph.edge_count()
            && directed_degrees(&link_inputs) == directed_degrees(&sham_links),
        "sham graph must preserve graph size and per-node directed degrees"
    );
    let projection_micros = projection_started.elapsed().as_micros();
    let logical_by_concept = documents
        .iter()
        .map(|document| (document.concept_id, document.logical_id.clone()))
        .collect::<HashMap<_, _>>();
    let concept_by_logical = concepts.into_iter().collect::<HashMap<_, _>>();
    let engine = HybridSearchEngine::new(
        database.clone(),
        embeddings,
        Arc::new(DeterministicEvidenceRelevanceProvider),
        NODE_ID,
    );
    Ok(ReplayCorpus {
        database,
        engine,
        graph,
        sham_graph,
        logical_by_concept,
        concept_by_logical,
        collection_ids,
        bundle_fingerprints,
        projection_micros,
        _workspace: workspace,
    })
}

fn seed_document_metadata(
    database: &Database,
    collection_id: Uuid,
    source_folder: &Path,
    language: &str,
    document: DocumentFixture,
) -> Result<SeedDocument> {
    let source_contents = format!("# {}\n\n{}\n", document.title, document.text);
    let source_sha256 = hex::encode(Sha256::digest(source_contents.as_bytes()));
    let source_path = source_folder.join(format!("{}.md", document.id));
    std::fs::write(&source_path, source_contents.as_bytes())
        .context("writing real-ranking replay source")?;
    let source = database.register_source(
        collection_id,
        &source_path,
        &source_sha256,
        "markdown",
        u64::try_from(source_contents.len()).context("replay source is too large")?,
    )?;
    database.mark_extracted(
        source.id(),
        0,
        u64::try_from(source_contents.chars().count())
            .context("replay source character count overflow")?,
    )?;
    let draft = EnrichmentDraft {
        concept_type: ConceptType::Document,
        title: document.title,
        description: document.description,
        language: language.to_owned(),
        tags: vec!["synthetic-evaluation".to_owned()],
        entities: Vec::new(),
        links: Vec::new(),
        summary: document.text.clone(),
        classification_confidence: 1.0,
        classification_explanation: "synthetic real-ranking graph replay".to_owned(),
    };
    let concept =
        database.save_enrichment(source.id(), draft.clone(), NODE_ID, "real-ranking-replay")?;
    Ok(SeedDocument {
        logical_id: document.id,
        collection_id,
        source_id: source.id(),
        concept_id: concept.id,
        draft,
        heading: document.heading,
        text: document.text,
        links: document.links,
    })
}

async fn embed_and_store_chunks(
    database: &Database,
    documents: &[SeedDocument],
    embeddings: Arc<dyn EmbeddingProvider>,
) -> Result<()> {
    for batch in documents.chunks(32) {
        let inputs = batch
            .iter()
            .map(|document| format!("passage: {}", document.text))
            .collect::<Vec<_>>();
        let vectors = embeddings.embed(&inputs).await?;
        ensure!(
            vectors.len() == batch.len(),
            "embedding batch size mismatch"
        );
        for (document, embedding) in batch.iter().zip(vectors) {
            ensure!(
                embedding.len() == EMBEDDING_DIMENSIONS,
                "embedding dimension mismatch"
            );
            let text_sha256 = hex::encode(Sha256::digest(document.text.as_bytes()));
            let chunk = StoredChunk {
                id: Uuid::new_v5(
                    &Uuid::NAMESPACE_URL,
                    format!("airwiki-real-graph:{}", document.logical_id).as_bytes(),
                ),
                concept_id: document.concept_id,
                source_document_id: document.source_id,
                collection_id: document.collection_id,
                ordinal: 0,
                heading_or_page: document.heading.clone(),
                text: document.text.clone(),
                text_sha256,
                embedding,
                source_revision: 1,
            };
            database.replace_chunks(document.concept_id, &[chunk])?;
        }
    }
    Ok(())
}

fn sham_links(nodes: &[GraphNodeInput], links: &[GraphLinkInput]) -> Result<Vec<GraphLinkInput>> {
    ensure_unique_directed_links(links, "real graph")?;
    let collections = nodes
        .iter()
        .map(|input| (input.node.concept_id, input.node.collection_id))
        .collect::<HashMap<_, _>>();
    let mut grouped = BTreeMap::<Uuid, Vec<GraphLinkInput>>::new();
    for link in links {
        let collection = collections
            .get(&link.source)
            .copied()
            .context("sham source is missing from graph")?;
        ensure!(
            collections.get(&link.target) == Some(&collection),
            "sham input crosses collection boundary"
        );
        grouped.entry(collection).or_default().push(*link);
    }
    let mut rewired = Vec::with_capacity(links.len());
    for (_, mut collection_links) in grouped {
        collection_links.sort_unstable_by_key(|link| (link.source, link.target));
        ensure!(
            collection_links.len() >= 2,
            "sham control requires at least two links per collection"
        );
        let targets = collection_links
            .iter()
            .map(|link| link.target)
            .collect::<Vec<_>>();
        for (index, link) in collection_links.into_iter().enumerate() {
            let target = targets[(index + 1) % targets.len()];
            ensure!(link.source != target, "sham rotation produced a self link");
            rewired.push(GraphLinkInput { target, ..link });
        }
    }
    ensure_unique_directed_links(&rewired, "sham graph")?;
    ensure!(
        directed_degrees(links) == directed_degrees(&rewired),
        "sham rewiring changed per-node directed degrees"
    );
    Ok(rewired)
}

fn ensure_unique_directed_links(links: &[GraphLinkInput], label: &str) -> Result<()> {
    let unique = links
        .iter()
        .map(|link| (link.source, link.target))
        .collect::<BTreeSet<_>>();
    ensure!(
        unique.len() == links.len(),
        "{label} contains duplicate directed links"
    );
    Ok(())
}

fn directed_degrees(links: &[GraphLinkInput]) -> (BTreeMap<Uuid, usize>, BTreeMap<Uuid, usize>) {
    let mut outgoing = BTreeMap::new();
    let mut incoming = BTreeMap::new();
    for link in links {
        *outgoing.entry(link.source).or_insert(0) += 1;
        *incoming.entry(link.target).or_insert(0) += 1;
    }
    (outgoing, incoming)
}

async fn run_replay(
    loaded: &LoadedFixture,
    corpus: &ReplayCorpus,
    embedding_profile: String,
) -> Result<ReplayReport> {
    let started = Instant::now();
    let scope = QueryScope {
        purpose: GraphPurpose::LocalAssistant,
        authorized_collections: corpus.collection_ids.clone(),
        external_ai_collections: BTreeSet::new(),
    };
    let mut reports = Vec::new();
    for domain in &loaded.fixture.domains {
        for case in &domain.cases {
            let ranked = corpus
                .engine
                .rank_local_for_evaluation(
                    &case.question,
                    SearchPurpose::LocalAssistant,
                    CONTROL_LIMIT,
                )
                .await?;
            ensure!(
                ranked
                    .iter()
                    .all(|candidate| candidate.source_revision == 1),
                "replay ranking returned a stale revision"
            );
            let ranked_concepts = ranked
                .iter()
                .map(|candidate| candidate.concept_id)
                .collect::<Vec<_>>();
            let c10_nodes =
                corpus
                    .graph
                    .visible_candidates(&ranked_concepts, &scope, BASELINE_LIMIT);
            let c32_nodes =
                corpus
                    .graph
                    .visible_candidates(&ranked_concepts, &scope, CONTROL_LIMIT);
            let g1_out_nodes = corpus
                .graph
                .expand_one_hop_with_backfill(
                    &c10_nodes,
                    &ranked_concepts,
                    &scope,
                    ExpansionDirection::Outgoing,
                )
                .candidates;
            let g1_bidir_nodes = corpus
                .graph
                .expand_one_hop_with_backfill(
                    &c10_nodes,
                    &ranked_concepts,
                    &scope,
                    ExpansionDirection::Bidirectional,
                )
                .candidates;
            let sham_seeds =
                corpus
                    .sham_graph
                    .visible_candidates(&ranked_concepts, &scope, BASELINE_LIMIT);
            let g1_sham_nodes = corpus
                .sham_graph
                .expand_one_hop_with_backfill(
                    &sham_seeds,
                    &ranked_concepts,
                    &scope,
                    ExpansionDirection::Bidirectional,
                )
                .candidates;
            let c10 = score_arm(
                node_documents(&corpus.graph, &c10_nodes, &corpus.logical_by_concept)?,
                &case.required_document_groups,
            );
            let c32 = score_arm(
                node_documents(&corpus.graph, &c32_nodes, &corpus.logical_by_concept)?,
                &case.required_document_groups,
            );
            let g1_out = score_arm(
                node_documents(&corpus.graph, &g1_out_nodes, &corpus.logical_by_concept)?,
                &case.required_document_groups,
            );
            let g1_bidir = score_arm(
                node_documents(&corpus.graph, &g1_bidir_nodes, &corpus.logical_by_concept)?,
                &case.required_document_groups,
            );
            let g1_sham = score_arm(
                node_documents(
                    &corpus.sham_graph,
                    &g1_sham_nodes,
                    &corpus.logical_by_concept,
                )?,
                &case.required_document_groups,
            );
            reports.push(CaseReport {
                id: case.id.clone(),
                domain: domain.id.clone(),
                outgoing_unique_group_count_over_c32: unique_groups(
                    &g1_out,
                    &c32,
                    &case.required_document_groups,
                ),
                bidir_unique_group_count_over_outgoing: unique_groups(
                    &g1_bidir,
                    &g1_out,
                    &case.required_document_groups,
                ),
                sham_unique_group_count_over_c32: unique_groups(
                    &g1_sham,
                    &c32,
                    &case.required_document_groups,
                ),
                lost_group_count_against_c32: lost_groups(
                    &g1_bidir,
                    &c32,
                    &case.required_document_groups,
                ),
                c10,
                c32,
                g1_out,
                g1_bidir,
                g1_sham,
            });
        }
    }
    let ranking_and_expansion_micros = started.elapsed().as_micros();
    let inspector = OkfBundleInspector::new(corpus.database.clone());
    let healthy_fingerprint_gate_passed =
        corpus
            .bundle_fingerprints
            .iter()
            .all(|(collection_id, expected)| {
                inspector
                    .inspect_bundle(*collection_id)
                    .is_ok_and(|bundle| {
                        bundle.state == KnowledgeBundleState::Ready
                            && bundle.health.is_healthy()
                            && bundle.fingerprint == *expected
                    })
            });
    ensure!(
        corpus.concept_by_logical.values().all(|concept_id| corpus
            .database
            .chunks_for_concept(*concept_id)
            .is_ok_and(|chunks| { chunks.len() == 1 && chunks[0].source_revision == 1 })),
        "real-ranking replay requires one current chunk per concept"
    );

    let c10 = aggregate(reports.iter().map(|report| &report.c10));
    let c32 = aggregate(reports.iter().map(|report| &report.c32));
    let g1_out = aggregate(reports.iter().map(|report| &report.g1_out));
    let g1_bidir = aggregate(reports.iter().map(|report| &report.g1_bidir));
    let g1_sham = aggregate(reports.iter().map(|report| &report.g1_sham));
    let outgoing_unique_group_count_over_c32 = reports
        .iter()
        .map(|report| report.outgoing_unique_group_count_over_c32)
        .sum();
    let bidir_unique_group_count_over_outgoing = reports
        .iter()
        .map(|report| report.bidir_unique_group_count_over_outgoing)
        .sum();
    let sham_unique_group_count_over_c32 = reports
        .iter()
        .map(|report| report.sham_unique_group_count_over_c32)
        .sum();
    let improved_domain_count_over_controls = loaded
        .fixture
        .domains
        .iter()
        .filter(|domain| {
            let domain_reports = reports.iter().filter(|report| report.domain == domain.id);
            let (c32_found, sham_found, bidir_found) =
                domain_reports.fold((0_u32, 0_u32, 0_u32), |counts, report| {
                    (
                        counts.0.saturating_add(report.c32.found_group_count),
                        counts.1.saturating_add(report.g1_sham.found_group_count),
                        counts.2.saturating_add(report.g1_bidir.found_group_count),
                    )
                });
            bidir_found > c32_found && bidir_found > sham_found
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let outgoing_rescue_domain_count = loaded
        .fixture
        .domains
        .iter()
        .filter(|domain| {
            reports.iter().any(|report| {
                report.domain == domain.id && report.outgoing_unique_group_count_over_c32 > 0
            })
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let backlink_rescue_domain_count = loaded
        .fixture
        .domains
        .iter()
        .filter(|domain| {
            reports.iter().any(|report| {
                report.domain == domain.id && report.bidir_unique_group_count_over_outgoing > 0
            })
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let regressed_case_count_against_c32 = reports
        .iter()
        .filter(|report| report.lost_group_count_against_c32 > 0)
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let retained_payload_bytes = corpus
        .graph
        .retained_payload_bytes()
        .saturating_add(corpus.sham_graph.retained_payload_bytes());
    let rejection_reasons = development_rejection_reasons(DevelopmentGateMetrics {
        healthy_fingerprint: healthy_fingerprint_gate_passed,
        c32_found_groups: c32.found_group_count,
        c32_support_density: c32.support_density,
        bidir_found_groups: g1_bidir.found_group_count,
        bidir_support_density: g1_bidir.support_density,
        sham_found_groups: g1_sham.found_group_count,
        outgoing_unique_groups: outgoing_unique_group_count_over_c32,
        backlink_unique_groups: bidir_unique_group_count_over_outgoing,
        sham_unique_groups: sham_unique_group_count_over_c32,
        improved_domains: improved_domain_count_over_controls,
        outgoing_rescue_domains: outgoing_rescue_domain_count,
        backlink_rescue_domains: backlink_rescue_domain_count,
        regressed_cases: regressed_case_count_against_c32,
        retained_payload_bytes,
    });
    let development_gate_passed = rejection_reasons.is_empty();
    Ok(ReplayReport {
        schema_version: REPORT_SCHEMA_VERSION,
        experiment_id: loaded.fixture.experiment_id.clone(),
        fixture_sha256: loaded.sha256.clone(),
        embedding_profile,
        target_os: std::env::consts::OS.to_owned(),
        target_arch: std::env::consts::ARCH.to_owned(),
        domain_count: u32::try_from(loaded.fixture.domains.len()).unwrap_or(u32::MAX),
        case_count: u32::try_from(reports.len()).unwrap_or(u32::MAX),
        concept_count: corpus.graph.node_count(),
        edge_count: corpus.graph.edge_count(),
        graph_fingerprint: corpus.graph.fingerprint().to_owned(),
        sham_graph_fingerprint: corpus.sham_graph.fingerprint().to_owned(),
        retained_payload_bytes,
        projection_micros: corpus.projection_micros,
        ranking_and_expansion_micros,
        c10,
        c32,
        g1_out,
        g1_bidir,
        g1_sham,
        outgoing_unique_group_count_over_c32,
        bidir_unique_group_count_over_outgoing,
        sham_unique_group_count_over_c32,
        improved_domain_count_over_controls,
        outgoing_rescue_domain_count,
        backlink_rescue_domain_count,
        regressed_case_count_against_c32,
        healthy_fingerprint_gate_passed,
        development_gate_passed,
        production_promotion_ready: false,
        rejection_reasons,
        cases: reports,
    })
}

fn development_rejection_reasons(metrics: DevelopmentGateMetrics) -> Vec<String> {
    let mut reasons = Vec::new();
    if !metrics.healthy_fingerprint {
        reasons.push("bundle health or fingerprint changed during replay".to_owned());
    }
    if metrics.bidir_found_groups < metrics.c32_found_groups.saturating_add(MIN_GROUP_GAIN) {
        reasons.push("bidirectional graph gained fewer than two groups over C32".to_owned());
    }
    if metrics.bidir_found_groups < metrics.sham_found_groups.saturating_add(MIN_GROUP_GAIN) {
        reasons
            .push("real graph gained fewer than two groups over degree-preserving sham".to_owned());
    }
    if metrics.improved_domains < MIN_IMPROVED_DOMAIN_COUNT {
        reasons.push("graph improvement was concentrated in too few domains".to_owned());
    }
    if metrics.outgoing_unique_groups == 0 {
        reasons.push("outgoing links produced no unique rescue over C32".to_owned());
    }
    if metrics.backlink_unique_groups == 0 {
        reasons.push("backlinks produced no unique rescue over outgoing links".to_owned());
    }
    if metrics.outgoing_rescue_domains < MIN_RESCUE_DOMAIN_COUNT {
        reasons.push("outgoing-link rescues covered fewer than two domains".to_owned());
    }
    if metrics.backlink_rescue_domains < MIN_RESCUE_DOMAIN_COUNT {
        reasons.push("backlink rescues covered fewer than two domains".to_owned());
    }
    if metrics.regressed_cases > 0 {
        reasons.push("graph expansion regressed at least one C32 case".to_owned());
    }
    if !matches!(
        (metrics.bidir_support_density, metrics.c32_support_density),
        (Some(bidir), Some(c32)) if bidir >= c32
    ) {
        reasons.push("graph support density regressed against C32".to_owned());
    }
    if metrics.retained_payload_bytes > MAX_RETAINED_PAYLOAD_BYTES {
        reasons.push("graph projections exceeded the one MiB payload budget".to_owned());
    }
    if metrics.sham_unique_groups > 0 {
        reasons.push("sham graph produced a unique gold rescue".to_owned());
    }
    reasons
}

fn node_documents(
    graph: &MiniGraph,
    nodes: &[super::mini_graph::NodeId],
    logical_by_concept: &HashMap<Uuid, String>,
) -> Result<Vec<String>> {
    nodes
        .iter()
        .map(|node_id| {
            let node = graph.node(*node_id).context("graph node disappeared")?;
            logical_by_concept
                .get(&node.concept_id)
                .cloned()
                .context("graph concept has no replay identity")
        })
        .collect()
}

fn score_arm(candidates: Vec<String>, required_groups: &[Vec<String>]) -> ArmReport {
    let candidate_set = candidates
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let support_ids = required_groups
        .iter()
        .flatten()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let found_group_count = required_groups
        .iter()
        .filter(|group| group.iter().any(|id| candidate_set.contains(id.as_str())))
        .count();
    let support_candidate_count = candidate_set.intersection(&support_ids).count();
    let candidate_count = candidates.len();
    ArmReport {
        candidate_count: u32::try_from(candidate_count).unwrap_or(u32::MAX),
        found_group_count: u32::try_from(found_group_count).unwrap_or(u32::MAX),
        required_group_count: u32::try_from(required_groups.len()).unwrap_or(u32::MAX),
        recall: ratio(found_group_count, required_groups.len()),
        support_candidate_count: u32::try_from(support_candidate_count).unwrap_or(u32::MAX),
        support_density: ratio(support_candidate_count, candidate_count),
        candidate_document_ids: candidates,
    }
}

fn unique_groups(candidate: &ArmReport, control: &ArmReport, groups: &[Vec<String>]) -> u32 {
    let candidate_ids = candidate
        .candidate_document_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let control_ids = control
        .candidate_document_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let count = groups
        .iter()
        .filter(|group| {
            group.iter().any(|id| candidate_ids.contains(id.as_str()))
                && !group.iter().any(|id| control_ids.contains(id.as_str()))
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn lost_groups(candidate: &ArmReport, control: &ArmReport, groups: &[Vec<String>]) -> u32 {
    unique_groups(control, candidate, groups)
}

fn aggregate<'a>(arms: impl Iterator<Item = &'a ArmReport>) -> AggregateArm {
    let mut aggregate = AggregateArm::default();
    for arm in arms {
        aggregate.candidate_count = aggregate
            .candidate_count
            .saturating_add(arm.candidate_count);
        aggregate.found_group_count = aggregate
            .found_group_count
            .saturating_add(arm.found_group_count);
        aggregate.required_group_count = aggregate
            .required_group_count
            .saturating_add(arm.required_group_count);
        aggregate.support_candidate_count = aggregate
            .support_candidate_count
            .saturating_add(arm.support_candidate_count);
    }
    aggregate.recall = ratio(
        usize::try_from(aggregate.found_group_count).unwrap_or(usize::MAX),
        usize::try_from(aggregate.required_group_count).unwrap_or(usize::MAX),
    );
    aggregate.support_density = ratio(
        usize::try_from(aggregate.support_candidate_count).unwrap_or(usize::MAX),
        usize::try_from(aggregate.candidate_count).unwrap_or(usize::MAX),
    );
    aggregate
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

fn write_report(report: &ReplayReport) -> Result<std::path::PathBuf> {
    let directory = workspace_root().join(REPORT_DIRECTORY);
    std::fs::create_dir_all(&directory).context("creating replay report directory")?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates Unix epoch")?
        .as_secs();
    let destination = directory.join(format!(
        "retrieval-mini-graph-real-development-{}-{epoch}.json",
        &report.fixture_sha256[..12]
    ));
    let temporary = destination.with_extension("json.tmp");
    let mut contents = serde_json::to_string_pretty(report).context("serializing replay report")?;
    contents.push('\n');
    std::fs::write(&temporary, contents)
        .with_context(|| format!("writing {}", temporary.display()))?;
    replace_file(&temporary, &destination)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioned_real_ranking_fixture_is_valid() {
        let loaded = load_fixture().unwrap();
        assert_eq!(loaded.fixture.domains.len(), 4);
        assert_eq!(
            loaded
                .fixture
                .domains
                .iter()
                .map(|domain| domain.cases.len())
                .sum::<usize>(),
            12
        );
    }

    #[test]
    fn sham_rotation_changes_endpoints_without_changing_directed_degrees() {
        let collection = Uuid::new_v4();
        let ids = (0..4).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        let nodes = ids
            .iter()
            .map(|concept_id| GraphNodeInput {
                node: GraphNode {
                    concept_id: *concept_id,
                    collection_id: collection,
                },
                state: NodeState::Current,
            })
            .collect::<Vec<_>>();
        let links = vec![
            GraphLinkInput {
                source: ids[0],
                target: ids[1],
                disposition: LinkDisposition::ReviewedInternal,
            },
            GraphLinkInput {
                source: ids[2],
                target: ids[3],
                disposition: LinkDisposition::ReviewedInternal,
            },
        ];

        let sham = sham_links(&nodes, &links).unwrap();

        assert_eq!(sham.len(), links.len());
        let original_by_source = links
            .iter()
            .map(|link| (link.source, link.target))
            .collect::<HashMap<_, _>>();
        assert!(sham.iter().all(|link| {
            original_by_source
                .get(&link.source)
                .is_some_and(|target| *target != link.target)
        }));
        assert_eq!(
            sham.iter().map(|link| link.source).collect::<BTreeSet<_>>(),
            links
                .iter()
                .map(|link| link.source)
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(
            sham.iter().map(|link| link.target).collect::<BTreeSet<_>>(),
            links
                .iter()
                .map(|link| link.target)
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(directed_degrees(&sham), directed_degrees(&links));
    }

    #[test]
    fn sham_rotation_rejects_duplicate_directed_links() {
        let collection = Uuid::new_v4();
        let ids = (0..3).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        let nodes = ids
            .iter()
            .map(|concept_id| GraphNodeInput {
                node: GraphNode {
                    concept_id: *concept_id,
                    collection_id: collection,
                },
                state: NodeState::Current,
            })
            .collect::<Vec<_>>();
        let link = GraphLinkInput {
            source: ids[0],
            target: ids[1],
            disposition: LinkDisposition::ReviewedInternal,
        };

        let error = sham_links(&nodes, &[link, link]).unwrap_err();

        assert!(error.to_string().contains("duplicate directed links"));
    }

    #[test]
    fn development_gate_accepts_the_minimum_distributed_gain() {
        let metrics = DevelopmentGateMetrics {
            healthy_fingerprint: true,
            c32_found_groups: 10,
            c32_support_density: Some(0.1),
            bidir_found_groups: 12,
            bidir_support_density: Some(0.1),
            sham_found_groups: 10,
            outgoing_unique_groups: 1,
            backlink_unique_groups: 1,
            sham_unique_groups: 0,
            improved_domains: 3,
            outgoing_rescue_domains: 2,
            backlink_rescue_domains: 2,
            regressed_cases: 0,
            retained_payload_bytes: MAX_RETAINED_PAYLOAD_BYTES,
        };

        assert!(development_rejection_reasons(metrics).is_empty());
    }

    #[test]
    fn development_gate_rejects_concentrated_gain_and_case_regression() {
        let metrics = DevelopmentGateMetrics {
            healthy_fingerprint: true,
            c32_found_groups: 10,
            c32_support_density: Some(0.1),
            bidir_found_groups: 11,
            bidir_support_density: Some(0.1),
            sham_found_groups: 10,
            outgoing_unique_groups: 1,
            backlink_unique_groups: 1,
            sham_unique_groups: 0,
            improved_domains: 1,
            outgoing_rescue_domains: 1,
            backlink_rescue_domains: 1,
            regressed_cases: 1,
            retained_payload_bytes: MAX_RETAINED_PAYLOAD_BYTES,
        };

        let reasons = development_rejection_reasons(metrics);

        assert!(reasons.iter().any(|reason| reason.contains("over C32")));
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("too few domains"))
        );
        assert!(reasons.iter().any(|reason| reason.contains("regressed")));
    }

    #[test]
    fn lost_groups_detects_equal_count_evidence_substitution() {
        let groups = vec![vec!["a".to_owned()], vec!["b".to_owned()]];
        let c32 = score_arm(vec!["a".to_owned()], &groups);
        let graph = score_arm(vec!["b".to_owned()], &groups);

        assert_eq!(c32.found_group_count, graph.found_group_count);
        assert_eq!(lost_groups(&graph, &c32, &groups), 1);
    }
}

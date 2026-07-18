//! Development-only diagnostic for reviewed-link navigation signal.
//!
//! The evaluator asks whether independently authored semantic concept pairs
//! are connected within two reviewed OKF-link hops more often in the real
//! topology than in a deterministic exact minimum-overlap,
//! weak-degree-preserving structural sham. It never nominates evidence and does
//! not alter production retrieval.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hint::black_box;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::mini_graph::{
    ExpansionDirection, GraphLinkInput, GraphNode, GraphNodeInput, GraphPurpose, LinkDisposition,
    MiniGraph, NodeId, NodeState, QueryScope,
};
use super::sham_graph::{StructuralShamStats, build_weak_structural_sham};
use crate::workspace_root;

const FIXTURE_DIRECTORY: &str = "fixtures/retrieval/okf-path-signal-development-v1";
const CONCEPTS_FILE: &str = "concepts.json";
const LINKS_FILE: &str = "links.json";
const CASES_FILE: &str = "cases.json";
const MANIFEST_FILE: &str = "manifest.json";
const REPORT_DIRECTORY: &str = "target/evals";
const EXPERIMENT_ID: &str = "okf-path-signal-development-v1";
const AUTHORING_PROTOCOL: &str = "procedurally-separated-concepts-links-pairs-v1";
const FIXTURE_SCHEMA_VERSION: u32 = 1;
const REPORT_SCHEMA_VERSION: u32 = 1;
const POLICY_VERSION: &str =
    "reviewed-links-bidirectional-path-at-most-2;sham=weak-degree-min-overlap-exact-v1";
const DOMAIN_COUNT: usize = 4;
const CONCEPTS_PER_DOMAIN: usize = 8;
const CASES_PER_DOMAIN: usize = 12;
const POSITIVE_CASES_PER_DOMAIN: usize = 6;
const MAX_PATH_P95_NANOS: u128 = 2_000_000;
const MAX_PROJECTION_MICROS: u128 = 1_000_000;
const MAX_PATH_SCRATCH_BYTES: usize = 8 * 1024;
const MAX_GRAPH_PAIR_PAYLOAD_BYTES: usize = 1024 * 1024;
const MIN_REAL_F1: f64 = 0.75;
const MIN_F1_DELTA_OVER_SHAM: f64 = 0.10;
const MIN_DOMAIN_WIN_COUNT: u32 = 3;
const MIN_SHAM_REWIRE_RATIO: f64 = 0.80;
const MAX_REAL_FALSE_POSITIVE_COUNT: u32 = 0;
const BENCHMARK_WARMUP_ROUNDS: usize = 50;
const BENCHMARK_ROUNDS: usize = 1_000;
const EPSILON: f64 = 1e-12;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
enum Language {
    Es,
    En,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConceptsFixture {
    schema_version: u32,
    experiment_id: String,
    authoring_role: String,
    domains: Vec<ConceptDomain>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConceptDomain {
    id: String,
    language: Language,
    title: String,
    concepts: Vec<ConceptFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConceptFixture {
    id: String,
    title: String,
    description: String,
    sections: Vec<ConceptSection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConceptSection {
    id: String,
    heading: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinksFixture {
    schema_version: u32,
    experiment_id: String,
    authoring_role: String,
    concepts_sha256: String,
    domains: Vec<LinkDomain>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkDomain {
    id: String,
    links: Vec<LinkFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkFixture {
    source_concept_id: String,
    target_concept_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CasesFixture {
    schema_version: u32,
    experiment_id: String,
    authoring_role: String,
    concepts_sha256: String,
    domains: Vec<CaseDomain>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseDomain {
    id: String,
    cases: Vec<PairCase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairCase {
    id: String,
    source_concept_id: String,
    target_concept_id: String,
    expected_related: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureManifest {
    schema_version: u32,
    experiment_id: String,
    authoring_protocol: String,
    concepts_sha256: String,
    links_sha256: String,
    cases_sha256: String,
}

struct LoadedFixture {
    concepts: ConceptsFixture,
    links: LinksFixture,
    cases: CasesFixture,
    concepts_sha256: String,
    links_sha256: String,
    cases_sha256: String,
    manifest_sha256: String,
}

#[derive(Debug, Clone, Copy)]
struct MaterializedCase {
    language: Language,
    domain_index: usize,
    source: NodeId,
    target: NodeId,
    expected_related: bool,
}

struct MaterializedFixture {
    graph: MiniGraph,
    sham_graph: MiniGraph,
    sham_stats: StructuralShamStats,
    scope: QueryScope,
    cases: Vec<MaterializedCase>,
    projection_micros: u128,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
struct ConfusionMatrix {
    true_positive: u32,
    false_positive: u32,
    true_negative: u32,
    false_negative: u32,
    indeterminate: u32,
}

impl ConfusionMatrix {
    fn observe(&mut self, expected: bool, predicted: Option<bool>) {
        match (expected, predicted) {
            (true, Some(true)) => self.true_positive = self.true_positive.saturating_add(1),
            (false, Some(true)) => self.false_positive = self.false_positive.saturating_add(1),
            (false, Some(false)) => self.true_negative = self.true_negative.saturating_add(1),
            (true, Some(false)) => self.false_negative = self.false_negative.saturating_add(1),
            (_, None) => self.indeterminate = self.indeterminate.saturating_add(1),
        }
    }

    fn determinate_count(self) -> u32 {
        self.true_positive
            .saturating_add(self.false_positive)
            .saturating_add(self.true_negative)
            .saturating_add(self.false_negative)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
struct ClassificationMetrics {
    confusion: ConfusionMatrix,
    precision: Option<f64>,
    recall: Option<f64>,
    f1: Option<f64>,
    accuracy: Option<f64>,
}

#[derive(Debug, Serialize)]
struct LanguageMetrics {
    language: Language,
    real: ClassificationMetrics,
    sham: ClassificationMetrics,
}

#[derive(Debug, Default, Serialize)]
struct TopologyReport {
    concept_count: u32,
    edge_count: u32,
    sham_retained_edge_count: u32,
    sham_rewired_edge_count: u32,
    sham_unchanged_collection_count: u32,
    sham_rewire_ratio: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
struct ResourceReport {
    graph_pair_retained_payload_bytes: usize,
    graph_pair_payload_budget_bytes: usize,
    path_requested_scratch_bytes: usize,
    path_scratch_limit_bytes: usize,
    projection_micros: u128,
    projection_budget_micros: u128,
    real_path_p95_nanos: Option<u128>,
    sham_path_p95_nanos: Option<u128>,
    path_latency_budget_nanos: u128,
    benchmark_rounds: u32,
    timing_gate_evaluated: bool,
}

#[derive(Debug, Default, Serialize)]
struct GateThresholds {
    min_real_f1: f64,
    min_f1_delta_over_sham: f64,
    min_domain_win_count: u32,
    min_sham_rewire_ratio: f64,
    max_real_false_positive_count: u32,
}

#[derive(Debug, Default, Serialize)]
struct EvaluationReport {
    schema_version: u32,
    experiment_id: &'static str,
    authoring_protocol: &'static str,
    policy_version: &'static str,
    concepts_sha256: String,
    links_sha256: String,
    cases_sha256: String,
    manifest_sha256: String,
    target_os: &'static str,
    target_arch: &'static str,
    build_profile: &'static str,
    domain_count: u32,
    case_count: u32,
    positive_case_count: u32,
    negative_case_count: u32,
    real_one_hop_count: u32,
    real_two_hop_count: u32,
    real: ClassificationMetrics,
    sham: ClassificationMetrics,
    by_language: Vec<LanguageMetrics>,
    domain_real_win_count: u32,
    path_symmetry_failure_count: u32,
    topology: TopologyReport,
    resources: ResourceReport,
    gate_thresholds: GateThresholds,
    development_gate_passed: bool,
    production_promotion_ready: bool,
    rejection_codes: Vec<&'static str>,
}

pub(crate) fn evaluate_path_signal() -> Result<()> {
    let fixture = load_fixture()?;
    let materialized = materialize_fixture(&fixture)?;
    let report = evaluate_materialized(&fixture, &materialized)?;
    let destination = write_report(&report)?;
    if report.development_gate_passed {
        println!(
            "OKF path-signal development gate passed; report written to {} (production promotion remains disabled)",
            destination.display()
        );
        return Ok(());
    }
    anyhow::bail!(
        "OKF path-signal development gate did not pass; report written to {}",
        destination.display()
    )
}

fn load_fixture() -> Result<LoadedFixture> {
    let directory = workspace_root().join(FIXTURE_DIRECTORY);
    let (concepts, concepts_sha256) = read_json::<ConceptsFixture>(&directory.join(CONCEPTS_FILE))?;
    let (links, links_sha256) = read_json::<LinksFixture>(&directory.join(LINKS_FILE))?;
    let (cases, cases_sha256) = read_json::<CasesFixture>(&directory.join(CASES_FILE))?;
    let (manifest, manifest_sha256) = read_json::<FixtureManifest>(&directory.join(MANIFEST_FILE))?;
    ensure!(
        manifest.concepts_sha256 == concepts_sha256
            && manifest.links_sha256 == links_sha256
            && manifest.cases_sha256 == cases_sha256,
        "path-signal manifest artifact hash mismatch"
    );
    ensure!(
        links.concepts_sha256 == concepts_sha256 && cases.concepts_sha256 == concepts_sha256,
        "path-signal author artifact does not bind the sealed concept inventory"
    );
    validate_fixture(&concepts, &links, &cases, &manifest)?;
    Ok(LoadedFixture {
        concepts,
        links,
        cases,
        concepts_sha256,
        links_sha256,
        cases_sha256,
        manifest_sha256,
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<(T, String)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let parsed =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    Ok((parsed, hex::encode(Sha256::digest(&bytes))))
}

fn validate_fixture(
    concepts: &ConceptsFixture,
    links: &LinksFixture,
    cases: &CasesFixture,
    manifest: &FixtureManifest,
) -> Result<()> {
    ensure!(
        [
            concepts.schema_version,
            links.schema_version,
            cases.schema_version,
            manifest.schema_version,
        ]
        .into_iter()
        .all(|version| version == FIXTURE_SCHEMA_VERSION),
        "unsupported path-signal fixture schema"
    );
    ensure!(
        [
            concepts.experiment_id.as_str(),
            links.experiment_id.as_str(),
            cases.experiment_id.as_str(),
            manifest.experiment_id.as_str(),
        ]
        .into_iter()
        .all(|id| id == EXPERIMENT_ID),
        "path-signal experiment identity mismatch"
    );
    ensure!(
        concepts.authoring_role == "concept-inventory-without-links-or-pairs"
            && links.authoring_role == "reviewed-links-without-pairs"
            && cases.authoring_role == "semantic-pairs-without-links"
            && manifest.authoring_protocol == AUTHORING_PROTOCOL,
        "path-signal authoring protocol mismatch"
    );
    ensure!(
        concepts.domains.len() == DOMAIN_COUNT,
        "path-signal fixture must contain exactly {DOMAIN_COUNT} domains"
    );

    let mut domain_ids = BTreeSet::new();
    let mut concept_ids = BTreeSet::new();
    let mut language_counts = BTreeMap::<Language, usize>::new();
    let mut concepts_by_domain = BTreeMap::<&str, BTreeSet<&str>>::new();
    for domain in &concepts.domains {
        validate_identifier(&domain.id, "domain")?;
        ensure!(
            domain_ids.insert(domain.id.as_str()),
            "duplicate path-signal domain"
        );
        ensure_nonempty(&domain.title, "domain title")?;
        ensure!(
            domain.concepts.len() == CONCEPTS_PER_DOMAIN,
            "path-signal domain must contain exactly {CONCEPTS_PER_DOMAIN} concepts"
        );
        *language_counts.entry(domain.language).or_default() += 1;
        let mut local_ids = BTreeSet::new();
        for concept in &domain.concepts {
            validate_identifier(&concept.id, "concept")?;
            ensure!(
                concept_ids.insert(concept.id.as_str()),
                "duplicate path-signal concept"
            );
            ensure!(
                local_ids.insert(concept.id.as_str()),
                "duplicate concept within domain"
            );
            ensure_nonempty(&concept.title, "concept title")?;
            ensure_nonempty(&concept.description, "concept description")?;
            ensure_no_link_hints(&concept.title)?;
            ensure_no_link_hints(&concept.description)?;
            ensure!(
                concept.sections.len() == 2,
                "path-signal concept must contain two sections"
            );
            let mut section_ids = BTreeSet::new();
            for section in &concept.sections {
                validate_identifier(&section.id, "section")?;
                ensure!(
                    section_ids.insert(section.id.as_str()),
                    "duplicate concept section"
                );
                ensure_nonempty(&section.heading, "section heading")?;
                ensure_nonempty(&section.text, "section text")?;
                ensure_no_link_hints(&section.heading)?;
                ensure_no_link_hints(&section.text)?;
            }
        }
        concepts_by_domain.insert(domain.id.as_str(), local_ids);
    }
    ensure!(
        language_counts.get(&Language::Es) == Some(&2)
            && language_counts.get(&Language::En) == Some(&2),
        "path-signal fixture must contain two Spanish and two English domains"
    );
    validate_links(links, &domain_ids, &concepts_by_domain)?;
    validate_cases(cases, &domain_ids, &concepts_by_domain)?;
    Ok(())
}

fn validate_links(
    fixture: &LinksFixture,
    expected_domains: &BTreeSet<&str>,
    concepts_by_domain: &BTreeMap<&str, BTreeSet<&str>>,
) -> Result<()> {
    let mut observed_domains = BTreeSet::new();
    for domain in &fixture.domains {
        ensure!(
            expected_domains.contains(domain.id.as_str())
                && observed_domains.insert(domain.id.as_str()),
            "path-signal links contain an unknown or duplicate domain"
        );
        let concepts = concepts_by_domain
            .get(domain.id.as_str())
            .context("path-signal link domain lost its concepts")?;
        let mut links = BTreeSet::new();
        let mut undirected_links = BTreeSet::new();
        let mut participants = BTreeSet::new();
        for link in &domain.links {
            ensure!(
                link.source_concept_id != link.target_concept_id,
                "path-signal links reject self-links"
            );
            ensure!(
                concepts.contains(link.source_concept_id.as_str())
                    && concepts.contains(link.target_concept_id.as_str()),
                "path-signal link endpoint is outside its domain"
            );
            ensure!(
                links.insert((
                    link.source_concept_id.as_str(),
                    link.target_concept_id.as_str()
                )),
                "duplicate path-signal directed link"
            );
            let undirected = if link.source_concept_id < link.target_concept_id {
                (
                    link.source_concept_id.as_str(),
                    link.target_concept_id.as_str(),
                )
            } else {
                (
                    link.target_concept_id.as_str(),
                    link.source_concept_id.as_str(),
                )
            };
            ensure!(
                undirected_links.insert(undirected),
                "path-signal links reject reciprocal pairs"
            );
            participants.insert(link.source_concept_id.as_str());
            participants.insert(link.target_concept_id.as_str());
        }
        ensure!(
            participants == *concepts,
            "every path-signal concept must participate in a reviewed link"
        );
    }
    ensure!(
        observed_domains == *expected_domains,
        "path-signal links do not cover every domain"
    );
    Ok(())
}

fn validate_cases(
    fixture: &CasesFixture,
    expected_domains: &BTreeSet<&str>,
    concepts_by_domain: &BTreeMap<&str, BTreeSet<&str>>,
) -> Result<()> {
    let mut observed_domains = BTreeSet::new();
    let mut case_ids = BTreeSet::new();
    for domain in &fixture.domains {
        ensure!(
            expected_domains.contains(domain.id.as_str())
                && observed_domains.insert(domain.id.as_str()),
            "path-signal pairs contain an unknown or duplicate domain"
        );
        ensure!(
            domain.cases.len() == CASES_PER_DOMAIN,
            "path-signal domain must contain exactly {CASES_PER_DOMAIN} pairs"
        );
        let concepts = concepts_by_domain
            .get(domain.id.as_str())
            .context("path-signal pair domain lost its concepts")?;
        let mut pairs = BTreeSet::new();
        let mut positive_count = 0_usize;
        for case in &domain.cases {
            validate_identifier(&case.id, "case")?;
            ensure!(
                case_ids.insert(case.id.as_str()),
                "duplicate path-signal case id"
            );
            ensure!(
                case.source_concept_id < case.target_concept_id,
                "path-signal pairs must use canonical endpoint order"
            );
            ensure!(
                concepts.contains(case.source_concept_id.as_str())
                    && concepts.contains(case.target_concept_id.as_str()),
                "path-signal pair endpoint is outside its domain"
            );
            ensure!(
                pairs.insert((
                    case.source_concept_id.as_str(),
                    case.target_concept_id.as_str()
                )),
                "duplicate path-signal concept pair"
            );
            positive_count += usize::from(case.expected_related);
        }
        ensure!(
            positive_count == POSITIVE_CASES_PER_DOMAIN,
            "path-signal domain must balance six positive and six negative pairs"
        );
    }
    ensure!(
        observed_domains == *expected_domains,
        "path-signal pairs do not cover every domain"
    );
    Ok(())
}

fn validate_identifier(value: &str, kind: &str) -> Result<()> {
    ensure!(!value.is_empty(), "path-signal {kind} identifier is empty");
    ensure!(
        value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        }),
        "path-signal {kind} identifier must use lowercase ASCII, digits, `_` or `-`"
    );
    Ok(())
}

fn ensure_nonempty(value: &str, kind: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "path-signal {kind} is empty");
    Ok(())
}

fn ensure_no_link_hints(value: &str) -> Result<()> {
    let normalized = value.to_ascii_lowercase();
    ensure!(
        !normalized.contains("](")
            && !normalized.contains("http://")
            && !normalized.contains("https://"),
        "path-signal concept inventory contains a link hint"
    );
    Ok(())
}

fn materialize_fixture(fixture: &LoadedFixture) -> Result<MaterializedFixture> {
    let projection_started = Instant::now();
    let languages = fixture
        .concepts
        .domains
        .iter()
        .map(|domain| (domain.id.as_str(), domain.language))
        .collect::<BTreeMap<_, _>>();
    let domain_indexes = fixture
        .concepts
        .domains
        .iter()
        .enumerate()
        .map(|(index, domain)| (domain.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut node_inputs = Vec::new();
    let mut concept_by_label = HashMap::new();
    let mut stable_label_by_concept = HashMap::new();
    let mut collections = BTreeSet::new();
    for domain in &fixture.concepts.domains {
        let collection_id = collection_uuid(&domain.id);
        collections.insert(collection_id);
        for concept in &domain.concepts {
            let concept_id = concept_uuid(&domain.id, &concept.id);
            node_inputs.push(GraphNodeInput {
                node: GraphNode {
                    concept_id,
                    collection_id,
                },
                state: NodeState::Current,
            });
            concept_by_label.insert((domain.id.as_str(), concept.id.as_str()), concept_id);
            stable_label_by_concept
                .insert(concept_id, opaque_stable_label(&domain.id, &concept.id));
        }
    }
    let mut link_inputs = Vec::new();
    for domain in &fixture.links.domains {
        for link in &domain.links {
            link_inputs.push(GraphLinkInput {
                source: *concept_by_label
                    .get(&(domain.id.as_str(), link.source_concept_id.as_str()))
                    .context("path-signal source concept disappeared")?,
                target: *concept_by_label
                    .get(&(domain.id.as_str(), link.target_concept_id.as_str()))
                    .context("path-signal target concept disappeared")?,
                disposition: LinkDisposition::ReviewedInternal,
            });
        }
    }
    let structural_sham =
        build_weak_structural_sham(&node_inputs, &link_inputs, &stable_label_by_concept)?;
    ensure!(
        !contains_reciprocal_links(&structural_sham.links),
        "path-signal sham introduced a reciprocal pair"
    );
    ensure!(
        weak_degrees(&link_inputs) == weak_degrees(&structural_sham.links),
        "path-signal sham changed a bidirectional traversal degree"
    );
    let graph = MiniGraph::build(&node_inputs, &link_inputs)?;
    let sham_graph = MiniGraph::build(&node_inputs, &structural_sham.links)?;
    ensure!(
        graph.node_count() == sham_graph.node_count()
            && graph.edge_count() == sham_graph.edge_count(),
        "path-signal sham changed graph size"
    );
    let scope = QueryScope {
        purpose: GraphPurpose::LocalAssistant,
        authorized_collections: collections,
        external_ai_collections: BTreeSet::new(),
    };
    let mut cases = Vec::new();
    for domain in &fixture.cases.domains {
        for case in &domain.cases {
            let source_concept = *concept_by_label
                .get(&(domain.id.as_str(), case.source_concept_id.as_str()))
                .context("path-signal case source disappeared")?;
            let target_concept = *concept_by_label
                .get(&(domain.id.as_str(), case.target_concept_id.as_str()))
                .context("path-signal case target disappeared")?;
            cases.push(MaterializedCase {
                language: *languages
                    .get(domain.id.as_str())
                    .context("path-signal case language disappeared")?,
                domain_index: *domain_indexes
                    .get(domain.id.as_str())
                    .context("path-signal case domain index disappeared")?,
                source: graph
                    .node_id(source_concept)
                    .context("path-signal source is not current")?,
                target: graph
                    .node_id(target_concept)
                    .context("path-signal target is not current")?,
                expected_related: case.expected_related,
            });
        }
    }
    Ok(MaterializedFixture {
        graph,
        sham_graph,
        sham_stats: structural_sham.stats,
        scope,
        cases,
        projection_micros: projection_started.elapsed().as_micros(),
    })
}

fn evaluate_materialized(
    fixture: &LoadedFixture,
    materialized: &MaterializedFixture,
) -> Result<EvaluationReport> {
    let mut real = ConfusionMatrix::default();
    let mut sham = ConfusionMatrix::default();
    let mut real_by_language = BTreeMap::<Language, ConfusionMatrix>::new();
    let mut sham_by_language = BTreeMap::<Language, ConfusionMatrix>::new();
    let mut real_by_domain = [ConfusionMatrix::default(); DOMAIN_COUNT];
    let mut sham_by_domain = [ConfusionMatrix::default(); DOMAIN_COUNT];
    let mut real_one_hop_count = 0_u32;
    let mut real_two_hop_count = 0_u32;
    let mut path_symmetry_failure_count = 0_u32;
    for case in &materialized.cases {
        let real_path = materialized.graph.path_within_two_hops(
            case.source,
            case.target,
            &materialized.scope,
            ExpansionDirection::Bidirectional,
        );
        let sham_path = materialized.sham_graph.path_within_two_hops(
            case.source,
            case.target,
            &materialized.scope,
            ExpansionDirection::Bidirectional,
        );
        let reversed = materialized.graph.path_within_two_hops(
            case.target,
            case.source,
            &materialized.scope,
            ExpansionDirection::Bidirectional,
        );
        let sham_reversed = materialized.sham_graph.path_within_two_hops(
            case.target,
            case.source,
            &materialized.scope,
            ExpansionDirection::Bidirectional,
        );
        if real_path.status != reversed.status {
            path_symmetry_failure_count = path_symmetry_failure_count.saturating_add(1);
        }
        if sham_path.status != sham_reversed.status {
            path_symmetry_failure_count = path_symmetry_failure_count.saturating_add(1);
        }
        match real_path.status.hops() {
            Some(1) => real_one_hop_count = real_one_hop_count.saturating_add(1),
            Some(2) => real_two_hop_count = real_two_hop_count.saturating_add(1),
            Some(0) | Some(3..) => {
                path_symmetry_failure_count = path_symmetry_failure_count.saturating_add(1)
            }
            None => {}
        }
        let real_prediction = real_path.status.connected();
        let sham_prediction = sham_path.status.connected();
        real.observe(case.expected_related, real_prediction);
        sham.observe(case.expected_related, sham_prediction);
        real_by_language
            .entry(case.language)
            .or_default()
            .observe(case.expected_related, real_prediction);
        sham_by_language
            .entry(case.language)
            .or_default()
            .observe(case.expected_related, sham_prediction);
        real_by_domain[case.domain_index].observe(case.expected_related, real_prediction);
        sham_by_domain[case.domain_index].observe(case.expected_related, sham_prediction);
    }

    let real_metrics = classification_metrics(real);
    let sham_metrics = classification_metrics(sham);
    let by_language = [Language::Es, Language::En]
        .into_iter()
        .map(|language| LanguageMetrics {
            language,
            real: classification_metrics(
                real_by_language.get(&language).copied().unwrap_or_default(),
            ),
            sham: classification_metrics(
                sham_by_language.get(&language).copied().unwrap_or_default(),
            ),
        })
        .collect::<Vec<_>>();
    let domain_real_win_count = real_by_domain
        .iter()
        .zip(&sham_by_domain)
        .filter(|(real, sham)| {
            metric_value(classification_metrics(**real).f1)
                > metric_value(classification_metrics(**sham).f1) + EPSILON
        })
        .count();
    let domain_real_win_count =
        u32::try_from(domain_real_win_count).context("too many path-signal domains")?;
    let edge_count = materialized.graph.edge_count();
    let sham_rewire_ratio = (edge_count > 0)
        .then(|| f64::from(materialized.sham_stats.rewired_edge_count) / f64::from(edge_count));
    let topology = TopologyReport {
        concept_count: materialized.graph.node_count(),
        edge_count,
        sham_retained_edge_count: materialized.sham_stats.retained_original_edge_count,
        sham_rewired_edge_count: materialized.sham_stats.rewired_edge_count,
        sham_unchanged_collection_count: materialized.sham_stats.unchanged_collection_count,
        sham_rewire_ratio,
    };
    let resources = benchmark_paths(materialized)?;
    let positive_case_count = materialized
        .cases
        .iter()
        .filter(|case| case.expected_related)
        .count();
    let mut rejection_codes = Vec::new();
    if metric_value(real_metrics.f1) + EPSILON < MIN_REAL_F1 {
        rejection_codes.push("real_f1_below_minimum");
    }
    if metric_value(real_metrics.f1) - metric_value(sham_metrics.f1) + EPSILON
        < MIN_F1_DELTA_OVER_SHAM
    {
        rejection_codes.push("real_f1_delta_over_sham_below_minimum");
    }
    if real_metrics.confusion.false_positive > MAX_REAL_FALSE_POSITIVE_COUNT {
        rejection_codes.push("real_false_attribution_count_nonzero");
    }
    if domain_real_win_count < MIN_DOMAIN_WIN_COUNT {
        rejection_codes.push("real_domain_win_count_below_minimum");
    }
    if by_language
        .iter()
        .any(|metrics| metric_value(metrics.real.f1) + EPSILON < metric_value(metrics.sham.f1))
    {
        rejection_codes.push("real_language_f1_regressed_against_sham");
    }
    if real_metrics.confusion.indeterminate > 0 || sham_metrics.confusion.indeterminate > 0 {
        rejection_codes.push("path_edge_budget_exhausted");
    }
    if path_symmetry_failure_count > 0 {
        rejection_codes.push("bidirectional_path_invariant_failed");
    }
    if topology
        .sham_rewire_ratio
        .is_none_or(|ratio| ratio + EPSILON < MIN_SHAM_REWIRE_RATIO)
    {
        rejection_codes.push("sham_rewire_ratio_below_minimum");
    }
    if topology.sham_unchanged_collection_count > 0 {
        rejection_codes.push("sham_contains_unchanged_collection");
    }
    if resources.graph_pair_retained_payload_bytes > MAX_GRAPH_PAIR_PAYLOAD_BYTES {
        rejection_codes.push("graph_pair_payload_exceeded");
    }
    if resources.path_requested_scratch_bytes > MAX_PATH_SCRATCH_BYTES {
        rejection_codes.push("path_scratch_budget_exceeded");
    }
    if resources.projection_micros > MAX_PROJECTION_MICROS {
        rejection_codes.push("graph_projection_latency_exceeded");
    }
    if !resources.timing_gate_evaluated {
        rejection_codes.push("release_timing_not_evaluated");
    } else if resources
        .real_path_p95_nanos
        .is_none_or(|value| value > MAX_PATH_P95_NANOS)
    {
        rejection_codes.push("real_path_latency_p95_exceeded");
    }
    if resources.timing_gate_evaluated
        && resources
            .sham_path_p95_nanos
            .is_none_or(|value| value > MAX_PATH_P95_NANOS)
    {
        rejection_codes.push("sham_path_latency_p95_exceeded");
    }
    Ok(EvaluationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        experiment_id: EXPERIMENT_ID,
        authoring_protocol: AUTHORING_PROTOCOL,
        policy_version: POLICY_VERSION,
        concepts_sha256: fixture.concepts_sha256.clone(),
        links_sha256: fixture.links_sha256.clone(),
        cases_sha256: fixture.cases_sha256.clone(),
        manifest_sha256: fixture.manifest_sha256.clone(),
        target_os: std::env::consts::OS,
        target_arch: std::env::consts::ARCH,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        domain_count: u32::try_from(DOMAIN_COUNT).context("too many path-signal domains")?,
        case_count: u32::try_from(materialized.cases.len())
            .context("too many path-signal cases")?,
        positive_case_count: u32::try_from(positive_case_count)
            .context("too many positive path-signal cases")?,
        negative_case_count: u32::try_from(
            materialized.cases.len().saturating_sub(positive_case_count),
        )
        .context("too many negative path-signal cases")?,
        real_one_hop_count,
        real_two_hop_count,
        real: real_metrics,
        sham: sham_metrics,
        by_language,
        domain_real_win_count,
        path_symmetry_failure_count,
        topology,
        resources,
        gate_thresholds: GateThresholds {
            min_real_f1: MIN_REAL_F1,
            min_f1_delta_over_sham: MIN_F1_DELTA_OVER_SHAM,
            min_domain_win_count: MIN_DOMAIN_WIN_COUNT,
            min_sham_rewire_ratio: MIN_SHAM_REWIRE_RATIO,
            max_real_false_positive_count: MAX_REAL_FALSE_POSITIVE_COUNT,
        },
        development_gate_passed: rejection_codes.is_empty(),
        production_promotion_ready: false,
        rejection_codes,
    })
}

fn classification_metrics(confusion: ConfusionMatrix) -> ClassificationMetrics {
    let predicted_positive = confusion
        .true_positive
        .saturating_add(confusion.false_positive);
    let actual_positive = confusion
        .true_positive
        .saturating_add(confusion.false_negative);
    let precision = (predicted_positive > 0)
        .then(|| f64::from(confusion.true_positive) / f64::from(predicted_positive));
    let recall = (actual_positive > 0)
        .then(|| f64::from(confusion.true_positive) / f64::from(actual_positive));
    let f1 = precision.zip(recall).and_then(|(precision, recall)| {
        let denominator = precision + recall;
        (denominator > 0.0).then(|| 2.0 * precision * recall / denominator)
    });
    let determinate = confusion.determinate_count();
    let accuracy = (determinate > 0).then(|| {
        f64::from(
            confusion
                .true_positive
                .saturating_add(confusion.true_negative),
        ) / f64::from(determinate)
    });
    ClassificationMetrics {
        confusion,
        precision,
        recall,
        f1,
        accuracy,
    }
}

fn metric_value(metric: Option<f64>) -> f64 {
    metric.unwrap_or(0.0)
}

fn benchmark_paths(materialized: &MaterializedFixture) -> Result<ResourceReport> {
    for _ in 0..BENCHMARK_WARMUP_ROUNDS {
        for case in &materialized.cases {
            black_box(materialized.graph.path_within_two_hops(
                black_box(case.source),
                black_box(case.target),
                black_box(&materialized.scope),
                ExpansionDirection::Bidirectional,
            ));
            black_box(materialized.sham_graph.path_within_two_hops(
                black_box(case.source),
                black_box(case.target),
                black_box(&materialized.scope),
                ExpansionDirection::Bidirectional,
            ));
        }
    }
    let timing_capacity = BENCHMARK_ROUNDS.saturating_mul(materialized.cases.len());
    let mut real_timings = Vec::with_capacity(timing_capacity);
    let mut sham_timings = Vec::with_capacity(timing_capacity);
    for _ in 0..BENCHMARK_ROUNDS {
        for case in &materialized.cases {
            let real_started = Instant::now();
            black_box(materialized.graph.path_within_two_hops(
                black_box(case.source),
                black_box(case.target),
                black_box(&materialized.scope),
                ExpansionDirection::Bidirectional,
            ));
            real_timings.push(real_started.elapsed().as_nanos());
            let sham_started = Instant::now();
            black_box(materialized.sham_graph.path_within_two_hops(
                black_box(case.source),
                black_box(case.target),
                black_box(&materialized.scope),
                ExpansionDirection::Bidirectional,
            ));
            sham_timings.push(sham_started.elapsed().as_nanos());
        }
    }
    real_timings.sort_unstable();
    sham_timings.sort_unstable();
    Ok(ResourceReport {
        graph_pair_retained_payload_bytes: materialized
            .graph
            .retained_payload_bytes()
            .saturating_add(materialized.sham_graph.retained_payload_bytes()),
        graph_pair_payload_budget_bytes: MAX_GRAPH_PAIR_PAYLOAD_BYTES,
        path_requested_scratch_bytes: MiniGraph::path_requested_scratch_bytes(),
        path_scratch_limit_bytes: MAX_PATH_SCRATCH_BYTES,
        projection_micros: materialized.projection_micros,
        projection_budget_micros: MAX_PROJECTION_MICROS,
        real_path_p95_nanos: percentile(&real_timings, 95),
        sham_path_p95_nanos: percentile(&sham_timings, 95),
        path_latency_budget_nanos: MAX_PATH_P95_NANOS,
        benchmark_rounds: u32::try_from(BENCHMARK_ROUNDS).context("too many benchmark rounds")?,
        timing_gate_evaluated: !cfg!(debug_assertions),
    })
}

fn percentile(sorted: &[u128], percentile: usize) -> Option<u128> {
    if sorted.is_empty() {
        return None;
    }
    let rank = percentile
        .saturating_mul(sorted.len())
        .div_ceil(100)
        .saturating_sub(1)
        .min(sorted.len().saturating_sub(1));
    sorted.get(rank).copied()
}

fn weak_degrees(links: &[GraphLinkInput]) -> BTreeMap<Uuid, usize> {
    let mut neighbors = BTreeMap::<Uuid, BTreeSet<Uuid>>::new();
    for link in links {
        neighbors
            .entry(link.source)
            .or_default()
            .insert(link.target);
        neighbors
            .entry(link.target)
            .or_default()
            .insert(link.source);
    }
    neighbors
        .into_iter()
        .map(|(node, neighbors)| (node, neighbors.len()))
        .collect()
}

fn contains_reciprocal_links(links: &[GraphLinkInput]) -> bool {
    let pairs = links
        .iter()
        .map(|link| (link.source, link.target))
        .collect::<BTreeSet<_>>();
    pairs
        .iter()
        .any(|(source, target)| pairs.contains(&(*target, *source)))
}

fn concept_uuid(domain_id: &str, concept_id: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("urn:airwiki:path-signal:{domain_id}:concept:{concept_id}").as_bytes(),
    )
}

fn collection_uuid(domain_id: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("urn:airwiki:path-signal:{domain_id}:collection").as_bytes(),
    )
}

fn opaque_stable_label(domain_id: &str, concept_id: &str) -> String {
    hex::encode(Sha256::digest(
        format!("airwiki-path-signal-stable-label:{domain_id}:{concept_id}").as_bytes(),
    ))
}

fn report_path(manifest_sha256: &str, epoch_seconds: u64) -> Result<PathBuf> {
    let manifest_prefix = manifest_sha256
        .get(..12)
        .context("path-signal manifest hash is too short")?;
    Ok(workspace_root().join(REPORT_DIRECTORY).join(format!(
        "retrieval-okf-path-signal-development-{manifest_prefix}-{}-{}-{epoch_seconds}.json",
        std::env::consts::OS,
        std::env::consts::ARCH
    )))
}

fn write_report(report: &EvaluationReport) -> Result<PathBuf> {
    let epoch_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs();
    let destination = report_path(&report.manifest_sha256, epoch_seconds)?;
    let parent = destination
        .parent()
        .context("path-signal report has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let mut contents = serde_json::to_string_pretty(report)?;
    contents.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .with_context(|| format!("creating unique report {}", destination.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("writing {}", destination.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing {}", destination.display()))?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn fixture_is_valid_and_binds_the_separated_artifacts() {
        let fixture = load_fixture().unwrap();

        assert_eq!(fixture.concepts.domains.len(), DOMAIN_COUNT);
        assert_eq!(fixture.cases.domains.len(), DOMAIN_COUNT);
        assert_eq!(fixture.links.domains.len(), DOMAIN_COUNT);
    }

    #[test]
    fn fixture_materializes_a_size_preserving_structural_sham_without_scoring_cases() {
        let fixture = load_fixture().unwrap();

        let materialized = materialize_fixture(&fixture).unwrap();

        assert_eq!(
            materialized.graph.node_count(),
            materialized.sham_graph.node_count()
        );
        assert_eq!(
            materialized.graph.edge_count(),
            materialized.sham_graph.edge_count()
        );
    }

    #[test]
    fn pair_schema_rejects_topology_hints() {
        let value = json!({
            "schema_version": 1,
            "experiment_id": EXPERIMENT_ID,
            "authoring_role": "semantic-pairs-without-links",
            "concepts_sha256": "00",
            "domains": [{
                "id": "domain",
                "cases": [{
                    "id": "case",
                    "source_concept_id": "alpha",
                    "target_concept_id": "beta",
                    "expected_related": true,
                    "expected_hops": 2
                }]
            }]
        });

        let error = serde_json::from_value::<CasesFixture>(value).unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn report_schema_has_no_fixture_content_or_concept_identity_fields() {
        let serialized = serde_json::to_value(EvaluationReport::default()).unwrap();

        for forbidden in [
            "domain_id",
            "case_id",
            "concept_id",
            "source_concept_id",
            "target_concept_id",
            "title",
            "description",
            "heading",
            "text",
            "links",
            "path",
            "question",
        ] {
            assert!(!contains_json_key(&serialized, forbidden));
        }
    }

    fn contains_json_key(value: &serde_json::Value, needle: &str) -> bool {
        match value {
            serde_json::Value::Object(object) => {
                object.contains_key(needle)
                    || object
                        .values()
                        .any(|nested| contains_json_key(nested, needle))
            }
            serde_json::Value::Array(values) => values
                .iter()
                .any(|nested| contains_json_key(nested, needle)),
            serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_) => false,
        }
    }

    #[test]
    fn classification_counts_indeterminate_paths_without_calling_them_negative() {
        let mut confusion = ConfusionMatrix::default();

        confusion.observe(true, None);
        confusion.observe(false, None);

        assert_eq!(confusion.indeterminate, 2);
        assert_eq!(confusion.determinate_count(), 0);
        assert_eq!(classification_metrics(confusion).f1, None);
    }
}

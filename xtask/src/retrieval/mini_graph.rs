//! Mechanistic ablation for a compact, in-process OKF concept graph.
//!
//! This module is deliberately confined to `xtask`. It tests whether one-hop
//! expansion over reviewed internal links can add candidate coverage that is
//! not reproduced by simply enlarging the hybrid candidate pool. It does not
//! alter production search, authorization, SQLite, or the OKF profile.

use std::collections::{BTreeMap, BTreeSet};
use std::hint::black_box;
use std::mem::{size_of, size_of_val};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{replace_file, workspace_root};

const FIXTURE_PATH: &str = "fixtures/retrieval/mini-graph-v1.json";
const REPORT_DIRECTORY: &str = "target/evals";
const FIXTURE_SCHEMA_VERSION: u32 = 1;
const REPORT_SCHEMA_VERSION: u32 = 1;
const GRAPH_CONTRACT_VERSION: &str = "okf-reviewed-concept-links-v1";
pub(in crate::retrieval) const BASELINE_LIMIT: usize = 10;
pub(in crate::retrieval) const CONTROL_LIMIT: usize = 32;
const MAX_EXPANDED_CANDIDATES: usize = CONTROL_LIMIT;
const MAX_GRAPH_NODES: usize = 500;
const MAX_GRAPH_EDGES: usize = 2_000;
const MAX_GRAPH_LINK_INPUTS: usize = 4_000;
const MAX_SCANNED_EDGES: usize = 128;
const MAX_RETAINED_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_BUILD_MICROS_P95: u128 = 50_000;
const MAX_EXPANSION_MICROS_P95: u128 = 5_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphFixture {
    schema_version: u32,
    experiment_id: String,
    cases: Vec<GraphCase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphCase {
    id: String,
    purpose: GraphPurpose,
    default_collection_id: String,
    authorized_collection_ids: Vec<String>,
    #[serde(default)]
    external_ai_collection_ids: Option<Vec<String>>,
    hybrid_ranked_concept_ids: Vec<String>,
    #[serde(default)]
    node_overrides: Vec<NodeOverride>,
    links: Vec<LinkFixture>,
    required_groups: Vec<Vec<String>>,
    forbidden_concept_ids: Vec<String>,
    expected_found_groups: ExpectedFoundGroups,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(in crate::retrieval) enum GraphPurpose {
    LocalAssistant,
    ExternalAi,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeOverride {
    id: String,
    #[serde(default)]
    collection_id: Option<String>,
    #[serde(default)]
    state: NodeState,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(in crate::retrieval) enum NodeState {
    #[default]
    Current,
    Withdrawn,
    Stale,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkFixture {
    source: String,
    target: String,
    disposition: LinkDisposition,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(in crate::retrieval) enum LinkDisposition {
    ReviewedInternal,
    Broken,
    Unsafe,
    External,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedFoundGroups {
    c10: u32,
    c32: u32,
    g1_out: u32,
    g1_bidir: u32,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::retrieval) struct NodeId(pub(in crate::retrieval) u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::retrieval) struct GraphNode {
    pub(in crate::retrieval) concept_id: Uuid,
    pub(in crate::retrieval) collection_id: Uuid,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::retrieval) struct GraphNodeInput {
    pub(in crate::retrieval) node: GraphNode,
    pub(in crate::retrieval) state: NodeState,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::retrieval) struct GraphLinkInput {
    pub(in crate::retrieval) source: Uuid,
    pub(in crate::retrieval) target: Uuid,
    pub(in crate::retrieval) disposition: LinkDisposition,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
struct GraphBuildStats {
    node_count: u32,
    edge_count: u32,
    duplicate_edge_count: u32,
    ignored_non_internal_count: u32,
    ignored_inactive_count: u32,
    ignored_cross_collection_count: u32,
    ignored_self_link_count: u32,
}

#[derive(Debug, Clone)]
pub(in crate::retrieval) struct MiniGraph {
    nodes: Box<[GraphNode]>,
    outgoing: Box<[Box<[NodeId]>]>,
    incoming: Box<[Box<[NodeId]>]>,
    fingerprint: String,
    stats: GraphBuildStats,
}

impl MiniGraph {
    pub(in crate::retrieval) fn build(
        nodes: &[GraphNodeInput],
        links: &[GraphLinkInput],
    ) -> Result<Self> {
        ensure!(
            links.len() <= MAX_GRAPH_LINK_INPUTS,
            "mini graph exceeds the {MAX_GRAPH_LINK_INPUTS} link-input budget"
        );
        let mut current_nodes = nodes
            .iter()
            .filter(|input| input.state == NodeState::Current)
            .map(|input| input.node)
            .collect::<Vec<_>>();
        current_nodes.sort_unstable_by_key(|node| node.concept_id);
        ensure!(
            current_nodes.len() <= MAX_GRAPH_NODES,
            "mini graph exceeds the {MAX_GRAPH_NODES} current-node budget"
        );
        ensure!(
            current_nodes
                .windows(2)
                .all(|window| window[0].concept_id != window[1].concept_id),
            "mini graph contains duplicate concept identities"
        );

        let mut outgoing = vec![Vec::<NodeId>::new(); current_nodes.len()];
        let mut incoming = vec![Vec::<NodeId>::new(); current_nodes.len()];
        let mut stats = GraphBuildStats::default();

        for link in links {
            if link.disposition != LinkDisposition::ReviewedInternal {
                stats.ignored_non_internal_count =
                    stats.ignored_non_internal_count.saturating_add(1);
                continue;
            }
            let Some(source) = node_id(&current_nodes, link.source) else {
                stats.ignored_inactive_count = stats.ignored_inactive_count.saturating_add(1);
                continue;
            };
            let Some(target) = node_id(&current_nodes, link.target) else {
                stats.ignored_inactive_count = stats.ignored_inactive_count.saturating_add(1);
                continue;
            };
            let source_index = node_index(source)?;
            let target_index = node_index(target)?;
            if source == target {
                stats.ignored_self_link_count = stats.ignored_self_link_count.saturating_add(1);
                continue;
            }
            if current_nodes[source_index].collection_id
                != current_nodes[target_index].collection_id
            {
                stats.ignored_cross_collection_count =
                    stats.ignored_cross_collection_count.saturating_add(1);
                continue;
            }
            outgoing[source_index].push(target);
            incoming[target_index].push(source);
        }

        let mut edge_count = 0_usize;
        let mut duplicate_edge_count = 0_usize;
        for neighbors in outgoing.iter_mut().chain(&mut incoming) {
            let before = neighbors.len();
            neighbors.sort_unstable();
            neighbors.dedup();
            duplicate_edge_count = duplicate_edge_count.saturating_add(before - neighbors.len());
        }
        for neighbors in &outgoing {
            edge_count = edge_count.saturating_add(neighbors.len());
        }
        ensure!(
            edge_count <= MAX_GRAPH_EDGES,
            "mini graph exceeds the {MAX_GRAPH_EDGES} directed-edge budget"
        );

        stats.node_count = u32::try_from(current_nodes.len()).unwrap_or(u32::MAX);
        stats.edge_count = u32::try_from(edge_count).unwrap_or(u32::MAX);
        // Each duplicate is observed in both adjacency directions.
        stats.duplicate_edge_count = u32::try_from(duplicate_edge_count / 2).unwrap_or(u32::MAX);
        let nodes = current_nodes.into_boxed_slice();
        let outgoing = boxed_adjacency(outgoing);
        let incoming = boxed_adjacency(incoming);
        let fingerprint = graph_fingerprint(&nodes, &outgoing);
        Ok(Self {
            nodes,
            outgoing,
            incoming,
            fingerprint,
            stats,
        })
    }

    pub(in crate::retrieval) fn node_id(&self, concept_id: Uuid) -> Option<NodeId> {
        node_id(&self.nodes, concept_id)
    }

    pub(in crate::retrieval) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub(in crate::retrieval) fn node_count(&self) -> u32 {
        self.stats.node_count
    }

    pub(in crate::retrieval) fn edge_count(&self) -> u32 {
        self.stats.edge_count
    }

    pub(in crate::retrieval) fn node(&self, id: NodeId) -> Option<&GraphNode> {
        node_index(id).ok().and_then(|index| self.nodes.get(index))
    }

    pub(in crate::retrieval) fn retained_payload_bytes(&self) -> usize {
        size_of_val(self)
            .saturating_add(self.fingerprint.capacity())
            .saturating_add(self.nodes.len().saturating_mul(size_of::<GraphNode>()))
            .saturating_add(
                self.outgoing
                    .len()
                    .saturating_mul(size_of::<Box<[NodeId]>>()),
            )
            .saturating_add(
                self.incoming
                    .len()
                    .saturating_mul(size_of::<Box<[NodeId]>>()),
            )
            .saturating_add(
                self.outgoing
                    .iter()
                    .chain(self.incoming.iter())
                    .map(|neighbors| neighbors.len().saturating_mul(size_of::<NodeId>()))
                    .sum::<usize>(),
            )
    }

    pub(in crate::retrieval) fn visible_candidates(
        &self,
        ranked: &[Uuid],
        scope: &QueryScope,
        limit: usize,
    ) -> Vec<NodeId> {
        let mut candidates = Vec::with_capacity(limit);
        let mut seen = BTreeSet::new();
        for concept_id in ranked {
            let Some(id) = self.node_id(*concept_id) else {
                continue;
            };
            if !self.is_visible(id, scope) || !seen.insert(id) {
                continue;
            }
            candidates.push(id);
            if candidates.len() == limit {
                break;
            }
        }
        candidates
    }

    fn expand_one_hop(
        &self,
        seeds: &[NodeId],
        scope: &QueryScope,
        direction: ExpansionDirection,
    ) -> ExpansionResult {
        let mut accumulator = ExpansionAccumulator::new();
        let mut visible_seeds = Vec::with_capacity(seeds.len().min(MAX_EXPANDED_CANDIDATES));
        for seed in seeds {
            if self.is_visible(*seed, scope) && !accumulator.candidates.contains(seed) {
                accumulator.candidates.push(*seed);
                visible_seeds.push(*seed);
                if accumulator.candidates.len() == MAX_EXPANDED_CANDIDATES {
                    accumulator.candidate_budget_exhausted = true;
                    return ExpansionResult {
                        candidates: accumulator.candidates,
                        edges_scanned: 0,
                        edge_budget_exhausted: false,
                        candidate_budget_exhausted: true,
                    };
                }
            }
        }
        append_neighbors(
            self,
            &visible_seeds,
            &self.outgoing,
            scope,
            &mut accumulator,
        );
        if direction == ExpansionDirection::Bidirectional
            && !accumulator.edge_budget_exhausted
            && !accumulator.candidate_budget_exhausted
        {
            append_neighbors(
                self,
                &visible_seeds,
                &self.incoming,
                scope,
                &mut accumulator,
            );
        }
        ExpansionResult {
            candidates: accumulator.candidates,
            edges_scanned: u32::try_from(accumulator.edges_scanned).unwrap_or(u32::MAX),
            edge_budget_exhausted: accumulator.edge_budget_exhausted,
            candidate_budget_exhausted: accumulator.candidate_budget_exhausted,
        }
    }

    pub(in crate::retrieval) fn expand_one_hop_with_backfill(
        &self,
        seeds: &[NodeId],
        ranked: &[Uuid],
        scope: &QueryScope,
        direction: ExpansionDirection,
    ) -> ExpansionResult {
        let mut result = self.expand_one_hop(seeds, scope, direction);
        if result.candidates.len() >= MAX_EXPANDED_CANDIDATES {
            return result;
        }
        for concept_id in ranked {
            let Some(id) = self.node_id(*concept_id) else {
                continue;
            };
            if !self.is_visible(id, scope) || result.candidates.contains(&id) {
                continue;
            }
            result.candidates.push(id);
            if result.candidates.len() == MAX_EXPANDED_CANDIDATES {
                result.candidate_budget_exhausted = true;
                break;
            }
        }
        result
    }

    fn is_visible(&self, id: NodeId, scope: &QueryScope) -> bool {
        self.node(id).is_some_and(|node| {
            scope.authorized_collections.contains(&node.collection_id)
                && (scope.purpose == GraphPurpose::LocalAssistant
                    || scope.external_ai_collections.contains(&node.collection_id))
        })
    }
}

struct ExpansionAccumulator {
    candidates: Vec<NodeId>,
    edges_scanned: usize,
    edge_budget_exhausted: bool,
    candidate_budget_exhausted: bool,
}

impl ExpansionAccumulator {
    fn new() -> Self {
        Self {
            candidates: Vec::with_capacity(MAX_EXPANDED_CANDIDATES),
            edges_scanned: 0,
            edge_budget_exhausted: false,
            candidate_budget_exhausted: false,
        }
    }
}

fn append_neighbors(
    graph: &MiniGraph,
    seeds: &[NodeId],
    adjacency: &[Box<[NodeId]>],
    scope: &QueryScope,
    accumulator: &mut ExpansionAccumulator,
) {
    for seed in seeds {
        let Ok(index) = node_index(*seed) else {
            continue;
        };
        let neighbors = adjacency.get(index).map_or(&[][..], AsRef::as_ref);
        for neighbor in neighbors {
            if accumulator.edges_scanned == MAX_SCANNED_EDGES {
                accumulator.edge_budget_exhausted = true;
                return;
            }
            accumulator.edges_scanned = accumulator.edges_scanned.saturating_add(1);
            if graph.is_visible(*neighbor, scope) && !accumulator.candidates.contains(neighbor) {
                accumulator.candidates.push(*neighbor);
                if accumulator.candidates.len() == MAX_EXPANDED_CANDIDATES {
                    accumulator.candidate_budget_exhausted = true;
                    return;
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::retrieval) struct ExpansionResult {
    pub(in crate::retrieval) candidates: Vec<NodeId>,
    pub(in crate::retrieval) edges_scanned: u32,
    pub(in crate::retrieval) edge_budget_exhausted: bool,
    pub(in crate::retrieval) candidate_budget_exhausted: bool,
}

fn boxed_adjacency(values: Vec<Vec<NodeId>>) -> Box<[Box<[NodeId]>]> {
    values
        .into_iter()
        .map(Vec::into_boxed_slice)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn node_id(nodes: &[GraphNode], concept_id: Uuid) -> Option<NodeId> {
    nodes
        .binary_search_by_key(&concept_id, |node| node.concept_id)
        .ok()
        .and_then(|index| u32::try_from(index).ok())
        .map(NodeId)
}

fn node_index(id: NodeId) -> Result<usize> {
    usize::try_from(id.0).context("mini graph node identity does not fit this platform")
}

fn graph_fingerprint(nodes: &[GraphNode], outgoing: &[Box<[NodeId]>]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(GRAPH_CONTRACT_VERSION.as_bytes());
    for (index, node) in nodes.iter().enumerate() {
        hasher.update(node.concept_id.as_bytes());
        hasher.update(node.collection_id.as_bytes());
        if let Some(neighbors) = outgoing.get(index) {
            for neighbor in neighbors.as_ref() {
                hasher.update(neighbor.0.to_le_bytes());
            }
        }
        hasher.update([0xff]);
    }
    hex::encode(hasher.finalize())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::retrieval) enum ExpansionDirection {
    Outgoing,
    Bidirectional,
}

#[derive(Debug)]
pub(in crate::retrieval) struct QueryScope {
    pub(in crate::retrieval) purpose: GraphPurpose,
    pub(in crate::retrieval) authorized_collections: BTreeSet<Uuid>,
    pub(in crate::retrieval) external_ai_collections: BTreeSet<Uuid>,
}

#[derive(Debug)]
struct MaterializedCase {
    graph: MiniGraph,
    labels_by_node: BTreeMap<NodeId, String>,
    ranked: Vec<Uuid>,
    required_groups: Vec<Vec<Uuid>>,
    forbidden: BTreeSet<Uuid>,
    scope: QueryScope,
}

#[derive(Debug, Clone, Serialize)]
struct ArmCaseReport {
    candidate_count: u32,
    added_candidate_count: u32,
    found_group_count: u32,
    required_group_count: u32,
    recall: Option<f64>,
    support_candidate_count: u32,
    support_density: Option<f64>,
    forbidden_candidate_count: u32,
    candidate_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct GraphCaseReport {
    id: String,
    purpose: GraphPurpose,
    graph_fingerprint: String,
    build_stats: GraphBuildStats,
    c10: ArmCaseReport,
    c32: ArmCaseReport,
    g1_out: ArmCaseReport,
    g1_bidir: ArmCaseReport,
    unique_outgoing_group_count_over_c32: u32,
    unique_bidir_group_count_over_c32: u32,
    unique_bidir_group_count_over_outgoing: u32,
    control_group_count_over_bidir: u32,
    expectations_met: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
struct ArmAggregate {
    candidate_count: u32,
    support_candidate_count: u32,
    forbidden_candidate_count: u32,
    required_group_count: u32,
    found_group_count: u32,
    recall: Option<f64>,
    support_density: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
struct AggregateReport {
    c10: ArmAggregate,
    c32: ArmAggregate,
    g1_out: ArmAggregate,
    g1_bidir: ArmAggregate,
    unique_outgoing_group_count_over_c32: u32,
    unique_bidir_group_count_over_c32: u32,
    unique_bidir_group_count_over_outgoing: u32,
    control_group_count_over_bidir: u32,
    expectation_failure_count: u32,
}

#[derive(Debug, Serialize)]
struct ResourceReport {
    scale_node_count: u32,
    scale_edge_count: u32,
    scale_retained_payload_bytes: usize,
    build_p95_micros: Option<u128>,
    expansion_p95_micros: Option<u128>,
    expansion_iterations: u32,
    timing_gate_evaluated: bool,
    build_budget_micros: u128,
    expansion_budget_micros: u128,
    retained_payload_budget_bytes: usize,
}

#[derive(Debug, Serialize)]
struct EvaluationReport {
    schema_version: u32,
    experiment_id: String,
    graph_contract_version: &'static str,
    fixture_sha256: String,
    build_profile: &'static str,
    baseline_candidate_limit: usize,
    control_candidate_limit: usize,
    graph_candidate_limit: usize,
    cases: Vec<GraphCaseReport>,
    aggregate: AggregateReport,
    resources: ResourceReport,
    structural_gate_passed: bool,
    synthetic_gate_passed: bool,
    production_promotion_ready: bool,
    rejection_reasons: Vec<String>,
}

pub(crate) fn evaluate_mini_graph() -> Result<()> {
    let loaded = load_fixture()?;
    let report = evaluate_fixture(&loaded.fixture, loaded.sha256)?;
    let destination = write_report(&report)?;
    if report.structural_gate_passed && !report.resources.timing_gate_evaluated {
        anyhow::bail!(
            "mini-graph structural gate passed, but timing requires a release build; rerun with `cargo run --release --locked -p xtask -- retrieval evaluate-mini-graph` (report written to {})",
            destination.display()
        );
    }
    ensure!(
        report.synthetic_gate_passed,
        "mini-graph did not meet the synthetic development gate; report written to {}",
        destination.display()
    );
    println!(
        "mini-graph passed its synthetic development gate; report written to {} (production promotion remains disabled)",
        destination.display()
    );
    Ok(())
}

#[derive(Debug)]
struct LoadedFixture {
    fixture: GraphFixture,
    sha256: String,
}

fn load_fixture() -> Result<LoadedFixture> {
    let path = workspace_root().join(FIXTURE_PATH);
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let fixture = serde_json::from_slice::<GraphFixture>(&bytes)
        .with_context(|| format!("parsing {}", path.display()))?;
    validate_fixture(&fixture)?;
    Ok(LoadedFixture {
        fixture,
        sha256: hex::encode(Sha256::digest(&bytes)),
    })
}

fn validate_fixture(fixture: &GraphFixture) -> Result<()> {
    ensure!(
        fixture.schema_version == FIXTURE_SCHEMA_VERSION,
        "unsupported mini-graph fixture schema"
    );
    validate_identifier(&fixture.experiment_id, "experiment")?;
    ensure!(!fixture.cases.is_empty(), "mini-graph fixture has no cases");
    let mut case_ids = BTreeSet::new();
    for case in &fixture.cases {
        validate_identifier(&case.id, "case")?;
        ensure!(
            case_ids.insert(case.id.as_str()),
            "duplicate mini-graph case id"
        );
        validate_identifier(&case.default_collection_id, "collection")?;
        ensure!(
            !case.authorized_collection_ids.is_empty(),
            "mini-graph case has no authorized collection"
        );
        ensure!(
            case.hybrid_ranked_concept_ids.len() >= BASELINE_LIMIT,
            "mini-graph case must supply at least {BASELINE_LIMIT} ranked concepts"
        );
        ensure_unique_identifiers(&case.hybrid_ranked_concept_ids, "hybrid-ranked concept")?;
        ensure_unique_identifiers(&case.authorized_collection_ids, "authorized collection")?;
        for collection in &case.authorized_collection_ids {
            validate_identifier(collection, "collection")?;
        }
        match (case.purpose, &case.external_ai_collection_ids) {
            (GraphPurpose::LocalAssistant, None) => {}
            (GraphPurpose::ExternalAi, Some(collections)) => {
                ensure_unique_identifiers(collections, "external-AI collection")?;
                for collection in collections {
                    ensure!(
                        case.authorized_collection_ids.contains(collection),
                        "external-AI collection must also be authorized"
                    );
                }
            }
            (GraphPurpose::LocalAssistant, Some(_)) => {
                anyhow::bail!("local-assistant case cannot declare external-AI collections");
            }
            (GraphPurpose::ExternalAi, None) => {
                anyhow::bail!("external-AI case must declare external-AI collections");
            }
        }
        let mut override_ids = BTreeSet::new();
        for node in &case.node_overrides {
            validate_identifier(&node.id, "concept")?;
            ensure!(
                override_ids.insert(node.id.as_str()),
                "duplicate mini-graph node override"
            );
            if let Some(collection) = &node.collection_id {
                validate_identifier(collection, "collection")?;
            }
        }
        for link in &case.links {
            validate_identifier(&link.source, "concept")?;
            validate_identifier(&link.target, "concept")?;
        }
        let universe = case_node_universe(case);
        let reference_universe = case_reference_universe(case);
        ensure!(
            universe.len() <= MAX_GRAPH_NODES,
            "mini-graph fixture case exceeds the node budget"
        );
        let mut relevant = BTreeSet::new();
        for group in &case.required_groups {
            ensure!(!group.is_empty(), "mini-graph required group is empty");
            ensure_unique_identifiers(group, "required concept")?;
            for id in group {
                ensure!(
                    universe.contains(id.as_str()),
                    "mini-graph required concept is missing from the case universe"
                );
                relevant.insert(id.as_str());
            }
        }
        ensure_unique_identifiers(&case.forbidden_concept_ids, "forbidden concept")?;
        for id in &case.forbidden_concept_ids {
            ensure!(
                reference_universe.contains(id.as_str()),
                "mini-graph forbidden concept is missing from the case references"
            );
            ensure!(
                !relevant.contains(id.as_str()),
                "mini-graph concept cannot be both required and forbidden"
            );
        }
        let expected = case.expected_found_groups;
        let total = u32::try_from(case.required_groups.len()).unwrap_or(u32::MAX);
        ensure!(
            [
                expected.c10,
                expected.c32,
                expected.g1_out,
                expected.g1_bidir
            ]
            .into_iter()
            .all(|count| count <= total),
            "mini-graph expected coverage exceeds required groups"
        );
    }
    Ok(())
}

fn validate_identifier(value: &str, kind: &str) -> Result<()> {
    ensure!(!value.is_empty(), "mini-graph {kind} identifier is empty");
    ensure!(
        value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        }),
        "mini-graph {kind} identifier must use lowercase ASCII, digits, `_` or `-`"
    );
    Ok(())
}

fn ensure_unique_identifiers(values: &[String], kind: &str) -> Result<()> {
    let unique = values.iter().map(String::as_str).collect::<BTreeSet<_>>();
    ensure!(
        unique.len() == values.len(),
        "mini-graph {kind} identifiers contain duplicates"
    );
    for value in values {
        validate_identifier(value, kind)?;
    }
    Ok(())
}

fn case_node_universe(case: &GraphCase) -> BTreeSet<&str> {
    case.hybrid_ranked_concept_ids
        .iter()
        .map(String::as_str)
        .chain(case.node_overrides.iter().map(|node| node.id.as_str()))
        .chain(
            case.links
                .iter()
                .filter(|link| link.disposition == LinkDisposition::ReviewedInternal)
                .flat_map(|link| [link.source.as_str(), link.target.as_str()]),
        )
        .collect()
}

fn case_reference_universe(case: &GraphCase) -> BTreeSet<&str> {
    case.hybrid_ranked_concept_ids
        .iter()
        .map(String::as_str)
        .chain(case.node_overrides.iter().map(|node| node.id.as_str()))
        .chain(
            case.links
                .iter()
                .flat_map(|link| [link.source.as_str(), link.target.as_str()]),
        )
        .collect()
}

fn evaluate_fixture(fixture: &GraphFixture, fixture_sha256: String) -> Result<EvaluationReport> {
    let mut case_reports = Vec::with_capacity(fixture.cases.len());
    for case in &fixture.cases {
        case_reports.push(evaluate_case(case)?);
    }
    let aggregate = aggregate_reports(&case_reports);
    let resources = resource_report()?;
    let mut rejection_reasons = Vec::new();
    if aggregate.unique_outgoing_group_count_over_c32 < 2 {
        rejection_reasons.push(
            "outgoing graph expansion added fewer than two required groups beyond C32".to_owned(),
        );
    }
    if aggregate.control_group_count_over_bidir == 0 {
        rejection_reasons
            .push("the wider-pool control recovered no graph-independent group".to_owned());
    }
    if aggregate.g1_out.found_group_count <= aggregate.c32.found_group_count {
        rejection_reasons.push("outgoing graph recall did not exceed C32".to_owned());
    }
    if aggregate.g1_bidir.found_group_count <= aggregate.g1_out.found_group_count {
        rejection_reasons.push("backlinks added no required group over outgoing links".to_owned());
    }
    if aggregate
        .g1_out
        .support_density
        .zip(aggregate.c32.support_density)
        .is_none_or(|(graph, control)| graph < control)
    {
        rejection_reasons.push("outgoing graph candidate density was lower than C32".to_owned());
    }
    if aggregate.c10.forbidden_candidate_count > 0
        || aggregate.c32.forbidden_candidate_count > 0
        || aggregate.g1_out.forbidden_candidate_count > 0
        || aggregate.g1_bidir.forbidden_candidate_count > 0
    {
        rejection_reasons
            .push("one or more candidate arms nominated forbidden concepts".to_owned());
    }
    if aggregate.expectation_failure_count > 0 {
        rejection_reasons.push("one or more frozen case expectations failed".to_owned());
    }
    if resources.scale_retained_payload_bytes > MAX_RETAINED_PAYLOAD_BYTES {
        rejection_reasons.push("retained graph payload exceeded the memory budget".to_owned());
    }
    let structural_gate_passed = rejection_reasons.is_empty();
    if resources.timing_gate_evaluated {
        if resources
            .build_p95_micros
            .is_none_or(|value| value > MAX_BUILD_MICROS_P95)
        {
            rejection_reasons.push("graph build p95 exceeded the development budget".to_owned());
        }
        if resources
            .expansion_p95_micros
            .is_none_or(|value| value > MAX_EXPANSION_MICROS_P95)
        {
            rejection_reasons
                .push("graph expansion p95 exceeded the development budget".to_owned());
        }
    } else {
        rejection_reasons.push("release timing gate was not evaluated".to_owned());
    }
    let synthetic_gate_passed = rejection_reasons.is_empty();
    Ok(EvaluationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        experiment_id: fixture.experiment_id.clone(),
        graph_contract_version: GRAPH_CONTRACT_VERSION,
        fixture_sha256,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        baseline_candidate_limit: BASELINE_LIMIT,
        control_candidate_limit: CONTROL_LIMIT,
        graph_candidate_limit: MAX_EXPANDED_CANDIDATES,
        cases: case_reports,
        aggregate,
        resources,
        structural_gate_passed,
        synthetic_gate_passed,
        production_promotion_ready: false,
        rejection_reasons,
    })
}

fn evaluate_case(case: &GraphCase) -> Result<GraphCaseReport> {
    let materialized_inputs = materialize_inputs(case)?;
    let graph = MiniGraph::build(&materialized_inputs.nodes, &materialized_inputs.links)?;
    let materialized = materialize_case(case, graph, &materialized_inputs.labels)?;

    let c10_ids = materialized.graph.visible_candidates(
        &materialized.ranked,
        &materialized.scope,
        BASELINE_LIMIT,
    );
    let c32_ids = materialized.graph.visible_candidates(
        &materialized.ranked,
        &materialized.scope,
        CONTROL_LIMIT,
    );
    let g1_out = materialized.graph.expand_one_hop_with_backfill(
        &c10_ids,
        &materialized.ranked,
        &materialized.scope,
        ExpansionDirection::Outgoing,
    );
    let g1_bidir = materialized.graph.expand_one_hop_with_backfill(
        &c10_ids,
        &materialized.ranked,
        &materialized.scope,
        ExpansionDirection::Bidirectional,
    );
    let g1_out_ids = &g1_out.candidates;
    let g1_bidir_ids = &g1_bidir.candidates;

    let c10 = score_arm(&materialized, &c10_ids, c10_ids.len());
    let c32 = score_arm(&materialized, &c32_ids, c10_ids.len());
    let g1_out_report = score_arm(&materialized, g1_out_ids, c10_ids.len());
    let g1_bidir_report = score_arm(&materialized, g1_bidir_ids, c10_ids.len());
    let unique_outgoing_group_count_over_c32 =
        unique_group_count(&materialized, &c32_ids, g1_out_ids);
    let unique_bidir_group_count_over_c32 =
        unique_group_count(&materialized, &c32_ids, g1_bidir_ids);
    let unique_bidir_group_count_over_outgoing =
        unique_group_count(&materialized, g1_out_ids, g1_bidir_ids);
    let control_group_count_over_bidir = unique_group_count(&materialized, g1_bidir_ids, &c32_ids);
    let expected = case.expected_found_groups;
    let expectations_met = c10.found_group_count == expected.c10
        && c32.found_group_count == expected.c32
        && g1_out_report.found_group_count == expected.g1_out
        && g1_bidir_report.found_group_count == expected.g1_bidir
        && c10.forbidden_candidate_count == 0
        && c32.forbidden_candidate_count == 0
        && g1_out_report.forbidden_candidate_count == 0
        && g1_bidir_report.forbidden_candidate_count == 0
        && !g1_out.edge_budget_exhausted
        && !g1_bidir.edge_budget_exhausted;

    Ok(GraphCaseReport {
        id: case.id.clone(),
        purpose: case.purpose,
        graph_fingerprint: materialized.graph.fingerprint.clone(),
        build_stats: materialized.graph.stats,
        c10,
        c32,
        g1_out: g1_out_report,
        g1_bidir: g1_bidir_report,
        unique_outgoing_group_count_over_c32,
        unique_bidir_group_count_over_c32,
        unique_bidir_group_count_over_outgoing,
        control_group_count_over_bidir,
        expectations_met,
    })
}

#[derive(Debug)]
struct MaterializedInputs {
    nodes: Vec<GraphNodeInput>,
    links: Vec<GraphLinkInput>,
    labels: BTreeMap<Uuid, String>,
}

fn materialize_inputs(case: &GraphCase) -> Result<MaterializedInputs> {
    let universe = case_node_universe(case);
    let overrides = case
        .node_overrides
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = Vec::with_capacity(universe.len());
    let mut labels = BTreeMap::new();
    for label in universe {
        let node_override = overrides.get(label).copied();
        let collection_label = node_override
            .and_then(|node| node.collection_id.as_deref())
            .unwrap_or(&case.default_collection_id);
        let concept_id = concept_uuid(&case.id, label);
        nodes.push(GraphNodeInput {
            node: GraphNode {
                concept_id,
                collection_id: collection_uuid(&case.id, collection_label),
            },
            state: node_override.map_or(NodeState::Current, |node| node.state),
        });
        labels.insert(concept_id, label.to_owned());
    }
    let links = case
        .links
        .iter()
        .map(|link| GraphLinkInput {
            source: concept_uuid(&case.id, &link.source),
            target: concept_uuid(&case.id, &link.target),
            disposition: link.disposition,
        })
        .collect();
    Ok(MaterializedInputs {
        nodes,
        links,
        labels,
    })
}

fn materialize_case(
    case: &GraphCase,
    graph: MiniGraph,
    labels: &BTreeMap<Uuid, String>,
) -> Result<MaterializedCase> {
    let mut labels_by_node = BTreeMap::new();
    for node in graph.nodes.iter() {
        let id = graph
            .node_id(node.concept_id)
            .context("materialized mini-graph node is missing")?;
        let label = labels
            .get(&node.concept_id)
            .context("materialized mini-graph label is missing")?;
        labels_by_node.insert(id, label.clone());
    }
    Ok(MaterializedCase {
        graph,
        labels_by_node,
        ranked: case
            .hybrid_ranked_concept_ids
            .iter()
            .map(|id| concept_uuid(&case.id, id))
            .collect(),
        required_groups: case
            .required_groups
            .iter()
            .map(|group| group.iter().map(|id| concept_uuid(&case.id, id)).collect())
            .collect(),
        forbidden: case
            .forbidden_concept_ids
            .iter()
            .map(|id| concept_uuid(&case.id, id))
            .collect(),
        scope: QueryScope {
            purpose: case.purpose,
            authorized_collections: case
                .authorized_collection_ids
                .iter()
                .map(|id| collection_uuid(&case.id, id))
                .collect(),
            external_ai_collections: case
                .external_ai_collection_ids
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|id| collection_uuid(&case.id, id))
                .collect(),
        },
    })
}

fn concept_uuid(case_id: &str, concept_id: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("urn:airwiki:mini-graph:{case_id}:concept:{concept_id}").as_bytes(),
    )
}

fn collection_uuid(case_id: &str, collection_id: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("urn:airwiki:mini-graph:{case_id}:collection:{collection_id}").as_bytes(),
    )
}

fn score_arm(
    case: &MaterializedCase,
    candidates: &[NodeId],
    baseline_count: usize,
) -> ArmCaseReport {
    let candidate_concepts = candidate_concepts(case, candidates);
    let relevant = case
        .required_groups
        .iter()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
    let found_group_count = case
        .required_groups
        .iter()
        .filter(|group| group.iter().any(|id| candidate_concepts.contains(id)))
        .count();
    let support_candidate_count = candidate_concepts.intersection(&relevant).count();
    let forbidden_candidate_count = candidate_concepts.intersection(&case.forbidden).count();
    let candidate_count = candidates.len();
    ArmCaseReport {
        candidate_count: u32::try_from(candidate_count).unwrap_or(u32::MAX),
        added_candidate_count: u32::try_from(candidate_count.saturating_sub(baseline_count))
            .unwrap_or(u32::MAX),
        found_group_count: u32::try_from(found_group_count).unwrap_or(u32::MAX),
        required_group_count: u32::try_from(case.required_groups.len()).unwrap_or(u32::MAX),
        recall: (!case.required_groups.is_empty())
            .then(|| found_group_count as f64 / case.required_groups.len() as f64),
        support_candidate_count: u32::try_from(support_candidate_count).unwrap_or(u32::MAX),
        support_density: (candidate_count > 0)
            .then(|| support_candidate_count as f64 / candidate_count as f64),
        forbidden_candidate_count: u32::try_from(forbidden_candidate_count).unwrap_or(u32::MAX),
        candidate_ids: candidates
            .iter()
            .filter_map(|id| case.labels_by_node.get(id).cloned())
            .collect(),
    }
}

fn candidate_concepts(case: &MaterializedCase, candidates: &[NodeId]) -> BTreeSet<Uuid> {
    candidates
        .iter()
        .filter_map(|id| case.graph.node(*id).map(|node| node.concept_id))
        .collect()
}

fn unique_group_count(case: &MaterializedCase, control: &[NodeId], graph: &[NodeId]) -> u32 {
    let control = candidate_concepts(case, control);
    let graph = candidate_concepts(case, graph);
    let unique = case
        .required_groups
        .iter()
        .filter(|group| {
            !group.iter().any(|id| control.contains(id))
                && group.iter().any(|id| graph.contains(id))
        })
        .count();
    u32::try_from(unique).unwrap_or(u32::MAX)
}

fn aggregate_reports(reports: &[GraphCaseReport]) -> AggregateReport {
    let mut aggregate = AggregateReport::default();
    for report in reports {
        add_arm(&mut aggregate.c10, &report.c10);
        add_arm(&mut aggregate.c32, &report.c32);
        add_arm(&mut aggregate.g1_out, &report.g1_out);
        add_arm(&mut aggregate.g1_bidir, &report.g1_bidir);
        aggregate.unique_outgoing_group_count_over_c32 = aggregate
            .unique_outgoing_group_count_over_c32
            .saturating_add(report.unique_outgoing_group_count_over_c32);
        aggregate.unique_bidir_group_count_over_c32 = aggregate
            .unique_bidir_group_count_over_c32
            .saturating_add(report.unique_bidir_group_count_over_c32);
        aggregate.unique_bidir_group_count_over_outgoing = aggregate
            .unique_bidir_group_count_over_outgoing
            .saturating_add(report.unique_bidir_group_count_over_outgoing);
        aggregate.control_group_count_over_bidir = aggregate
            .control_group_count_over_bidir
            .saturating_add(report.control_group_count_over_bidir);
        if !report.expectations_met {
            aggregate.expectation_failure_count =
                aggregate.expectation_failure_count.saturating_add(1);
        }
    }
    finalize_arm(&mut aggregate.c10);
    finalize_arm(&mut aggregate.c32);
    finalize_arm(&mut aggregate.g1_out);
    finalize_arm(&mut aggregate.g1_bidir);
    aggregate
}

fn add_arm(aggregate: &mut ArmAggregate, case: &ArmCaseReport) {
    aggregate.candidate_count = aggregate
        .candidate_count
        .saturating_add(case.candidate_count);
    aggregate.support_candidate_count = aggregate
        .support_candidate_count
        .saturating_add(case.support_candidate_count);
    aggregate.forbidden_candidate_count = aggregate
        .forbidden_candidate_count
        .saturating_add(case.forbidden_candidate_count);
    aggregate.required_group_count = aggregate
        .required_group_count
        .saturating_add(case.required_group_count);
    aggregate.found_group_count = aggregate
        .found_group_count
        .saturating_add(case.found_group_count);
}

fn finalize_arm(aggregate: &mut ArmAggregate) {
    aggregate.recall = (aggregate.required_group_count > 0).then(|| {
        f64::from(aggregate.found_group_count) / f64::from(aggregate.required_group_count)
    });
    aggregate.support_density = (aggregate.candidate_count > 0).then(|| {
        f64::from(aggregate.support_candidate_count) / f64::from(aggregate.candidate_count)
    });
}

fn resource_report() -> Result<ResourceReport> {
    const BUILD_ITERATIONS: usize = 25;
    const EXPANSION_WARMUP: usize = 50;
    const EXPANSION_ITERATIONS: usize = 1_000;
    let (nodes, links) = scale_inputs();
    let mut build_micros = Vec::with_capacity(BUILD_ITERATIONS);
    let mut graph = None;
    for _ in 0..BUILD_ITERATIONS {
        let started = Instant::now();
        let built = MiniGraph::build(black_box(&nodes), black_box(&links))?;
        build_micros.push(started.elapsed().as_micros());
        graph = Some(black_box(built));
    }
    build_micros.sort_unstable();
    let graph = graph.context("scale profile did not build a mini graph")?;
    let scope = QueryScope {
        purpose: GraphPurpose::LocalAssistant,
        authorized_collections: [Uuid::from_u128(1)].into_iter().collect(),
        external_ai_collections: BTreeSet::new(),
    };
    let seeds = (0..BASELINE_LIMIT)
        .filter_map(|index| graph.node_id(Uuid::from_u128(index as u128 + 1)))
        .collect::<Vec<_>>();
    let ranked = (0..MAX_GRAPH_NODES)
        .map(|index| Uuid::from_u128(index as u128 + 1))
        .collect::<Vec<_>>();
    for _ in 0..EXPANSION_WARMUP {
        black_box(graph.expand_one_hop_with_backfill(
            black_box(&seeds),
            black_box(&ranked),
            black_box(&scope),
            ExpansionDirection::Bidirectional,
        ));
    }
    let mut expansion_micros = Vec::with_capacity(EXPANSION_ITERATIONS);
    for _ in 0..EXPANSION_ITERATIONS {
        let started = Instant::now();
        black_box(graph.expand_one_hop_with_backfill(
            black_box(&seeds),
            black_box(&ranked),
            black_box(&scope),
            ExpansionDirection::Bidirectional,
        ));
        expansion_micros.push(started.elapsed().as_micros());
    }
    expansion_micros.sort_unstable();
    Ok(ResourceReport {
        scale_node_count: graph.stats.node_count,
        scale_edge_count: graph.stats.edge_count,
        scale_retained_payload_bytes: graph.retained_payload_bytes(),
        build_p95_micros: percentile(&build_micros, 95),
        expansion_p95_micros: percentile(&expansion_micros, 95),
        expansion_iterations: u32::try_from(EXPANSION_ITERATIONS).unwrap_or(u32::MAX),
        timing_gate_evaluated: !cfg!(debug_assertions),
        build_budget_micros: MAX_BUILD_MICROS_P95,
        expansion_budget_micros: MAX_EXPANSION_MICROS_P95,
        retained_payload_budget_bytes: MAX_RETAINED_PAYLOAD_BYTES,
    })
}

fn scale_inputs() -> (Vec<GraphNodeInput>, Vec<GraphLinkInput>) {
    let nodes = (0..MAX_GRAPH_NODES)
        .map(|index| GraphNodeInput {
            node: GraphNode {
                concept_id: Uuid::from_u128(index as u128 + 1),
                collection_id: Uuid::from_u128(1),
            },
            state: NodeState::Current,
        })
        .collect::<Vec<_>>();
    let links = (0..MAX_GRAPH_NODES)
        .flat_map(|source| {
            (1..=4).map(move |offset| GraphLinkInput {
                source: Uuid::from_u128(source as u128 + 1),
                target: Uuid::from_u128(((source + offset) % MAX_GRAPH_NODES) as u128 + 1),
                disposition: LinkDisposition::ReviewedInternal,
            })
        })
        .collect::<Vec<_>>();
    (nodes, links)
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

fn report_path() -> PathBuf {
    workspace_root().join(REPORT_DIRECTORY).join(format!(
        "retrieval-mini-graph-development-{}-{}.json",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}

fn write_report(report: &EvaluationReport) -> Result<PathBuf> {
    let destination = report_path();
    let parent = destination
        .parent()
        .context("mini-graph report has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let temporary = destination.with_extension("json.tmp");
    let mut contents = serde_json::to_string_pretty(report)?;
    contents.push('\n');
    std::fs::write(&temporary, contents)
        .with_context(|| format!("writing {}", temporary.display()))?;
    replace_file(&temporary, &destination)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_node(index: u32) -> GraphNodeInput {
        GraphNodeInput {
            node: GraphNode {
                concept_id: Uuid::from_u128(u128::from(index) + 1),
                collection_id: Uuid::from_u128(1),
            },
            state: NodeState::Current,
        }
    }

    fn reviewed_link(source: u32, target: u32) -> GraphLinkInput {
        GraphLinkInput {
            source: Uuid::from_u128(u128::from(source) + 1),
            target: Uuid::from_u128(u128::from(target) + 1),
            disposition: LinkDisposition::ReviewedInternal,
        }
    }

    #[test]
    fn fixture_meets_the_frozen_mechanistic_gate() {
        let loaded = load_fixture().unwrap();

        let report = evaluate_fixture(&loaded.fixture, loaded.sha256).unwrap();

        assert!(
            report.structural_gate_passed,
            "mini-graph report: {report:#?}"
        );
        if cfg!(debug_assertions) {
            assert!(!report.synthetic_gate_passed);
            assert!(
                report
                    .rejection_reasons
                    .iter()
                    .any(|reason| reason.contains("release timing gate"))
            );
        }
        let lyra = report
            .cases
            .iter()
            .find(|case| case.id == "outgoing-rescue-lyra")
            .unwrap();
        assert_eq!(lyra.c32.candidate_count, lyra.g1_out.candidate_count);
        let orion = report
            .cases
            .iter()
            .find(|case| case.id == "external-ai-policy-orion")
            .unwrap();
        assert_eq!(orion.build_stats.edge_count, 1);
        assert_eq!(orion.build_stats.ignored_cross_collection_count, 0);
        assert_eq!(orion.c10.candidate_count, 0);
        let nova = report
            .cases
            .iter()
            .find(|case| case.id == "invalid-links-nova")
            .unwrap();
        assert_eq!(nova.build_stats.node_count, 10);
    }

    #[test]
    fn ground_truth_cannot_materialize_a_graph_node() {
        let mut loaded = load_fixture().unwrap();
        loaded.fixture.cases[0]
            .required_groups
            .push(vec!["expectation-only-concept".to_owned()]);

        let error = validate_fixture(&loaded.fixture).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("required concept is missing from the case universe")
        );
    }

    #[test]
    fn forbidden_candidates_veto_every_arm() {
        let mut loaded = load_fixture().unwrap();
        loaded.fixture.cases[0]
            .forbidden_concept_ids
            .push("lyra-seed".to_owned());
        validate_fixture(&loaded.fixture).unwrap();

        let report = evaluate_fixture(&loaded.fixture, loaded.sha256).unwrap();

        assert!(!report.structural_gate_passed);
        assert!(
            report
                .rejection_reasons
                .iter()
                .any(|reason| reason.contains("candidate arms nominated forbidden concepts"))
        );
    }

    #[test]
    fn graph_fingerprint_is_stable_when_inputs_are_reordered() {
        let nodes = (0..4).map(graph_node).collect::<Vec<_>>();
        let links = vec![reviewed_link(0, 1), reviewed_link(1, 2)];
        let mut reversed_nodes = nodes.clone();
        let mut reversed_links = links.clone();
        reversed_nodes.reverse();
        reversed_links.reverse();

        let baseline = MiniGraph::build(&nodes, &links).unwrap();
        let reversed = MiniGraph::build(&reversed_nodes, &reversed_links).unwrap();

        assert_eq!(baseline.fingerprint, reversed.fingerprint);
    }

    #[test]
    fn one_hop_expansion_does_not_walk_a_cycle_or_second_hop() {
        let nodes = (0..3).map(graph_node).collect::<Vec<_>>();
        let links = vec![
            reviewed_link(0, 1),
            reviewed_link(1, 0),
            reviewed_link(1, 2),
        ];
        let graph = MiniGraph::build(&nodes, &links).unwrap();
        let scope = QueryScope {
            purpose: GraphPurpose::LocalAssistant,
            authorized_collections: [Uuid::from_u128(1)].into_iter().collect(),
            external_ai_collections: BTreeSet::new(),
        };
        let seed = graph.node_id(Uuid::from_u128(1)).unwrap();
        let immediate_neighbor = graph.node_id(Uuid::from_u128(2)).unwrap();
        let second_hop = graph.node_id(Uuid::from_u128(3)).unwrap();

        let expanded = graph.expand_one_hop(&[seed], &scope, ExpansionDirection::Outgoing);

        assert_eq!(expanded.candidates, vec![seed, immediate_neighbor]);
        assert!(!expanded.candidates.contains(&second_hop));
    }

    #[test]
    fn seed_prefill_respects_the_candidate_budget() {
        let nodes = (0..40).map(graph_node).collect::<Vec<_>>();
        let graph = MiniGraph::build(&nodes, &[]).unwrap();
        let scope = QueryScope {
            purpose: GraphPurpose::LocalAssistant,
            authorized_collections: [Uuid::from_u128(1)].into_iter().collect(),
            external_ai_collections: BTreeSet::new(),
        };
        let seeds = nodes
            .iter()
            .filter_map(|input| graph.node_id(input.node.concept_id))
            .collect::<Vec<_>>();

        let expanded = graph.expand_one_hop(&seeds, &scope, ExpansionDirection::Outgoing);

        assert_eq!(expanded.candidates.len(), MAX_EXPANDED_CANDIDATES);
        assert!(expanded.candidate_budget_exhausted);
        assert_eq!(expanded.edges_scanned, 0);
    }

    #[test]
    fn withdrawn_and_non_internal_targets_never_enter_the_graph() {
        let mut nodes = vec![graph_node(0), graph_node(1)];
        nodes[1].state = NodeState::Withdrawn;
        let links = vec![
            reviewed_link(0, 1),
            GraphLinkInput {
                source: Uuid::from_u128(1),
                target: Uuid::from_u128(2),
                disposition: LinkDisposition::Unsafe,
            },
        ];

        let graph = MiniGraph::build(&nodes, &links).unwrap();

        assert_eq!(graph.stats.edge_count, 0);
    }

    #[test]
    fn cross_collection_self_and_duplicate_links_do_not_widen_the_graph() {
        let mut nodes = vec![graph_node(0), graph_node(1), graph_node(2)];
        nodes[2].node.collection_id = Uuid::from_u128(2);
        let links = vec![
            reviewed_link(0, 0),
            reviewed_link(0, 1),
            reviewed_link(0, 1),
            reviewed_link(0, 2),
        ];

        let graph = MiniGraph::build(&nodes, &links).unwrap();

        assert_eq!(
            (
                graph.stats.edge_count,
                graph.stats.duplicate_edge_count,
                graph.stats.ignored_self_link_count,
                graph.stats.ignored_cross_collection_count,
            ),
            (1, 1, 1, 1)
        );
    }

    #[test]
    fn external_ai_scope_filters_nodes_before_expansion() {
        let nodes = vec![graph_node(0), graph_node(1)];
        let graph = MiniGraph::build(&nodes, &[reviewed_link(0, 1)]).unwrap();
        let scope = QueryScope {
            purpose: GraphPurpose::ExternalAi,
            authorized_collections: [Uuid::from_u128(1)].into_iter().collect(),
            external_ai_collections: BTreeSet::new(),
        };
        let seed = graph.node_id(Uuid::from_u128(1)).unwrap();

        for direction in [
            ExpansionDirection::Outgoing,
            ExpansionDirection::Bidirectional,
        ] {
            let expanded = graph.expand_one_hop(&[seed], &scope, direction);
            assert!(expanded.candidates.is_empty());
            assert_eq!(expanded.edges_scanned, 0);
        }
    }

    #[test]
    fn edge_scan_budget_bounds_dense_visible_neighbors() {
        let nodes = (0..15).map(graph_node).collect::<Vec<_>>();
        let links = (0..15)
            .flat_map(|source| {
                (0..15)
                    .filter(move |target| *target != source)
                    .map(move |target| reviewed_link(source, target))
            })
            .collect::<Vec<_>>();
        let graph = MiniGraph::build(&nodes, &links).unwrap();
        let scope = QueryScope {
            purpose: GraphPurpose::LocalAssistant,
            authorized_collections: [Uuid::from_u128(1)].into_iter().collect(),
            external_ai_collections: BTreeSet::new(),
        };
        let seeds = (0..15)
            .filter_map(|index| graph.node_id(Uuid::from_u128(index + 1)))
            .collect::<Vec<_>>();

        let expanded = graph.expand_one_hop(&seeds, &scope, ExpansionDirection::Outgoing);

        assert!(expanded.edge_budget_exhausted);
        assert_eq!(expanded.edges_scanned, MAX_SCANNED_EDGES as u32);
        assert!(expanded.candidates.len() <= MAX_EXPANDED_CANDIDATES);
    }

    #[test]
    fn five_hundred_nodes_and_two_thousand_edges_fit_the_payload_budget() {
        let nodes = (0..500).map(graph_node).collect::<Vec<_>>();
        let links = (0..500)
            .flat_map(|source| {
                (1..=4).map(move |offset| reviewed_link(source, (source + offset) % 500))
            })
            .collect::<Vec<_>>();

        let graph = MiniGraph::build(&nodes, &links).unwrap();

        assert!(
            graph.retained_payload_bytes() < MAX_RETAINED_PAYLOAD_BYTES,
            "retained payload: {} bytes",
            graph.retained_payload_bytes()
        );
    }
}

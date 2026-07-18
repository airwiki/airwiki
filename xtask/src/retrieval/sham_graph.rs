//! Deterministic degree-preserving structural sham for graph ablations.
//!
//! The projection minimizes retained reviewed links per collection. It is not
//! a random or uniform sample from the directed degree-sequence ensemble.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};

use anyhow::{Context, Result, ensure};
use uuid::Uuid;

use super::mini_graph::{GraphLinkInput, GraphNodeInput, LinkDisposition, NodeState};

const MAX_FLOW_VERTICES: usize = 1_024;
const MAX_POSSIBLE_ARCS: usize = 250_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::retrieval) struct StructuralShamStats {
    pub(in crate::retrieval) collection_count: u32,
    pub(in crate::retrieval) retained_original_edge_count: u32,
    pub(in crate::retrieval) rewired_edge_count: u32,
    pub(in crate::retrieval) unchanged_collection_count: u32,
}

#[derive(Debug)]
pub(in crate::retrieval) struct StructuralSham {
    pub(in crate::retrieval) links: Vec<GraphLinkInput>,
    pub(in crate::retrieval) stats: StructuralShamStats,
}

#[derive(Debug, Clone)]
struct StableNode {
    collection_id: Uuid,
    label: String,
}

#[derive(Debug, Clone, Copy)]
struct ArcReference {
    source: Uuid,
    target: Uuid,
    flow_node: usize,
    flow_edge: usize,
}

pub(in crate::retrieval) fn build_structural_sham(
    nodes: &[GraphNodeInput],
    links: &[GraphLinkInput],
    stable_label_by_concept: &HashMap<Uuid, String>,
) -> Result<StructuralSham> {
    let stable_nodes = stable_nodes(nodes, stable_label_by_concept)?;
    let mut grouped = BTreeMap::<Uuid, Vec<GraphLinkInput>>::new();
    let mut original_pairs = BTreeSet::new();
    for link in links {
        ensure!(
            link.disposition == LinkDisposition::ReviewedInternal,
            "structural sham accepts only reviewed internal links"
        );
        ensure!(
            link.source != link.target,
            "structural sham rejects self links"
        );
        let source = stable_nodes
            .get(&link.source)
            .context("structural sham source is unavailable")?;
        let target = stable_nodes
            .get(&link.target)
            .context("structural sham target is unavailable")?;
        ensure!(
            source.collection_id == target.collection_id,
            "structural sham rejects cross-collection links"
        );
        ensure!(
            original_pairs.insert((link.source, link.target)),
            "structural sham rejects duplicate directed links"
        );
        grouped.entry(source.collection_id).or_default().push(*link);
    }

    let mut projected = Vec::with_capacity(links.len());
    let mut retained_original_edge_count = 0_usize;
    let mut unchanged_collection_count = 0_usize;
    for collection_links in grouped.values() {
        let collection_projection = solve_collection(collection_links, &stable_nodes)?;
        let original = collection_links
            .iter()
            .map(|link| (link.source, link.target))
            .collect::<BTreeSet<_>>();
        let replacement = collection_projection
            .iter()
            .map(|link| (link.source, link.target))
            .collect::<BTreeSet<_>>();
        retained_original_edge_count = retained_original_edge_count
            .saturating_add(original.intersection(&replacement).count());
        if original == replacement {
            unchanged_collection_count = unchanged_collection_count.saturating_add(1);
        }
        projected.extend(collection_projection);
    }
    sort_links(&mut projected, &stable_nodes)?;
    ensure!(
        projected.len() == links.len(),
        "structural sham changed the directed edge count"
    );

    Ok(StructuralSham {
        links: projected,
        stats: StructuralShamStats {
            collection_count: u32::try_from(grouped.len())
                .context("too many structural-sham collections")?,
            retained_original_edge_count: u32::try_from(retained_original_edge_count)
                .context("too many retained structural-sham links")?,
            rewired_edge_count: u32::try_from(
                links.len().saturating_sub(retained_original_edge_count),
            )
            .context("too many rewired structural-sham links")?,
            unchanged_collection_count: u32::try_from(unchanged_collection_count)
                .context("too many unchanged structural-sham collections")?,
        },
    })
}

fn stable_nodes(
    nodes: &[GraphNodeInput],
    stable_label_by_concept: &HashMap<Uuid, String>,
) -> Result<HashMap<Uuid, StableNode>> {
    let mut stable_nodes = HashMap::new();
    let mut labels = BTreeSet::new();
    for input in nodes {
        if input.state != NodeState::Current {
            continue;
        }
        let label = stable_label_by_concept
            .get(&input.node.concept_id)
            .filter(|label| !label.is_empty())
            .cloned()
            .context("structural sham node has no stable label")?;
        ensure!(
            labels.insert(label.clone()),
            "structural sham labels are not unique"
        );
        ensure!(
            stable_nodes
                .insert(
                    input.node.concept_id,
                    StableNode {
                        collection_id: input.node.collection_id,
                        label,
                    },
                )
                .is_none(),
            "structural sham contains a duplicate concept"
        );
    }
    Ok(stable_nodes)
}

fn solve_collection(
    original_links: &[GraphLinkInput],
    nodes: &HashMap<Uuid, StableNode>,
) -> Result<Vec<GraphLinkInput>> {
    let original = original_links
        .iter()
        .map(|link| (link.source, link.target))
        .collect::<BTreeSet<_>>();
    let mut outdegrees = BTreeMap::<Uuid, usize>::new();
    let mut indegrees = BTreeMap::<Uuid, usize>::new();
    for link in original_links {
        *outdegrees.entry(link.source).or_default() += 1;
        *indegrees.entry(link.target).or_default() += 1;
    }
    let sources = stable_sorted_ids(outdegrees.keys().copied(), nodes)?;
    let targets = stable_sorted_ids(indegrees.keys().copied(), nodes)?;

    let source_node = 0;
    let left_offset = 1;
    let right_offset = left_offset + sources.len();
    let sink_node = right_offset + targets.len();
    ensure!(
        sink_node.saturating_add(1) <= MAX_FLOW_VERTICES,
        "structural sham exceeds its flow-vertex budget"
    );
    let mut flow = MinCostFlow::new(sink_node + 1);
    for (index, concept_id) in sources.iter().enumerate() {
        flow.add_edge(
            source_node,
            left_offset + index,
            i32::try_from(
                *outdegrees
                    .get(concept_id)
                    .context("structural-sham source lost its outdegree")?,
            )
            .context("outdegree exceeds flow capacity")?,
            0,
        );
    }
    for (index, concept_id) in targets.iter().enumerate() {
        flow.add_edge(
            right_offset + index,
            sink_node,
            i32::try_from(
                *indegrees
                    .get(concept_id)
                    .context("structural-sham target lost its indegree")?,
            )
            .context("indegree exceeds flow capacity")?,
            0,
        );
    }

    let mut arcs = Vec::new();
    for (source_index, source) in sources.iter().enumerate() {
        for (target_index, target) in targets.iter().enumerate() {
            if source == target {
                continue;
            }
            ensure!(
                arcs.len() < MAX_POSSIBLE_ARCS,
                "structural sham exceeds its possible-arc budget"
            );
            let flow_node = left_offset + source_index;
            let flow_edge = flow.add_edge(
                flow_node,
                right_offset + target_index,
                1,
                if original.contains(&(*source, *target)) {
                    1
                } else {
                    0
                },
            );
            arcs.push(ArcReference {
                source: *source,
                target: *target,
                flow_node,
                flow_edge,
            });
        }
    }

    let required_flow = i32::try_from(original_links.len()).context("too many structural links")?;
    let (actual_flow, _) = flow.solve(source_node, sink_node, required_flow)?;
    ensure!(
        actual_flow == required_flow,
        "directed degree sequence has no simple collection-local realization"
    );
    let mut projected = arcs
        .into_iter()
        .filter(|arc| flow.edge_used(arc.flow_node, arc.flow_edge))
        .map(|arc| GraphLinkInput {
            source: arc.source,
            target: arc.target,
            disposition: LinkDisposition::ReviewedInternal,
        })
        .collect::<Vec<_>>();
    ensure!(
        projected.len() == original_links.len(),
        "structural sham materialized an incomplete realization"
    );
    sort_links(&mut projected, nodes)?;
    Ok(projected)
}

fn stable_sorted_ids(
    ids: impl IntoIterator<Item = Uuid>,
    nodes: &HashMap<Uuid, StableNode>,
) -> Result<Vec<Uuid>> {
    let mut decorated = ids
        .into_iter()
        .map(|id| {
            nodes
                .get(&id)
                .map(|node| (node.label.clone(), id))
                .context("structural-sham endpoint is unavailable")
        })
        .collect::<Result<Vec<_>>>()?;
    decorated.sort_unstable();
    Ok(decorated.into_iter().map(|(_, id)| id).collect())
}

fn sort_links(links: &mut Vec<GraphLinkInput>, nodes: &HashMap<Uuid, StableNode>) -> Result<()> {
    let mut decorated = links
        .iter()
        .copied()
        .map(|link| {
            let source = nodes
                .get(&link.source)
                .context("structural-sham source is unavailable while sorting")?;
            let target = nodes
                .get(&link.target)
                .context("structural-sham target is unavailable while sorting")?;
            Ok(((source.label.clone(), target.label.clone()), link))
        })
        .collect::<Result<Vec<_>>>()?;
    decorated.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    *links = decorated.into_iter().map(|(_, link)| link).collect();
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct FlowEdge {
    to: usize,
    reverse: usize,
    capacity: i32,
    cost: i64,
}

struct MinCostFlow {
    adjacency: Vec<Vec<FlowEdge>>,
}

impl MinCostFlow {
    fn new(vertex_count: usize) -> Self {
        Self {
            adjacency: vec![Vec::new(); vertex_count],
        }
    }

    fn add_edge(&mut self, from: usize, to: usize, capacity: i32, cost: i64) -> usize {
        let forward_index = self.adjacency[from].len();
        let reverse_index = self.adjacency[to].len();
        self.adjacency[from].push(FlowEdge {
            to,
            reverse: reverse_index,
            capacity,
            cost,
        });
        self.adjacency[to].push(FlowEdge {
            to: from,
            reverse: forward_index,
            capacity: 0,
            cost: -cost,
        });
        forward_index
    }

    fn edge_used(&self, from: usize, edge: usize) -> bool {
        self.adjacency
            .get(from)
            .and_then(|edges| edges.get(edge))
            .is_some_and(|edge| edge.capacity == 0)
    }

    fn solve(&mut self, source: usize, sink: usize, required: i32) -> Result<(i32, i64)> {
        let vertex_count = self.adjacency.len();
        let mut potential = vec![0_i64; vertex_count];
        let mut total_flow = 0_i32;
        let mut total_cost = 0_i64;
        while total_flow < required {
            let mut distance = vec![i64::MAX; vertex_count];
            let mut previous = vec![None::<(usize, usize)>; vertex_count];
            let mut queue = BinaryHeap::new();
            distance[source] = 0;
            queue.push((Reverse(0_i64), Reverse(source)));
            while let Some((Reverse(current_distance), Reverse(vertex))) = queue.pop() {
                if current_distance != distance[vertex] {
                    continue;
                }
                for (edge_index, edge) in self.adjacency[vertex].iter().enumerate() {
                    if edge.capacity <= 0 {
                        continue;
                    }
                    let reduced_cost = edge
                        .cost
                        .saturating_add(potential[vertex])
                        .saturating_sub(potential[edge.to]);
                    ensure!(
                        reduced_cost >= 0,
                        "structural sham flow lost non-negative reduced costs"
                    );
                    let candidate = current_distance.saturating_add(reduced_cost);
                    if candidate < distance[edge.to] {
                        distance[edge.to] = candidate;
                        previous[edge.to] = Some((vertex, edge_index));
                        queue.push((Reverse(candidate), Reverse(edge.to)));
                    }
                }
            }
            ensure!(
                distance[sink] != i64::MAX,
                "directed degree sequence cannot reach required flow"
            );
            for (vertex, value) in distance.iter().enumerate() {
                if *value != i64::MAX {
                    potential[vertex] = potential[vertex].saturating_add(*value);
                }
            }

            let mut augment = required.saturating_sub(total_flow);
            let mut vertex = sink;
            while vertex != source {
                let (parent, edge_index) =
                    previous[vertex].context("structural sham flow path is incomplete")?;
                augment = augment.min(self.adjacency[parent][edge_index].capacity);
                vertex = parent;
            }
            let mut path_cost = 0_i64;
            vertex = sink;
            while vertex != source {
                let (parent, edge_index) =
                    previous[vertex].context("structural sham flow path is incomplete")?;
                let edge = self.adjacency[parent][edge_index];
                path_cost = path_cost.saturating_add(edge.cost);
                self.adjacency[parent][edge_index].capacity -= augment;
                self.adjacency[vertex][edge.reverse].capacity += augment;
                vertex = parent;
            }
            total_flow = total_flow.saturating_add(augment);
            total_cost = total_cost.saturating_add(path_cost.saturating_mul(i64::from(augment)));
        }
        Ok((total_flow, total_cost))
    }
}

#[cfg(test)]
mod tests {
    use super::super::mini_graph::GraphNode;
    use super::*;

    #[test]
    fn unique_realization_retains_unavoidable_edges() {
        let collection = Uuid::from_u128(100);
        let ids = ids(3);
        let nodes = nodes(collection, &ids);
        let links = links(&[(ids[0], ids[1]), (ids[2], ids[0])]);

        let sham = build_structural_sham(&nodes, &links, &labels(&ids)).unwrap();

        assert_eq!(pairs(&sham.links), pairs(&links));
        assert_eq!(sham.stats.retained_original_edge_count, 2);
        assert_eq!(sham.stats.rewired_edge_count, 0);
        assert_eq!(sham.stats.unchanged_collection_count, 1);
    }

    #[test]
    fn rewireable_realization_minimizes_original_edges() {
        let collection = Uuid::from_u128(100);
        let ids = ids(4);
        let nodes = nodes(collection, &ids);
        let original = links(&[(ids[0], ids[1]), (ids[2], ids[3])]);

        let sham = build_structural_sham(&nodes, &original, &labels(&ids)).unwrap();

        assert_eq!(
            pairs(&sham.links),
            [(ids[0], ids[3]), (ids[2], ids[1])].into()
        );
        assert_eq!(sham.stats.retained_original_edge_count, 0);
        assert_eq!(sham.stats.rewired_edge_count, 2);
        assert_eq!(sham.stats.unchanged_collection_count, 0);
        assert_valid_projection(&original, &sham.links);
    }

    #[test]
    fn projection_is_stable_when_inputs_are_reordered() {
        let collection = Uuid::from_u128(100);
        let ids = ids(5);
        let mut nodes = nodes(collection, &ids);
        let mut original = links(&[
            (ids[0], ids[1]),
            (ids[0], ids[2]),
            (ids[3], ids[2]),
            (ids[4], ids[1]),
        ]);
        let expected = build_structural_sham(&nodes, &original, &labels(&ids)).unwrap();
        nodes.reverse();
        original.reverse();

        let reordered = build_structural_sham(&nodes, &original, &labels(&ids)).unwrap();

        assert_eq!(pairs(&expected.links), pairs(&reordered.links));
        assert_eq!(expected.stats, reordered.stats);
    }

    #[test]
    fn min_cost_matches_bruteforce_on_a_tiny_digraph() {
        let collection = Uuid::from_u128(100);
        let ids = ids(4);
        let nodes = nodes(collection, &ids);
        let original = links(&[
            (ids[0], ids[1]),
            (ids[0], ids[2]),
            (ids[1], ids[2]),
            (ids[3], ids[0]),
        ]);

        let sham = build_structural_sham(&nodes, &original, &labels(&ids)).unwrap();

        assert_eq!(
            usize::try_from(sham.stats.retained_original_edge_count).unwrap(),
            brute_force_minimum_overlap(&ids, &original)
        );
        assert_valid_projection(&original, &sham.links);
    }

    #[test]
    fn cross_collection_link_is_rejected() {
        let ids = ids(2);
        let nodes = vec![
            GraphNodeInput {
                node: GraphNode {
                    concept_id: ids[0],
                    collection_id: Uuid::from_u128(100),
                },
                state: NodeState::Current,
            },
            GraphNodeInput {
                node: GraphNode {
                    concept_id: ids[1],
                    collection_id: Uuid::from_u128(200),
                },
                state: NodeState::Current,
            },
        ];

        let error =
            build_structural_sham(&nodes, &links(&[(ids[0], ids[1])]), &labels(&ids)).unwrap_err();

        assert!(error.to_string().contains("cross-collection"));
    }

    #[test]
    fn duplicate_directed_link_is_rejected() {
        let collection = Uuid::from_u128(100);
        let ids = ids(2);
        let nodes = nodes(collection, &ids);
        let link = GraphLinkInput {
            source: ids[0],
            target: ids[1],
            disposition: LinkDisposition::ReviewedInternal,
        };

        let error = build_structural_sham(&nodes, &[link, link], &labels(&ids)).unwrap_err();

        assert!(error.to_string().contains("duplicate directed links"));
    }

    fn ids(count: usize) -> Vec<Uuid> {
        (0..count)
            .map(|index| Uuid::from_u128(u128::try_from(index + 1).unwrap()))
            .collect()
    }

    fn nodes(collection_id: Uuid, ids: &[Uuid]) -> Vec<GraphNodeInput> {
        ids.iter()
            .map(|concept_id| GraphNodeInput {
                node: GraphNode {
                    concept_id: *concept_id,
                    collection_id,
                },
                state: NodeState::Current,
            })
            .collect()
    }

    fn labels(ids: &[Uuid]) -> HashMap<Uuid, String> {
        ids.iter()
            .enumerate()
            .map(|(index, id)| (*id, format!("node_{index:02}")))
            .collect()
    }

    fn links(pairs: &[(Uuid, Uuid)]) -> Vec<GraphLinkInput> {
        pairs
            .iter()
            .map(|(source, target)| GraphLinkInput {
                source: *source,
                target: *target,
                disposition: LinkDisposition::ReviewedInternal,
            })
            .collect()
    }

    fn pairs(links: &[GraphLinkInput]) -> BTreeSet<(Uuid, Uuid)> {
        links
            .iter()
            .map(|link| (link.source, link.target))
            .collect()
    }

    fn assert_valid_projection(original: &[GraphLinkInput], projected: &[GraphLinkInput]) {
        assert_eq!(original.len(), projected.len());
        assert!(projected.iter().all(|link| link.source != link.target));
        assert_eq!(pairs(projected).len(), projected.len());
        assert_eq!(degrees(original), degrees(projected));
    }

    fn degrees(links: &[GraphLinkInput]) -> (BTreeMap<Uuid, usize>, BTreeMap<Uuid, usize>) {
        let mut outgoing = BTreeMap::new();
        let mut incoming = BTreeMap::new();
        for link in links {
            *outgoing.entry(link.source).or_default() += 1;
            *incoming.entry(link.target).or_default() += 1;
        }
        (outgoing, incoming)
    }

    fn brute_force_minimum_overlap(ids: &[Uuid], original: &[GraphLinkInput]) -> usize {
        let possible = ids
            .iter()
            .flat_map(|source| {
                ids.iter()
                    .filter(move |target| *target != source)
                    .map(move |target| (*source, *target))
            })
            .collect::<Vec<_>>();
        let original_pairs = pairs(original);
        let original_degrees = degrees(original);
        (0_u64..(1_u64 << possible.len()))
            .filter(|mask| mask.count_ones() as usize == original.len())
            .filter_map(|mask| {
                let candidate = possible
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1_u64 << index) != 0)
                    .map(|(_, pair)| GraphLinkInput {
                        source: pair.0,
                        target: pair.1,
                        disposition: LinkDisposition::ReviewedInternal,
                    })
                    .collect::<Vec<_>>();
                (degrees(&candidate) == original_degrees)
                    .then(|| pairs(&candidate).intersection(&original_pairs).count())
            })
            .min()
            .unwrap()
    }
}

use std::{collections::BTreeSet, fmt};

use mdstream_protocol::{
    ChangePayloadCost, ChildList, ChildListOwner, ContentNode, Document, NodeId, NodeStability,
    ProjectionOp, ResourceId,
};

use super::{
    MaterializedForest, MaterializedNode,
    definitions::SemanticCorrection,
    operations::{OperationLimitError, OperationSink},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ReconcileMetrics {
    pub(crate) nodes_visited: u64,
    pub(crate) structure_owners_visited: u64,
    pub(crate) structure_id_comparisons: u64,
    pub(crate) structure_version_steps: u64,
    pub(crate) structure_ids_emitted: u64,
    pub(crate) resources_visited: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StructureWork {
    owners_visited: u64,
    id_comparisons: u64,
    version_steps: u64,
    ids_emitted: u64,
}

impl ReconcileMetrics {
    fn add_structure(&mut self, work: StructureWork) {
        self.structure_owners_visited = self
            .structure_owners_visited
            .saturating_add(work.owners_visited);
        self.structure_id_comparisons = self
            .structure_id_comparisons
            .saturating_add(work.id_comparisons);
        self.structure_version_steps = self
            .structure_version_steps
            .saturating_add(work.version_steps);
        self.structure_ids_emitted = self.structure_ids_emitted.saturating_add(work.ids_emitted);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReconcileOutput {
    pub(crate) metrics: ReconcileMetrics,
}

pub(crate) struct ReconcileInput<'a> {
    pub(crate) document: Option<&'a Document>,
    pub(crate) stable_root_count: usize,
    pub(crate) previous_frontier_resources: &'a BTreeSet<ResourceId>,
    pub(crate) stable_resources: &'a BTreeSet<ResourceId>,
    pub(crate) newly_stable_resources: &'a BTreeSet<ResourceId>,
    pub(crate) candidate: &'a MaterializedForest,
    pub(crate) semantic_corrections: Vec<SemanticCorrection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReconcileError {
    StableRootCount,
    MissingNode(NodeId),
    CandidateOverlapsStable(NodeId),
    CandidateMissingChild(NodeId),
    ProjectionVersion(NodeId),
    OperationLimit(OperationLimitError),
    InvalidPayload(String),
    NumericOverflow(&'static str),
}

impl fmt::Display for ReconcileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StableRootCount => {
                formatter.write_str("stable root count exceeds the canonical root list")
            }
            Self::MissingNode(id) => write!(formatter, "canonical node {id} is missing"),
            Self::CandidateOverlapsStable(id) => {
                write!(formatter, "frontier candidate overlaps stable node {id}")
            }
            Self::CandidateMissingChild(id) => {
                write!(formatter, "candidate child {id} is not materialized")
            }
            Self::ProjectionVersion(id) => {
                write!(
                    formatter,
                    "candidate node {id} has an inconsistent projection version"
                )
            }
            Self::OperationLimit(error) => write!(
                formatter,
                "{} {} exceeds the configured limit of {}",
                error.field, error.actual, error.limit
            ),
            Self::InvalidPayload(message) => {
                write!(formatter, "candidate payload is invalid: {message}")
            }
            Self::NumericOverflow(field) => write!(formatter, "{field} exceeds protocol limits"),
        }
    }
}

impl From<OperationLimitError> for ReconcileError {
    fn from(error: OperationLimitError) -> Self {
        Self::OperationLimit(error)
    }
}

impl std::error::Error for ReconcileError {}

pub(crate) fn reconcile_frontier(
    input: ReconcileInput<'_>,
    operations: &mut OperationSink,
) -> Result<ReconcileOutput, ReconcileError> {
    let ReconcileInput {
        document,
        stable_root_count,
        previous_frontier_resources,
        stable_resources,
        newly_stable_resources,
        candidate,
        semantic_corrections,
    } = input;
    let current_roots = document.map_or(&[][..], |document| document.roots().as_slice());
    if stable_root_count > current_roots.len() {
        return Err(ReconcileError::StableRootCount);
    }

    let preserved_roots = &current_roots[..stable_root_count];
    let frontier_roots = &current_roots[stable_root_count..];
    let current_frontier = collect_frontier_nodes(document, frontier_roots.iter().copied())?;
    let mut metrics = ReconcileMetrics {
        nodes_visited: u64::try_from(current_frontier.len())
            .map_err(|_| ReconcileError::NumericOverflow("node visit count"))?,
        ..ReconcileMetrics::default()
    };

    for id in candidate.nodes.keys() {
        if document.is_some_and(|document| document.node(*id).is_some())
            && !current_frontier.contains(id)
        {
            return Err(ReconcileError::CandidateOverlapsStable(*id));
        }
    }
    for node in candidate.nodes.values() {
        if let Some(missing) = node
            .children
            .iter()
            .find(|child| !candidate.nodes.contains_key(child))
        {
            return Err(ReconcileError::CandidateMissingChild(*missing));
        }
    }

    reconcile_resources(document, candidate, operations, &mut metrics)?;
    for correction in semantic_corrections {
        operations.push_with(correction.cost, || correction.operation)?;
    }
    reconcile_nodes(
        document,
        &current_frontier,
        candidate,
        operations,
        &mut metrics,
    )?;
    reconcile_structures(
        document,
        preserved_roots,
        &current_frontier,
        candidate,
        operations,
        &mut metrics,
    )?;

    for id in current_frontier
        .iter()
        .filter(|id| !candidate.nodes.contains_key(id))
    {
        let current = document
            .and_then(|document| document.node(*id))
            .ok_or(ReconcileError::MissingNode(*id))?;
        operations.push_with(ChangePayloadCost::ZERO, || ProjectionOp::RemoveNode {
            node_id: *id,
            expected_version: current.version.clone(),
        })?;
    }

    for id in previous_frontier_resources {
        if candidate.resources.contains_key(id)
            || stable_resources.contains(id)
            || newly_stable_resources.contains(id)
        {
            continue;
        }
        if let Some(resource) = document.and_then(|document| document.resource(*id)) {
            operations.push_with(ChangePayloadCost::ZERO, || ProjectionOp::RemoveResource {
                resource_id: *id,
                expected_version: resource.version.clone(),
            })?;
        }
    }

    Ok(ReconcileOutput { metrics })
}

fn reconcile_resources(
    document: Option<&Document>,
    candidate: &MaterializedForest,
    operations: &mut OperationSink,
    metrics: &mut ReconcileMetrics,
) -> Result<(), ReconcileError> {
    for resource in candidate.resources.values() {
        metrics.resources_visited = metrics.resources_visited.saturating_add(1);
        match document.and_then(|document| document.resource(resource.id)) {
            None => {
                let cost = ChangePayloadCost::for_resource(resource, operations.limits())
                    .map_err(|error| ReconcileError::InvalidPayload(error.to_string()))?;
                operations.push_with(cost, || ProjectionOp::InsertResource {
                    resource: resource.clone(),
                })?;
            }
            Some(current) if current != resource => {
                let cost = ChangePayloadCost::for_resource(resource, operations.limits())
                    .map_err(|error| ReconcileError::InvalidPayload(error.to_string()))?;
                operations.push_with(cost, || ProjectionOp::ReplaceResource {
                    resource_id: resource.id,
                    expected_version: current.version.clone(),
                    resource: resource.clone(),
                })?;
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn reconcile_nodes(
    document: Option<&Document>,
    current_frontier: &BTreeSet<NodeId>,
    candidate: &MaterializedForest,
    operations: &mut OperationSink,
    metrics: &mut ReconcileMetrics,
) -> Result<(), ReconcileError> {
    for (id, node) in &candidate.nodes {
        metrics.nodes_visited = metrics.nodes_visited.saturating_add(1);
        let Some(current) = document.and_then(|document| document.node(*id)) else {
            let cost =
                ChangePayloadCost::for_projection(*id, &node.projection, operations.limits())
                    .map_err(|error| ReconcileError::InvalidPayload(error.to_string()))?;
            let permit = operations.reserve(cost)?;
            let inserted = node_without_children(*id, node)?;
            permit.commit(ProjectionOp::InsertNode { node: inserted });
            continue;
        };
        if !current_frontier.contains(id) {
            return Err(ReconcileError::CandidateOverlapsStable(*id));
        }
        if current.projection() == node.projection {
            continue;
        }
        if current.stability == NodeStability::Provisional
            && node.projection.stability == NodeStability::Stable
            && current.source == node.projection.source
            && current.body == node.projection.body
            && current.content == node.projection.content
        {
            operations.push_with(ChangePayloadCost::ZERO, || ProjectionOp::StabilizeNode {
                node_id: *id,
                expected_version: current.version.clone(),
                new_version: node.projection.version.clone(),
            })?;
        } else {
            let cost =
                ChangePayloadCost::for_projection(*id, &node.projection, operations.limits())
                    .map_err(|error| ReconcileError::InvalidPayload(error.to_string()))?;
            operations.push_with(cost, || ProjectionOp::ReplaceNode {
                node_id: *id,
                expected_version: current.version.clone(),
                projection: node.projection.clone(),
            })?;
        }
    }
    Ok(())
}

fn reconcile_structures(
    document: Option<&Document>,
    preserved_roots: &[NodeId],
    current_frontier: &BTreeSet<NodeId>,
    candidate: &MaterializedForest,
    operations: &mut OperationSink,
    metrics: &mut ReconcileMetrics,
) -> Result<(), ReconcileError> {
    for id in current_frontier {
        if candidate.nodes.contains_key(id) {
            continue;
        }
        let current = document
            .and_then(|document| document.node(*id))
            .ok_or(ReconcileError::MissingNode(*id))?;
        let work = push_minimal_splice(
            ChildListOwner::Node { node_id: *id },
            &current.children,
            0,
            0,
            |_| unreachable!("an empty child list has no elements"),
            operations,
        )?;
        metrics.add_structure(work);
    }

    for (id, node) in &candidate.nodes {
        let empty = ChildList::empty();
        let current = document
            .and_then(|document| document.node(*id))
            .map_or(&empty, |node| &node.children);
        let work = push_minimal_splice(
            ChildListOwner::Node { node_id: *id },
            current,
            0,
            node.children.len(),
            |index| node.children[index],
            operations,
        )?;
        metrics.add_structure(work);
    }

    let current = document
        .map(|document| document.roots())
        .unwrap_or_else(|| empty_roots());
    let root_count = preserved_roots
        .len()
        .checked_add(candidate.roots.len())
        .ok_or(ReconcileError::NumericOverflow("root count"))?;
    let root_work = push_minimal_splice(
        ChildListOwner::Document,
        current,
        preserved_roots.len(),
        root_count,
        |index| {
            if index < preserved_roots.len() {
                preserved_roots[index]
            } else {
                candidate.roots[index - preserved_roots.len()]
            }
        },
        operations,
    )?;
    metrics.add_structure(root_work);
    Ok(())
}

fn push_minimal_splice(
    owner: ChildListOwner,
    current: &ChildList,
    known_prefix: usize,
    next_len: usize,
    next_at: impl Fn(usize) -> NodeId,
    operations: &mut OperationSink,
) -> Result<StructureWork, ReconcileError> {
    if known_prefix > current.len() || known_prefix > next_len {
        return Err(ReconcileError::StableRootCount);
    }

    let mut work = StructureWork {
        owners_visited: 1,
        ..StructureWork::default()
    };
    let shared_limit = current.len().min(next_len);
    let mut prefix = known_prefix;
    while prefix < shared_limit {
        work.id_comparisons = work.id_comparisons.saturating_add(1);
        if current.get(prefix).copied() != Some(next_at(prefix)) {
            break;
        }
        prefix += 1;
    }

    if prefix == current.len() && prefix == next_len {
        return Ok(work);
    }

    let mut suffix = 0usize;
    while current.len().saturating_sub(suffix) > prefix && next_len.saturating_sub(suffix) > prefix
    {
        work.id_comparisons = work.id_comparisons.saturating_add(1);
        let current_index = current.len() - suffix - 1;
        let next_index = next_len - suffix - 1;
        if current.get(current_index).copied() != Some(next_at(next_index)) {
            break;
        }
        suffix += 1;
    }

    let delete_len = current
        .len()
        .checked_sub(prefix)
        .and_then(|remaining| remaining.checked_sub(suffix))
        .ok_or(ReconcileError::NumericOverflow("splice delete count"))?;
    let insert_end = next_len
        .checked_sub(suffix)
        .ok_or(ReconcileError::NumericOverflow("splice insert range"))?;
    let start =
        u32::try_from(prefix).map_err(|_| ReconcileError::NumericOverflow("splice start"))?;
    let delete_count = u32::try_from(delete_len)
        .map_err(|_| ReconcileError::NumericOverflow("splice delete count"))?;
    let insert_len = insert_end
        .checked_sub(prefix)
        .ok_or(ReconcileError::NumericOverflow("splice insert range"))?;
    work.version_steps = if prefix == current.len() && delete_len == 0 && suffix == 0 {
        u64::try_from(insert_len)
            .map_err(|_| ReconcileError::NumericOverflow("structure version steps"))?
    } else {
        u64::try_from(next_len)
            .map_err(|_| ReconcileError::NumericOverflow("structure version steps"))?
    };
    work.ids_emitted = u64::try_from(insert_len)
        .map_err(|_| ReconcileError::NumericOverflow("structure IDs emitted"))?;
    let cost = ChangePayloadCost::for_splice(insert_len, operations.limits())
        .map_err(|error| ReconcileError::InvalidPayload(error.to_string()))?;
    let permit = operations.reserve(cost)?;
    let insert = (prefix..insert_end).map(&next_at).collect::<Vec<_>>();
    let new_version = if prefix == current.len() && delete_len == 0 && suffix == 0 {
        current.version_after_append(&insert)
    } else {
        ChildList::version_for((0..next_len).map(&next_at))
    };
    permit.commit(ProjectionOp::SpliceChildren {
        owner,
        expected_version: current.version().clone(),
        start,
        delete_count,
        insert,
        new_version,
    });
    Ok(work)
}

pub(super) fn collect_frontier_nodes(
    document: Option<&Document>,
    roots: impl IntoIterator<Item = NodeId>,
) -> Result<BTreeSet<NodeId>, ReconcileError> {
    let Some(document) = document else {
        return Ok(BTreeSet::new());
    };
    let mut pending = roots.into_iter().collect::<Vec<_>>();
    let mut nodes = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !nodes.insert(id) {
            continue;
        }
        let node = document.node(id).ok_or(ReconcileError::MissingNode(id))?;
        pending.extend(node.children.iter().copied());
    }
    Ok(nodes)
}

fn node_without_children(
    id: NodeId,
    materialized: &MaterializedNode,
) -> Result<ContentNode, ReconcileError> {
    let projection = &materialized.projection;
    let node = ContentNode::new(
        id,
        projection.stability,
        projection.source,
        projection.body,
        Vec::new(),
        projection.content.clone(),
    );
    if node.version != projection.version {
        return Err(ReconcileError::ProjectionVersion(id));
    }
    Ok(node)
}

fn empty_roots() -> &'static ChildList {
    static EMPTY: std::sync::OnceLock<ChildList> = std::sync::OnceLock::new();
    EMPTY.get_or_init(ChildList::empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[u128]) -> Vec<NodeId> {
        values.iter().copied().map(NodeId::new).collect()
    }

    #[test]
    fn minimal_splices_replay_to_the_canonical_target() {
        let cases: &[(&[u128], &[u128])] = &[
            (&[], &[]),
            (&[], &[1, 2]),
            (&[1, 2], &[]),
            (&[1, 2], &[1, 2, 3]),
            (&[1, 2, 3], &[1, 2]),
            (&[2, 3], &[1, 2, 3]),
            (&[1, 3], &[1, 2, 3]),
            (&[1, 2, 3], &[1, 3]),
            (&[1, 2, 4], &[1, 3, 4]),
            (&[1, 2, 3], &[3, 2, 1]),
        ];

        for (old, next) in cases {
            let current = ChildList::new(ids(old));
            let next = ids(next);
            let mut operations = OperationSink::new(
                mdstream_protocol::ProtocolLimits {
                    max_operations: usize::MAX,
                    ..mdstream_protocol::ProtocolLimits::default()
                },
                0,
            )
            .unwrap();
            let work = push_minimal_splice(
                ChildListOwner::Document,
                &current,
                0,
                next.len(),
                |index| next[index],
                &mut operations,
            )
            .unwrap();
            let operations = operations.into_parts().0;

            assert_eq!(work.owners_visited, 1);
            if current.as_slice() == next {
                assert!(operations.is_empty());
                continue;
            }

            let [
                ProjectionOp::SpliceChildren {
                    start,
                    delete_count,
                    insert,
                    new_version,
                    ..
                },
            ] = operations.as_slice()
            else {
                panic!("a changed child list should emit one splice");
            };
            let start = usize::try_from(*start).unwrap();
            let end = start + usize::try_from(*delete_count).unwrap();
            let mut replayed = current.as_slice().to_vec();
            replayed.splice(start..end, insert.iter().copied());
            assert_eq!(replayed, next);
            assert_eq!(new_version, ChildList::new(next.clone()).version());
            assert_eq!(work.ids_emitted, insert.len() as u64);
        }
    }

    #[test]
    fn append_version_work_only_visits_the_inserted_suffix() {
        let current = ChildList::new(ids(&[1, 2]));
        let next = ids(&[1, 2, 3]);
        let mut operations = OperationSink::new(
            mdstream_protocol::ProtocolLimits {
                max_operations: usize::MAX,
                ..mdstream_protocol::ProtocolLimits::default()
            },
            0,
        )
        .unwrap();

        let work = push_minimal_splice(
            ChildListOwner::Document,
            &current,
            0,
            next.len(),
            |index| next[index],
            &mut operations,
        )
        .unwrap();

        assert_eq!(work.id_comparisons, 2);
        assert_eq!(work.version_steps, 1);
        assert_eq!(work.ids_emitted, 1);
    }
}

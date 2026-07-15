use std::collections::BTreeSet;

use mdstream_protocol::{
    ChangePayloadCost, ContentKind, Document, NodeId, NodeStability, ProjectionOp, ProtocolLimits,
    ResourceId, SemanticText, SourceCursor,
};

use super::{identity::MaterializedForest, types::CompilerError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OperationLimitError {
    pub(super) field: &'static str,
    pub(super) limit: usize,
    pub(super) actual: usize,
}

pub(super) struct OperationSink {
    operations: Vec<ProjectionOp>,
    limits: ProtocolLimits,
    reserved_tail: usize,
    payload_cost: ChangePayloadCost,
}

pub(super) struct OperationPermit<'sink> {
    sink: &'sink mut OperationSink,
    reserved_tail: bool,
    next_payload_cost: ChangePayloadCost,
}

impl OperationSink {
    pub(super) fn new(
        limits: ProtocolLimits,
        reserved_tail: usize,
    ) -> Result<Self, OperationLimitError> {
        if reserved_tail > limits.max_operations {
            return Err(OperationLimitError {
                field: "change.operations",
                limit: limits.max_operations,
                actual: reserved_tail,
            });
        }
        Ok(Self {
            operations: Vec::new(),
            limits,
            reserved_tail,
            payload_cost: ChangePayloadCost::ZERO,
        })
    }

    pub(super) fn reserve(
        &mut self,
        cost: ChangePayloadCost,
    ) -> Result<OperationPermit<'_>, OperationLimitError> {
        let actual = self
            .operations
            .len()
            .checked_add(self.reserved_tail)
            .and_then(|used| used.checked_add(1))
            .unwrap_or(usize::MAX);
        if actual > self.limits.max_operations {
            return Err(OperationLimitError {
                field: "change.operations",
                limit: self.limits.max_operations,
                actual,
            });
        }
        let structural_items = self
            .payload_cost
            .structural_items
            .saturating_add(cost.structural_items);
        if structural_items > self.limits.max_change_structural_items {
            return Err(OperationLimitError {
                field: "change.structural_items",
                limit: self.limits.max_change_structural_items,
                actual: structural_items,
            });
        }
        let metadata_bytes = self
            .payload_cost
            .metadata_bytes
            .saturating_add(cost.metadata_bytes);
        if metadata_bytes > self.limits.max_change_metadata_bytes {
            return Err(OperationLimitError {
                field: "change.metadata",
                limit: self.limits.max_change_metadata_bytes,
                actual: metadata_bytes,
            });
        }
        let wire_text_bytes = self
            .payload_cost
            .wire_text_bytes
            .checked_add(cost.wire_text_bytes)
            .expect("compiler payload reservations fit protocol limits");
        Ok(OperationPermit {
            sink: self,
            reserved_tail: false,
            next_payload_cost: ChangePayloadCost {
                structural_items,
                metadata_bytes,
                wire_text_bytes,
            },
        })
    }

    pub(super) fn reserve_tail(&mut self) -> OperationPermit<'_> {
        assert!(
            self.reserved_tail > 0,
            "operation tail permits must be declared when the sink is created"
        );
        let next_payload_cost = self.payload_cost;
        OperationPermit {
            sink: self,
            reserved_tail: true,
            next_payload_cost,
        }
    }

    pub(super) const fn limits(&self) -> ProtocolLimits {
        self.limits
    }

    pub(super) fn push_with(
        &mut self,
        cost: ChangePayloadCost,
        build: impl FnOnce() -> ProjectionOp,
    ) -> Result<(), OperationLimitError> {
        self.reserve(cost)?.commit(build());
        Ok(())
    }

    pub(super) fn push_tail_with(&mut self, build: impl FnOnce() -> ProjectionOp) {
        self.reserve_tail().commit(build());
    }

    pub(super) fn into_parts(self) -> (Vec<ProjectionOp>, ChangePayloadCost) {
        debug_assert_eq!(self.reserved_tail, 0);
        (self.operations, self.payload_cost)
    }
}

impl OperationPermit<'_> {
    pub(super) fn commit(self, operation: ProjectionOp) {
        let wire_text_overhead = operation
            .wire_text_overhead()
            .expect("compiler operation versions fit the retained address space");
        let wire_text_bytes = self
            .next_payload_cost
            .wire_text_bytes
            .checked_add(wire_text_overhead)
            .expect("compiler operation payload totals fit protocol limits");
        let payload_cost = ChangePayloadCost {
            wire_text_bytes,
            ..self.next_payload_cost
        };
        if self.reserved_tail {
            self.sink.reserved_tail -= 1;
        }
        self.sink.payload_cost = payload_cost;
        self.sink.operations.push(operation);
    }
}

pub(super) fn collect_resources(
    candidate: &MaterializedForest,
    roots: impl IntoIterator<Item = NodeId>,
) -> Result<BTreeSet<ResourceId>, CompilerError> {
    let mut resources = BTreeSet::new();
    let mut pending = roots.into_iter().collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let node = candidate.nodes.get(&id).ok_or_else(|| {
            CompilerError::InvalidReconciliation(format!("candidate node {id} is missing"))
        })?;
        if let Some(resource) = node.projection.content.referenced_resource() {
            resources.insert(resource);
        }
        pending.extend(node.children.iter().copied());
    }
    Ok(resources)
}

pub(super) fn incremental_operations(
    document: Option<&Document>,
    stable_root_count: usize,
    suffix: &str,
    current_cursor: SourceCursor,
    revision: SourceCursor,
    operations: &mut OperationSink,
) -> Result<Option<u64>, CompilerError> {
    let Some(document) = document else {
        return Ok(Some(0));
    };
    let Some(root) = document.roots().get(stable_root_count).copied() else {
        return Ok(None);
    };
    let mut path = vec![root];
    while let Some(child) = document
        .node(*path.last().expect("path is non-empty"))
        .and_then(|node| node.children.as_slice().last())
        .copied()
    {
        path.push(child);
    }
    if path.len() > 2 {
        return Ok(None);
    }

    let leaf_id = *path.last().expect("path is non-empty");
    let leaf = document.node(leaf_id).ok_or_else(|| {
        CompilerError::InvalidReconciliation(format!("canonical node {leaf_id} is missing"))
    })?;
    if !leaf.children.is_empty()
        || leaf.source.end != current_cursor
        || leaf.body.end != current_cursor
        || !can_extend_leaf_semantics(&leaf.content, suffix)
    {
        return Ok(None);
    }

    for id in path.iter().rev().copied() {
        let node = document.node(id).ok_or_else(|| {
            CompilerError::InvalidReconciliation(format!("canonical node {id} is missing"))
        })?;
        if node.stability == NodeStability::Stable {
            return Ok(None);
        }
        if node.source.end != current_cursor || node.body.end != current_cursor {
            return Ok(None);
        }
    }

    for id in path.iter().rev().copied() {
        let node = document.node(id).ok_or_else(|| {
            CompilerError::InvalidReconciliation(format!("canonical node {id} is missing"))
        })?;
        let cost = ChangePayloadCost::for_content(&node.content, operations.limits())
            .map_err(|error| CompilerError::InvalidReconciliation(error.to_string()))?;
        let permit = operations.reserve(cost)?;
        let mut projection = node.projection();
        projection.source.end = revision;
        projection.body.end = revision;
        projection.version = projection.derived_version();
        permit.commit(ProjectionOp::ReplaceNode {
            node_id: id,
            expected_version: node.version.clone(),
            projection,
        });
    }
    let visits = u64::try_from(path.len())
        .map_err(|_| CompilerError::MetricsOverflow("incremental projections"))?;
    Ok(Some(visits))
}

fn can_extend_leaf_semantics(content: &ContentKind, suffix: &str) -> bool {
    let (semantic, exact) = match content {
        ContentKind::Text { text } => (text, suffix.chars().all(is_plain_text_extension)),
        ContentKind::Html { text, .. } => (
            text,
            suffix
                .chars()
                .all(|character| !matches!(character, '\r' | '\n' | '&' | '<' | '>')),
        ),
        _ => return false,
    };
    if !exact {
        return false;
    }
    match semantic {
        SemanticText::Source {} => true,
        SemanticText::Normalized { .. } => false,
    }
}

fn is_plain_text_extension(character: char) -> bool {
    character == ' ' || character == '\t' || character.is_alphanumeric() || !character.is_ascii()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn a_full_operation_sink_rejects_before_building_the_payload() {
        let limits = ProtocolLimits {
            max_operations: 1,
            ..ProtocolLimits::default()
        };
        let mut sink = OperationSink::new(limits, 1).unwrap();
        let built = Cell::new(false);

        assert_eq!(
            sink.push_with(ChangePayloadCost::ZERO, || {
                built.set(true);
                ProjectionOp::FinishDocument
            }),
            Err(OperationLimitError {
                field: "change.operations",
                limit: 1,
                actual: 2,
            })
        );
        assert!(!built.get());

        sink.push_tail_with(|| ProjectionOp::FinishDocument);
        assert_eq!(sink.into_parts().0, vec![ProjectionOp::FinishDocument]);
    }

    #[test]
    fn structural_and_metadata_limits_reject_before_building_payloads() {
        let structural_limits = ProtocolLimits {
            max_change_structural_items: 1,
            ..ProtocolLimits::default()
        };
        let mut structural = OperationSink::new(structural_limits, 0).unwrap();
        structural
            .push_with(
                ChangePayloadCost {
                    structural_items: 1,
                    metadata_bytes: 0,
                    ..ChangePayloadCost::ZERO
                },
                || ProjectionOp::FinishDocument,
            )
            .unwrap();
        let structural_built = Cell::new(false);
        assert_eq!(
            structural.push_with(
                ChangePayloadCost {
                    structural_items: 1,
                    metadata_bytes: 0,
                    ..ChangePayloadCost::ZERO
                },
                || {
                    structural_built.set(true);
                    ProjectionOp::FinishDocument
                },
            ),
            Err(OperationLimitError {
                field: "change.structural_items",
                limit: 1,
                actual: 2,
            })
        );
        assert!(!structural_built.get());

        let metadata_limits = ProtocolLimits {
            max_change_metadata_bytes: 1,
            ..ProtocolLimits::default()
        };
        let mut metadata = OperationSink::new(metadata_limits, 0).unwrap();
        let metadata_built = Cell::new(false);
        assert_eq!(
            metadata.push_with(
                ChangePayloadCost {
                    structural_items: 0,
                    metadata_bytes: 2,
                    ..ChangePayloadCost::ZERO
                },
                || {
                    metadata_built.set(true);
                    ProjectionOp::FinishDocument
                },
            ),
            Err(OperationLimitError {
                field: "change.metadata",
                limit: 1,
                actual: 2,
            })
        );
        assert!(!metadata_built.get());
    }

    #[test]
    fn operation_sink_returns_protocol_owned_wire_text_cost() {
        let mut sink = OperationSink::new(ProtocolLimits::default(), 0).unwrap();
        sink.push_with(ChangePayloadCost::ZERO, || ProjectionOp::StabilizeNode {
            node_id: NodeId::new(1),
            expected_version: mdstream_protocol::NodeVersion::new("old").unwrap(),
            new_version: mdstream_protocol::NodeVersion::new("newer").unwrap(),
        })
        .unwrap();

        let (operations, cost) = sink.into_parts();
        assert_eq!(operations.len(), 1);
        assert_eq!(cost.wire_text_bytes, "old".len() + "newer".len());
    }
}

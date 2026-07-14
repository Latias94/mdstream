use serde::{Deserialize, Serialize};

use crate::{
    ChangeId, ContentKind, ContentNode, Coordinate, Epoch, NodeId, NodeProjection, NodeVersion,
    PayloadDigest, ProtocolError, ProtocolLimits, ProtocolMaturity, ResourceId, ResourceVersion,
    SchemaVersion, SemanticResource, Sequence, SourceCursor, StructureVersion,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// An append-only change to the document's single canonical source store.
pub struct SourceDelta {
    pub expected_cursor: SourceCursor,
    pub suffix: String,
}

impl SourceDelta {
    pub fn append(expected_cursor: SourceCursor, suffix: impl Into<String>) -> Self {
        Self {
            expected_cursor,
            suffix: suffix.into(),
        }
    }

    pub const fn unchanged(cursor: SourceCursor) -> Self {
        Self {
            expected_cursor: cursor,
            suffix: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpochStart {
    pub predecessor: Option<Coordinate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum ChildListOwner {
    Document,
    Node { node_id: NodeId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
/// A compare-and-set operation over canonical projections or resources.
pub enum ProjectionOp {
    /// Advances the exclusive source frontier covered by the canonical projection.
    ///
    /// This operation is compare-and-set against the document projection cursor.
    /// Source-only changes intentionally omit it and leave projection coverage
    /// unchanged.
    AdvanceProjection {
        expected_cursor: SourceCursor,
        new_cursor: SourceCursor,
    },
    InsertNode {
        node: ContentNode,
    },
    ReplaceNode {
        node_id: NodeId,
        expected_version: NodeVersion,
        projection: NodeProjection,
    },
    StabilizeNode {
        node_id: NodeId,
        expected_version: NodeVersion,
        new_version: NodeVersion,
    },
    RemoveNode {
        node_id: NodeId,
        expected_version: NodeVersion,
    },
    SpliceChildren {
        owner: ChildListOwner,
        expected_version: StructureVersion,
        start: u32,
        delete_count: u32,
        insert: Vec<NodeId>,
        new_version: StructureVersion,
    },
    InsertResource {
        resource: SemanticResource,
    },
    /// Replaces a resource and atomically rebinds unchanged dependent nodes.
    ///
    /// Inserted and explicitly replaced projections must already reference the
    /// replacement version. Lifecycle-only stabilization is composed with the
    /// bulk rebind before its final `NodeVersion` is checked; all other current
    /// users also receive a derived version without expanding resource fanout.
    ReplaceResource {
        resource_id: ResourceId,
        expected_version: ResourceVersion,
        resource: SemanticResource,
    },
    RemoveResource {
        resource_id: ResourceId,
        expected_version: ResourceVersion,
    },
    FinishDocument,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Aggregate payload budgets consumed by projection operations in one change.
pub struct ChangePayloadCost {
    pub structural_items: usize,
    pub metadata_bytes: usize,
}

impl ChangePayloadCost {
    pub const ZERO: Self = Self {
        structural_items: 0,
        metadata_bytes: 0,
    };

    pub fn for_insert_node(
        node: &ContentNode,
        limits: ProtocolLimits,
    ) -> Result<Self, ProtocolError> {
        let content_items = content_structural_items(&node.content);
        validate_structural_component("child_list.children", node.children.len(), limits)?;
        validate_structural_component("change.table.alignments", content_items, limits)?;
        Ok(Self {
            structural_items: node
                .children
                .len()
                .checked_add(content_items)
                .ok_or(ProtocolError::MetadataOverflow)?,
            metadata_bytes: node.validate_shape(limits)?,
        })
    }

    pub fn for_projection(
        node_id: NodeId,
        projection: &NodeProjection,
        limits: ProtocolLimits,
    ) -> Result<Self, ProtocolError> {
        let content_items = content_structural_items(&projection.content);
        validate_structural_component("change.table.alignments", content_items, limits)?;
        Ok(Self {
            structural_items: content_items,
            metadata_bytes: projection.validate_shape(node_id, limits)?,
        })
    }

    pub fn for_content(
        content: &ContentKind,
        limits: ProtocolLimits,
    ) -> Result<Self, ProtocolError> {
        let content_items = content_structural_items(content);
        validate_structural_component("change.table.alignments", content_items, limits)?;
        Ok(Self {
            structural_items: content_items,
            metadata_bytes: crate::ir::validate_kind(content, limits)?,
        })
    }

    pub fn for_splice(insert_len: usize, limits: ProtocolLimits) -> Result<Self, ProtocolError> {
        validate_structural_component("child_list.children", insert_len, limits)?;
        Ok(Self {
            structural_items: insert_len,
            metadata_bytes: 0,
        })
    }

    pub fn for_resource(
        resource: &SemanticResource,
        limits: ProtocolLimits,
    ) -> Result<Self, ProtocolError> {
        Ok(Self {
            structural_items: 0,
            metadata_bytes: resource.validate_local(limits)?,
        })
    }

    pub fn checked_add(self, other: Self) -> Result<Self, ProtocolError> {
        Ok(Self {
            structural_items: self
                .structural_items
                .checked_add(other.structural_items)
                .ok_or(ProtocolError::MetadataOverflow)?,
            metadata_bytes: self
                .metadata_bytes
                .checked_add(other.metadata_bytes)
                .ok_or(ProtocolError::MetadataOverflow)?,
        })
    }
}

fn validate_structural_component(
    field: &'static str,
    actual: usize,
    limits: ProtocolLimits,
) -> Result<(), ProtocolError> {
    if actual > limits.max_children_per_list {
        Err(ProtocolError::ValueTooLarge {
            field,
            limit: limits.max_children_per_list,
            actual,
        })
    } else {
        Ok(())
    }
}

impl ProjectionOp {
    pub fn payload_cost(&self, limits: ProtocolLimits) -> Result<ChangePayloadCost, ProtocolError> {
        match self {
            Self::InsertNode { node } => ChangePayloadCost::for_insert_node(node, limits),
            Self::ReplaceNode {
                node_id,
                projection,
                ..
            } => ChangePayloadCost::for_projection(*node_id, projection, limits),
            Self::SpliceChildren { insert, .. } => {
                ChangePayloadCost::for_splice(insert.len(), limits)
            }
            Self::InsertResource { resource } | Self::ReplaceResource { resource, .. } => {
                ChangePayloadCost::for_resource(resource, limits)
            }
            _ => Ok(ChangePayloadCost::ZERO),
        }
    }

    pub(crate) fn node_target(&self) -> Option<NodeId> {
        match self {
            Self::InsertNode { node } => Some(node.id),
            Self::ReplaceNode { node_id, .. }
            | Self::StabilizeNode { node_id, .. }
            | Self::RemoveNode { node_id, .. } => Some(*node_id),
            _ => None,
        }
    }

    pub(crate) fn resource_target(&self) -> Option<ResourceId> {
        match self {
            Self::InsertResource { resource } => Some(resource.id),
            Self::ReplaceResource { resource_id, .. }
            | Self::RemoveResource { resource_id, .. } => Some(*resource_id),
            _ => None,
        }
    }

    pub(crate) fn structure_target(&self) -> Option<ChildListOwner> {
        match self {
            Self::SpliceChildren { owner, .. } => Some(*owner),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// One atomic, ordered source-and-projection transition.
///
/// Construction validates the envelope. The canonical [`crate::Reducer`]
/// performs state-dependent CAS, ownership, grammar, and resource validation
/// before committing the complete set atomically.
pub struct ChangeSet {
    schema: SchemaVersion,
    maturity: ProtocolMaturity,
    epoch: Epoch,
    sequence: Sequence,
    change_id: ChangeId,
    epoch_start: Option<EpochStart>,
    source: SourceDelta,
    operations: Vec<ProjectionOp>,
}

impl ChangeSet {
    pub fn new(
        epoch: Epoch,
        sequence: Sequence,
        change_id: ChangeId,
        source: SourceDelta,
        operations: Vec<ProjectionOp>,
    ) -> Result<Self, ProtocolError> {
        let change = Self {
            schema: SchemaVersion::current(),
            maturity: ProtocolMaturity::Draft,
            epoch,
            sequence,
            change_id,
            epoch_start: None,
            source,
            operations,
        };
        change.validate_envelope()?;
        if change.source.suffix.is_empty() && change.operations.is_empty() {
            return Err(ProtocolError::InvalidChange(
                "ordinary changes cannot be empty".to_string(),
            ));
        }
        Ok(change)
    }

    pub fn start_epoch(
        epoch: Epoch,
        change_id: ChangeId,
        predecessor: Option<Coordinate>,
        source: SourceDelta,
        operations: Vec<ProjectionOp>,
    ) -> Result<Self, ProtocolError> {
        let change = Self {
            schema: SchemaVersion::current(),
            maturity: ProtocolMaturity::Draft,
            epoch,
            sequence: Sequence::new(0),
            change_id,
            epoch_start: Some(EpochStart { predecessor }),
            source,
            operations,
        };
        change.validate_envelope()?;
        Ok(change)
    }

    pub fn schema(&self) -> &SchemaVersion {
        &self.schema
    }

    pub const fn maturity(&self) -> ProtocolMaturity {
        self.maturity
    }

    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub const fn sequence(&self) -> Sequence {
        self.sequence
    }

    pub fn change_id(&self) -> &ChangeId {
        &self.change_id
    }

    pub fn epoch_start(&self) -> Option<&EpochStart> {
        self.epoch_start.as_ref()
    }

    pub fn source(&self) -> &SourceDelta {
        &self.source
    }

    pub fn operations(&self) -> &[ProjectionOp] {
        &self.operations
    }

    pub fn payload_digest(&self) -> PayloadDigest {
        #[derive(Serialize)]
        struct DigestView<'a> {
            schema: &'a SchemaVersion,
            maturity: ProtocolMaturity,
            epoch: Epoch,
            sequence: Sequence,
            epoch_start: &'a Option<EpochStart>,
            source: &'a SourceDelta,
            operations: &'a [ProjectionOp],
        }

        PayloadDigest::digest_json(&DigestView {
            schema: &self.schema,
            maturity: self.maturity,
            epoch: self.epoch,
            sequence: self.sequence,
            epoch_start: &self.epoch_start,
            source: &self.source,
            operations: &self.operations,
        })
    }

    pub(crate) fn validate_envelope(&self) -> Result<(), ProtocolError> {
        self.schema.ensure_supported()?;
        if self.maturity != ProtocolMaturity::Draft {
            return Err(ProtocolError::UnsupportedSchema(format!(
                "maturity {:?}",
                self.maturity
            )));
        }
        if self.epoch_start.is_some() && self.sequence != Sequence::new(0) {
            return Err(ProtocolError::InvalidChange(
                "EpochStart must use sequence zero".to_string(),
            ));
        }
        if self.epoch_start.is_some() && self.source.expected_cursor != SourceCursor::new(0) {
            return Err(ProtocolError::InvalidChange(
                "EpochStart must append to an empty source".to_string(),
            ));
        }
        if self.epoch_start.is_none() && self.sequence == Sequence::new(0) {
            return Err(ProtocolError::InvalidChange(
                "ordinary changes cannot use sequence zero".to_string(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_complete(&self, limits: ProtocolLimits) -> Result<(), ProtocolError> {
        self.validate_envelope()?;
        if self.epoch_start.is_none() && self.source.suffix.is_empty() && self.operations.is_empty()
        {
            return Err(ProtocolError::InvalidChange(
                "ordinary changes cannot be empty".to_string(),
            ));
        }
        if self.source.suffix.len() > limits.max_source_bytes {
            return Err(ProtocolError::SourceTooLarge {
                limit: limits.max_source_bytes,
                actual: self.source.suffix.len(),
            });
        }
        if self.operations.len() > limits.max_operations {
            return Err(ProtocolError::TooManyOperations {
                limit: limits.max_operations,
                actual: self.operations.len(),
            });
        }

        let mut payload_cost = ChangePayloadCost::ZERO;
        for operation in &self.operations {
            payload_cost = payload_cost.checked_add(operation.payload_cost(limits)?)?;
        }
        if payload_cost.structural_items > limits.max_change_structural_items {
            return Err(ProtocolError::ValueTooLarge {
                field: "change.structural_items",
                limit: limits.max_change_structural_items,
                actual: payload_cost.structural_items,
            });
        }
        if payload_cost.metadata_bytes > limits.max_change_metadata_bytes {
            return Err(ProtocolError::ValueTooLarge {
                field: "change.metadata",
                limit: limits.max_change_metadata_bytes,
                actual: payload_cost.metadata_bytes,
            });
        }
        Ok(())
    }
}

fn content_structural_items(content: &crate::ContentKind) -> usize {
    match content {
        crate::ContentKind::Table { alignments } => alignments.len(),
        _ => 0,
    }
}

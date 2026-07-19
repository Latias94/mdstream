use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    ApplyOutcome, ChildListOwner, ContinuityGeneration, Coordinate, DocumentLifecycle, Epoch,
    NodeId, NodeStability, NodeVersion, ProtocolError, ResourceId, ResourceVersion, SourceCursor,
    SourceRange, StructureVersion,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentStateStamp {
    pub continuity_generation: ContinuityGeneration,
    pub coordinate: Coordinate,
    pub lifecycle: DocumentLifecycle,
    pub projection_cursor: SourceCursor,
    pub roots_version: StructureVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionNodeKey {
    pub continuity_generation: ContinuityGeneration,
    pub epoch: Epoch,
    pub node_id: NodeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionResourceKey {
    pub continuity_generation: ContinuityGeneration,
    pub epoch: Epoch,
    pub resource_id: ResourceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum TransitionChildListOwner {
    Document,
    Node { key: TransitionNodeKey },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeStateStamp {
    pub version: NodeVersion,
    pub stability: NodeStability,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub parent: Option<TransitionChildListOwner>,
    pub children_version: StructureVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum TextTransition {
    ProjectionAppend { range: SourceRange, text: String },
    Replacement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeTransition {
    pub key: TransitionNodeKey,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub before: Option<NodeStateStamp>,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub after: Option<NodeStateStamp>,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub text: Option<TextTransition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureTransition {
    pub owner: TransitionChildListOwner,
    pub before_version: StructureVersion,
    pub after_version: StructureVersion,
    pub start: u32,
    pub removed: Vec<TransitionNodeKey>,
    pub inserted: Vec<TransitionNodeKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceTransition {
    pub key: TransitionResourceKey,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub before_version: Option<ResourceVersion>,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub after_version: Option<ResourceVersion>,
    pub affected_nodes: Vec<TransitionNodeKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "scope", deny_unknown_fields)]
pub enum TransitionFacts {
    Continuous {
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        before: Option<DocumentStateStamp>,
        after: DocumentStateStamp,
        nodes: Vec<NodeTransition>,
        structures: Vec<StructureTransition>,
        resources: Vec<ResourceTransition>,
    },
    FullReplace {
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        before: Option<DocumentStateStamp>,
        after: DocumentStateStamp,
    },
}

impl TransitionFacts {
    pub const fn before(&self) -> Option<&DocumentStateStamp> {
        match self {
            Self::Continuous { before, .. } | Self::FullReplace { before, .. } => before.as_ref(),
        }
    }

    pub const fn after(&self) -> &DocumentStateStamp {
        match self {
            Self::Continuous { after, .. } | Self::FullReplace { after, .. } => after,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionOutcome {
    pub outcome: ApplyOutcome,
    pub facts: Option<TransitionFacts>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransitionMetrics {
    /// Successfully emitted transition fact sets.
    pub facts_built: u64,
    /// Changed entities visited only to derive transition facts.
    pub entity_visits: u64,
    /// Facts-specific element copies into the splice journal and qualified output keys.
    pub splice_ids_copied: u64,
    /// UTF-8 bytes copied into owned projection-append facts.
    pub owned_text_bytes_copied: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    Protocol(ProtocolError),
    ContinuityOverflow,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => error.fmt(formatter),
            Self::ContinuityOverflow => formatter.write_str("transition continuity overflow"),
        }
    }
}

impl std::error::Error for TransitionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Protocol(error) => Some(error),
            Self::ContinuityOverflow => None,
        }
    }
}

impl From<ProtocolError> for TransitionError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

pub(crate) fn qualify_owner(
    owner: ChildListOwner,
    continuity_generation: ContinuityGeneration,
    epoch: Epoch,
) -> TransitionChildListOwner {
    match owner {
        ChildListOwner::Document => TransitionChildListOwner::Document,
        ChildListOwner::Node { node_id } => TransitionChildListOwner::Node {
            key: TransitionNodeKey {
                continuity_generation,
                epoch,
                node_id,
            },
        },
    }
}

use serde::{Deserialize, Serialize};

use crate::{Coordinate, Epoch, NodeId, ResourceId, Sequence};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentLifecycle {
    #[default]
    Open,
    Finalized,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStability {
    #[default]
    Provisional,
    Stable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum RecoveryReason {
    SequenceGap {
        expected: Sequence,
        received: Sequence,
    },
    SequenceFork {
        sequence: Sequence,
    },
    UnannouncedEpoch {
        current: Epoch,
        received: Epoch,
    },
    SourceDivergence,
    ProjectionDivergence,
    VersionDivergence,
    StructureDivergence,
    ResourceDivergence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReducerStatus {
    Uninitialized,
    Ready,
    NeedsSnapshot {
        last_good: Coordinate,
        reason: RecoveryReason,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeImpact {
    /// Node view keys invalidated by the transition, including removed nodes.
    pub changed_nodes: Vec<NodeId>,
    /// Removed node keys. This is a subset of `changed_nodes`.
    pub removed_nodes: Vec<NodeId>,
    /// Resource view keys invalidated by the transition, including removals.
    pub changed_resources: Vec<ResourceId>,
    /// Removed resource keys. This is a subset of `changed_resources`.
    pub removed_resources: Vec<ResourceId>,
    pub source_changed: bool,
    pub projection_changed: bool,
    pub lifecycle_changed: bool,
    pub roots_changed: bool,
    pub full_replace: bool,
}

impl ChangeImpact {
    pub fn is_empty(&self) -> bool {
        self.changed_nodes.is_empty()
            && self.removed_nodes.is_empty()
            && self.changed_resources.is_empty()
            && self.removed_resources.is_empty()
            && !self.source_changed
            && !self.projection_changed
            && !self.lifecycle_changed
            && !self.roots_changed
            && !self.full_replace
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied {
        coordinate: Coordinate,
        impact: ChangeImpact,
    },
    Recovered {
        coordinate: Coordinate,
        impact: ChangeImpact,
    },
    Idempotent,
    Stale {
        current: Coordinate,
        received_epoch: Epoch,
        received_sequence: Sequence,
    },
    RecoveryRequired {
        last_good: Coordinate,
        reason: RecoveryReason,
    },
}

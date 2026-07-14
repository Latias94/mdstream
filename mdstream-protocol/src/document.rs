use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ApplyOutcome, ChangeId, ChangeImpact, ChangeSet, ChildList, ChildListOwner,
    ChildSequenceCompleteness, ChildSequenceValidator, ContentKind, ContentNode, Coordinate,
    DocumentLifecycle, Epoch, NodeId, NodeStability, PayloadDigest, ProjectionOp, ProtocolError,
    ProtocolLimits, ProtocolMaturity, RecoveryReason, ReducerStatus, ResourceId, SemanticResource,
    SemanticResourceKind, Sequence, Snapshot, SourceCursor, validate_child_kind,
    validate_table_row_width,
};

#[derive(Debug, Clone, PartialEq, Eq)]
/// The reducer-owned canonical document for one epoch.
///
/// Source text is stored once. Nodes carry ranges and semantic projections,
/// while parent indexes and resource dependency indexes remain reducer-private.
pub struct Document {
    coordinate: Coordinate,
    last_payload_digest: PayloadDigest,
    lifecycle: DocumentLifecycle,
    source: String,
    projection_cursor: SourceCursor,
    roots: ChildList,
    nodes: BTreeMap<NodeId, ContentNode>,
    provisional_nodes: BTreeSet<NodeId>,
    parents: BTreeMap<NodeId, ChildListOwner>,
    resources: BTreeMap<ResourceId, SemanticResource>,
    resource_users: BTreeMap<ResourceId, BTreeSet<NodeId>>,
    metadata_bytes: usize,
    structural_items: usize,
}

impl Document {
    fn blank(epoch: Epoch, change_id: ChangeId, digest: PayloadDigest) -> Self {
        Self {
            coordinate: Coordinate {
                epoch,
                sequence: Sequence::new(0),
                change_id,
                source_cursor: SourceCursor::new(0),
            },
            last_payload_digest: digest,
            lifecycle: DocumentLifecycle::Open,
            source: String::new(),
            projection_cursor: SourceCursor::new(0),
            roots: ChildList::empty(),
            nodes: BTreeMap::new(),
            provisional_nodes: BTreeSet::new(),
            parents: BTreeMap::new(),
            resources: BTreeMap::new(),
            resource_users: BTreeMap::new(),
            metadata_bytes: 0,
            structural_items: 0,
        }
    }

    fn from_validated_snapshot(snapshot: &Snapshot, validation: ValidationStats) -> Self {
        let nodes = snapshot
            .nodes()
            .iter()
            .map(clone_node_owned)
            .map(|node| (node.id, node))
            .collect::<BTreeMap<_, _>>();
        let parents = build_parent_index(snapshot.roots(), &nodes)
            .expect("validated snapshots have one owner for every node");
        let resources = snapshot
            .resources()
            .iter()
            .cloned()
            .map(|resource| (resource.id, resource))
            .collect::<BTreeMap<_, _>>();
        let resource_users = build_resource_users(&nodes);
        let provisional_nodes = nodes
            .values()
            .filter_map(|node| (node.stability == NodeStability::Provisional).then_some(node.id))
            .collect();
        Self {
            coordinate: snapshot.coordinate().clone(),
            last_payload_digest: snapshot.last_payload_digest().clone(),
            lifecycle: snapshot.lifecycle(),
            source: snapshot.source().to_string(),
            projection_cursor: snapshot.projection_cursor(),
            roots: clone_child_list_owned(snapshot.roots()),
            nodes,
            provisional_nodes,
            parents,
            resources,
            resource_users,
            metadata_bytes: validation.metadata_bytes,
            structural_items: validation.structural_items,
        }
    }

    pub fn coordinate(&self) -> &Coordinate {
        &self.coordinate
    }

    pub const fn lifecycle(&self) -> DocumentLifecycle {
        self.lifecycle
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the exclusive source frontier represented by the canonical projection.
    pub const fn projection_cursor(&self) -> SourceCursor {
        self.projection_cursor
    }

    /// Returns the canonical source range not yet represented by the projection.
    pub const fn pending_source_range(&self) -> crate::SourceRange {
        crate::SourceRange::new(self.projection_cursor, self.coordinate.source_cursor)
    }

    /// Returns the canonical source suffix not yet represented by the projection.
    pub fn pending_source(&self) -> &str {
        let start = usize::try_from(self.projection_cursor.get())
            .expect("canonical projection cursors fit the source address space");
        self.source
            .get(start..)
            .expect("canonical projection cursors are UTF-8 boundaries")
    }

    pub fn roots(&self) -> &ChildList {
        &self.roots
    }

    pub fn nodes(&self) -> impl ExactSizeIterator<Item = &ContentNode> {
        self.nodes.values()
    }

    pub fn node(&self, id: NodeId) -> Option<&ContentNode> {
        self.nodes.get(&id)
    }

    pub fn parent(&self, id: NodeId) -> Option<ChildListOwner> {
        self.parents.get(&id).copied()
    }

    pub fn resources(&self) -> impl ExactSizeIterator<Item = &SemanticResource> {
        self.resources.values()
    }

    pub fn resource(&self, id: ResourceId) -> Option<&SemanticResource> {
        self.resources.get(&id)
    }

    pub const fn metadata_bytes(&self) -> usize {
        self.metadata_bytes
    }

    pub const fn structural_items(&self) -> usize {
        self.structural_items
    }

    /// Returns the number of nodes that must stabilize before finalization.
    pub fn provisional_node_count(&self) -> usize {
        self.provisional_nodes.len()
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot::from_canonical_parts(crate::wire::CanonicalSnapshotParts {
            coordinate: self.coordinate.clone(),
            last_payload_digest: self.last_payload_digest.clone(),
            lifecycle: self.lifecycle,
            source: self.source.clone(),
            projection_cursor: self.projection_cursor,
            roots: clone_child_list_owned(&self.roots),
            nodes: self.nodes.values().map(clone_node_owned).collect(),
            resources: self.resources.values().cloned().collect(),
        })
    }
}

fn clone_child_list_owned(list: &ChildList) -> ChildList {
    list.clone()
}

fn clone_node_owned(node: &ContentNode) -> ContentNode {
    node.clone()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReducerMetrics {
    pub applied_changes: u64,
    pub operations_visited: u64,
    pub nodes_validated: u64,
    pub relationship_steps: u64,
    pub child_ids_copied: u64,
    pub snapshots_validated: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Control {
    Uninitialized,
    Ready,
    NeedsSnapshot {
        last_good: Coordinate,
        last_digest: PayloadDigest,
        reason: RecoveryReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotProgression {
    SameFloor,
    Advanced,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Canonical replay state machine for [`ChangeSet`].
///
/// The reducer is the sole authority for sequence routing, snapshot recovery,
/// lifecycle transitions, ownership, and atomic projection updates.
pub struct Reducer {
    control: Control,
    document: Option<Document>,
    limits: ProtocolLimits,
    metrics: ReducerMetrics,
}

impl Default for Reducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer {
    pub fn new() -> Self {
        Self::with_limits(ProtocolLimits::default())
    }

    pub fn with_limits(limits: ProtocolLimits) -> Self {
        Self {
            control: Control::Uninitialized,
            document: None,
            limits,
            metrics: ReducerMetrics::default(),
        }
    }

    pub fn status(&self) -> ReducerStatus {
        match &self.control {
            Control::Uninitialized => ReducerStatus::Uninitialized,
            Control::Ready => ReducerStatus::Ready,
            Control::NeedsSnapshot {
                last_good, reason, ..
            } => ReducerStatus::NeedsSnapshot {
                last_good: last_good.clone(),
                reason: reason.clone(),
            },
        }
    }

    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }

    pub const fn metrics(&self) -> ReducerMetrics {
        self.metrics
    }

    /// Applies one ordered change or classifies it as retry, stale, or recovery.
    ///
    /// Invalid changes never partially mutate the retained document.
    pub fn apply(&mut self, change: ChangeSet) -> Result<ApplyOutcome, ProtocolError> {
        self.apply_ref(&change)
    }

    fn apply_ref(&mut self, change: &ChangeSet) -> Result<ApplyOutcome, ProtocolError> {
        if matches!(self.control, Control::NeedsSnapshot { .. }) && change.epoch_start().is_none() {
            return Err(ProtocolError::NeedsSnapshot);
        }
        change.validate_envelope()?;

        match &self.control {
            Control::Uninitialized => self.apply_initial_epoch(change),
            Control::NeedsSnapshot { .. } => self.apply_recovery_epoch(change),
            Control::Ready => self.apply_ready(change),
        }
    }

    /// Applies a producer-authored change without retaining recovery routing
    /// state for a non-canonical outcome.
    ///
    /// Producers use this to validate their own changes against the canonical
    /// reducer without cloning the retained document. State-changing
    /// [`ApplyOutcome::Applied`] and [`ApplyOutcome::Recovered`] results commit
    /// normally. Every other outcome, and every error, preserves the reducer's
    /// prior routing state and metrics; the retained document is already
    /// transactional under [`Self::apply`].
    pub fn apply_producer(&mut self, change: ChangeSet) -> Result<ApplyOutcome, ProtocolError> {
        self.apply_producer_ref(&change)
    }

    /// Borrowed producer apply that leaves the authored change available for
    /// transport after canonical validation.
    pub fn apply_producer_ref(
        &mut self,
        change: &ChangeSet,
    ) -> Result<ApplyOutcome, ProtocolError> {
        let control = self.control.clone();
        let metrics = self.metrics;
        match self.apply_ref(change) {
            Ok(outcome @ (ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. })) => {
                Ok(outcome)
            }
            Ok(outcome @ ApplyOutcome::Idempotent)
            | Ok(outcome @ ApplyOutcome::Stale { .. })
            | Ok(outcome @ ApplyOutcome::RecoveryRequired { .. }) => {
                self.control = control;
                self.metrics = metrics;
                Ok(outcome)
            }
            Err(error) => {
                self.control = control;
                self.metrics = metrics;
                Err(error)
            }
        }
    }

    /// Installs a fully validated snapshot during bootstrap or recovery.
    ///
    /// Ready reducers reject snapshot replacement so callers cannot bypass the
    /// ordered change protocol.
    pub fn recover_snapshot(&mut self, snapshot: Snapshot) -> Result<ApplyOutcome, ProtocolError> {
        if matches!(self.control, Control::Ready) {
            return Err(ProtocolError::SnapshotNotAllowed);
        }

        let validation = validate_snapshot(&snapshot, self.limits)?;
        let progression = if let Control::NeedsSnapshot {
            last_good,
            last_digest,
            ..
        } = &self.control
        {
            self.validate_snapshot_progression(&snapshot, last_good, last_digest)?
        } else {
            SnapshotProgression::Advanced
        };

        if progression == SnapshotProgression::SameFloor {
            let coordinate = self
                .document
                .as_ref()
                .expect("same-floor recovery retains a document")
                .coordinate
                .clone();
            self.control = Control::Ready;
            self.record_snapshot_validation(validation);
            return Ok(ApplyOutcome::Recovered {
                coordinate,
                impact: ChangeImpact::default(),
            });
        }

        let replacement = Document::from_validated_snapshot(&snapshot, validation);
        let impact = replacement_impact(self.document.as_ref(), &replacement, true);
        let coordinate = replacement.coordinate.clone();
        self.document = Some(replacement);
        self.control = Control::Ready;
        self.record_snapshot_validation(validation);
        Ok(ApplyOutcome::Recovered { coordinate, impact })
    }

    fn record_snapshot_validation(&mut self, validation: ValidationStats) {
        self.metrics.snapshots_validated = self.metrics.snapshots_validated.saturating_add(1);
        self.metrics.nodes_validated = self
            .metrics
            .nodes_validated
            .saturating_add(usize_to_u64(validation.nodes));
        self.metrics.relationship_steps = self
            .metrics
            .relationship_steps
            .saturating_add(usize_to_u64(validation.relationship_steps));
    }

    fn validate_snapshot_progression(
        &self,
        snapshot: &Snapshot,
        last_good: &Coordinate,
        last_digest: &PayloadDigest,
    ) -> Result<SnapshotProgression, ProtocolError> {
        if snapshot.coordinate().epoch < last_good.epoch {
            return Err(ProtocolError::StaleSnapshot {
                floor: last_good.sequence,
                received: snapshot.coordinate().sequence,
            });
        }
        if snapshot.coordinate().epoch > last_good.epoch {
            return Ok(SnapshotProgression::Advanced);
        }
        if snapshot.coordinate().sequence < last_good.sequence {
            return Err(ProtocolError::StaleSnapshot {
                floor: last_good.sequence,
                received: snapshot.coordinate().sequence,
            });
        }

        let retained = self
            .document
            .as_ref()
            .expect("recovery mode retains the last-good document");
        if snapshot.coordinate().sequence == last_good.sequence {
            if snapshot.coordinate() != last_good
                || snapshot.last_payload_digest() != last_digest
                || !snapshot_matches_document(snapshot, retained)
            {
                return Err(ProtocolError::InvalidSnapshot(
                    "same-floor snapshot differs from retained canonical state".to_string(),
                ));
            }
            return Ok(SnapshotProgression::SameFloor);
        }

        if retained.lifecycle == DocumentLifecycle::Finalized {
            return Err(ProtocolError::InvalidSnapshot(
                "a finalized epoch cannot advance through snapshot recovery".to_string(),
            ));
        }
        if !snapshot.source().starts_with(&retained.source) {
            return Err(ProtocolError::InvalidSnapshot(
                "same-epoch snapshot source must preserve the retained prefix".to_string(),
            ));
        }
        if snapshot.coordinate().source_cursor < retained.coordinate.source_cursor {
            return Err(ProtocolError::InvalidSnapshot(
                "same-epoch snapshot source cursor cannot move backwards".to_string(),
            ));
        }
        if snapshot.projection_cursor() < retained.projection_cursor {
            return Err(ProtocolError::InvalidSnapshot(
                "same-epoch snapshot projection cursor cannot move backwards".to_string(),
            ));
        }
        if retained.lifecycle == DocumentLifecycle::Finalized
            && snapshot.lifecycle() != DocumentLifecycle::Finalized
        {
            return Err(ProtocolError::InvalidSnapshot(
                "same-epoch snapshot cannot reopen a finalized document".to_string(),
            ));
        }

        let incoming_nodes = snapshot
            .nodes()
            .iter()
            .map(|node| (node.id, node))
            .collect::<BTreeMap<_, _>>();
        for old in retained.nodes.values() {
            if old.stability == NodeStability::Stable {
                if incoming_nodes
                    .get(&old.id)
                    .is_some_and(|node| node.stability == NodeStability::Provisional)
                {
                    return Err(ProtocolError::InvalidSnapshot(
                        "same-epoch snapshot makes a stable node provisional".to_string(),
                    ));
                }
                if incoming_nodes
                    .get(&old.id)
                    .is_some_and(|node| stable_table_width_changed(&old.content, &node.content))
                {
                    return Err(ProtocolError::InvalidSnapshot(
                        "same-epoch snapshot changed a stable table column count".to_string(),
                    ));
                }
            }
        }
        let incoming_resources = snapshot
            .resources()
            .iter()
            .map(|resource| (resource.id, resource))
            .collect::<BTreeMap<_, _>>();
        for current in retained.resources.values() {
            let Some(replacement) = incoming_resources.get(&current.id) else {
                continue;
            };
            validate_resource_identity(current, replacement).map_err(|_| {
                ProtocolError::InvalidSnapshot(
                    "same-epoch snapshot changed a resource's semantic identity".to_string(),
                )
            })?;
        }
        Ok(SnapshotProgression::Advanced)
    }

    fn apply_initial_epoch(&mut self, change: &ChangeSet) -> Result<ApplyOutcome, ProtocolError> {
        let Some(epoch_start) = change.epoch_start() else {
            return Err(ProtocolError::InvalidEpochStart {
                current: None,
                received: change.epoch(),
            });
        };
        if epoch_start.predecessor.is_some() {
            return Err(ProtocolError::InvalidEpochStart {
                current: None,
                received: change.epoch(),
            });
        }
        change.validate_complete(self.limits)?;
        self.install_epoch(change, false)
    }

    fn apply_ready(&mut self, change: &ChangeSet) -> Result<ApplyOutcome, ProtocolError> {
        let document = self
            .document
            .as_ref()
            .expect("ready reducer has a document");
        let current = document.coordinate.clone();

        if change.epoch() < current.epoch {
            return Ok(ApplyOutcome::Stale {
                current,
                received_epoch: change.epoch(),
                received_sequence: change.sequence(),
            });
        }
        if change.epoch() > current.epoch {
            if change.epoch_start().is_some() {
                self.validate_ready_predecessor(change, &current)?;
                change.validate_complete(self.limits)?;
                return self.install_epoch(change, true);
            }
            return Ok(self.enter_recovery(RecoveryReason::UnannouncedEpoch {
                current: current.epoch,
                received: change.epoch(),
            }));
        }
        if change.sequence() < current.sequence {
            return Ok(ApplyOutcome::Stale {
                current,
                received_epoch: change.epoch(),
                received_sequence: change.sequence(),
            });
        }
        if change.sequence() == current.sequence {
            if change.change_id() != &current.change_id {
                return Ok(self.enter_recovery(RecoveryReason::SequenceFork {
                    sequence: change.sequence(),
                }));
            }
            change.validate_complete(self.limits)?;
            if change.payload_digest() == document.last_payload_digest {
                return Ok(ApplyOutcome::Idempotent);
            }
            return Ok(self.enter_recovery(RecoveryReason::SequenceFork {
                sequence: change.sequence(),
            }));
        }

        if document.lifecycle == DocumentLifecycle::Finalized {
            return Err(ProtocolError::IllegalLifecycle(
                "a finalized document accepts only a new epoch".to_string(),
            ));
        }
        let expected = current
            .sequence
            .checked_add(1)
            .ok_or(ProtocolError::SequenceOverflow)?;
        if change.sequence() != expected {
            return Ok(self.enter_recovery(RecoveryReason::SequenceGap {
                expected,
                received: change.sequence(),
            }));
        }
        if change.epoch_start().is_some() {
            return Err(ProtocolError::InvalidEpochStart {
                current: Some(current.epoch),
                received: change.epoch(),
            });
        }
        change.validate_complete(self.limits)?;

        let staged = stage_document(document, change, self.limits);
        match staged {
            Ok(staged) => self.commit_ready(change, staged),
            Err(StageFailure::Divergence(reason)) => Ok(self.enter_recovery(reason)),
            Err(StageFailure::Invalid(error)) => Err(error),
        }
    }

    fn validate_ready_predecessor(
        &self,
        change: &ChangeSet,
        current: &Coordinate,
    ) -> Result<(), ProtocolError> {
        let predecessor = change
            .epoch_start()
            .and_then(|start| start.predecessor.as_ref());
        if predecessor != Some(current) || change.epoch() <= current.epoch {
            return Err(ProtocolError::InvalidEpochStart {
                current: Some(current.epoch),
                received: change.epoch(),
            });
        }
        Ok(())
    }

    fn apply_recovery_epoch(&mut self, change: &ChangeSet) -> Result<ApplyOutcome, ProtocolError> {
        let Control::NeedsSnapshot {
            last_good,
            last_digest,
            ..
        } = &self.control
        else {
            unreachable!();
        };
        let predecessor = change
            .epoch_start()
            .and_then(|start| start.predecessor.as_ref())
            .ok_or(ProtocolError::InvalidEpochStart {
                current: Some(last_good.epoch),
                received: change.epoch(),
            })?;
        let same_floor = predecessor == last_good;
        let advances_floor = predecessor.epoch == last_good.epoch
            && predecessor.sequence > last_good.sequence
            && predecessor.source_cursor >= last_good.source_cursor;
        let retained_finalized = self
            .document
            .as_ref()
            .is_some_and(|document| document.lifecycle == DocumentLifecycle::Finalized);
        if change.epoch() <= predecessor.epoch
            || (!same_floor && !advances_floor)
            || (retained_finalized && !same_floor)
        {
            return Err(ProtocolError::InvalidEpochStart {
                current: Some(last_good.epoch),
                received: change.epoch(),
            });
        }
        if same_floor
            && self
                .document
                .as_ref()
                .is_some_and(|document| document.last_payload_digest != *last_digest)
        {
            return Err(ProtocolError::InvalidEpochStart {
                current: Some(last_good.epoch),
                received: change.epoch(),
            });
        }
        change.validate_complete(self.limits)?;
        self.install_epoch(change, true)
    }

    fn install_epoch(
        &mut self,
        change: &ChangeSet,
        recovered: bool,
    ) -> Result<ApplyOutcome, ProtocolError> {
        let digest = change.payload_digest();
        let blank = Document::blank(change.epoch(), change.change_id().clone(), digest);
        let staged =
            stage_document(&blank, change, self.limits).map_err(|failure| match failure {
                StageFailure::Divergence(_) => ProtocolError::InvalidChange(
                    "EpochStart diverged from its empty document".to_string(),
                ),
                StageFailure::Invalid(error) => error,
            })?;
        let staged_impact = staged.impact.clone();
        let stats = staged.stats;
        let mut replacement = blank;
        commit_document(&mut replacement, change, staged);
        let impact = if recovered || self.document.is_some() {
            replacement_impact(self.document.as_ref(), &replacement, true)
        } else {
            staged_impact
        };
        let coordinate = replacement.coordinate.clone();
        self.document = Some(replacement);
        self.control = Control::Ready;
        self.record_apply(change.operations().len(), stats);
        if recovered {
            Ok(ApplyOutcome::Recovered { coordinate, impact })
        } else {
            Ok(ApplyOutcome::Applied { coordinate, impact })
        }
    }

    fn commit_ready(
        &mut self,
        change: &ChangeSet,
        staged: StagedChange,
    ) -> Result<ApplyOutcome, ProtocolError> {
        let impact = staged.impact.clone();
        let stats = staged.stats;
        let operation_count = change.operations().len();
        let document = self
            .document
            .as_mut()
            .expect("ready reducer has a document");
        commit_document(document, change, staged);
        let coordinate = document.coordinate.clone();
        self.control = Control::Ready;
        self.record_apply(operation_count, stats);
        Ok(ApplyOutcome::Applied { coordinate, impact })
    }

    fn record_apply(&mut self, operation_count: usize, stats: ValidationStats) {
        self.metrics.applied_changes = self.metrics.applied_changes.saturating_add(1);
        self.metrics.operations_visited = self
            .metrics
            .operations_visited
            .saturating_add(usize_to_u64(operation_count));
        self.metrics.nodes_validated = self
            .metrics
            .nodes_validated
            .saturating_add(usize_to_u64(stats.nodes));
        self.metrics.relationship_steps = self
            .metrics
            .relationship_steps
            .saturating_add(usize_to_u64(stats.relationship_steps));
        self.metrics.child_ids_copied = self
            .metrics
            .child_ids_copied
            .saturating_add(usize_to_u64(stats.child_ids_copied));
    }

    fn enter_recovery(&mut self, reason: RecoveryReason) -> ApplyOutcome {
        let document = self
            .document
            .as_ref()
            .expect("only initialized reducers enter recovery");
        let last_good = document.coordinate.clone();
        self.control = Control::NeedsSnapshot {
            last_good: last_good.clone(),
            last_digest: document.last_payload_digest.clone(),
            reason: reason.clone(),
        };
        ApplyOutcome::RecoveryRequired { last_good, reason }
    }
}

fn snapshot_matches_document(snapshot: &Snapshot, document: &Document) -> bool {
    snapshot.coordinate() == &document.coordinate
        && snapshot.last_payload_digest() == &document.last_payload_digest
        && snapshot.lifecycle() == document.lifecycle
        && snapshot.source() == document.source
        && snapshot.projection_cursor() == document.projection_cursor
        && snapshot.roots() == &document.roots
        && snapshot.nodes().len() == document.nodes.len()
        && snapshot
            .nodes()
            .iter()
            .zip(document.nodes.values())
            .all(|(incoming, retained)| incoming == retained)
        && snapshot.resources().len() == document.resources.len()
        && snapshot
            .resources()
            .iter()
            .zip(document.resources.values())
            .all(|(incoming, retained)| incoming == retained)
}

#[derive(Debug)]
enum StageFailure {
    Divergence(RecoveryReason),
    Invalid(ProtocolError),
}

impl From<ProtocolError> for StageFailure {
    fn from(error: ProtocolError) -> Self {
        Self::Invalid(error)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ValidationStats {
    nodes: usize,
    relationship_steps: usize,
    metadata_bytes: usize,
    structural_items: usize,
    child_ids_copied: usize,
}

struct StagedChange {
    nodes: BTreeMap<NodeId, Option<ContentNode>>,
    resources: BTreeMap<ResourceId, Option<SemanticResource>>,
    structures: BTreeMap<ChildListOwner, StructureEdit>,
    parents: BTreeMap<NodeId, Option<ChildListOwner>>,
    metadata_bytes: usize,
    structural_items: usize,
    resulting_cursor: SourceCursor,
    projection_cursor: SourceCursor,
    finish: bool,
    impact: ChangeImpact,
    stats: ValidationStats,
}

enum StructureEdit {
    Append {
        insert: Vec<NodeId>,
        new_version: crate::StructureVersion,
    },
    Replace(ChildList),
}

fn stage_document(
    document: &Document,
    change: &ChangeSet,
    limits: ProtocolLimits,
) -> Result<StagedChange, StageFailure> {
    if document.lifecycle == DocumentLifecycle::Finalized {
        return Err(ProtocolError::IllegalLifecycle(
            "a finalized document accepts only a new epoch".to_string(),
        )
        .into());
    }
    if change.source().expected_cursor != document.coordinate.source_cursor {
        return Err(StageFailure::Divergence(RecoveryReason::SourceDivergence));
    }
    let resulting_len = document
        .source
        .len()
        .checked_add(change.source().suffix.len())
        .ok_or(ProtocolError::CursorOverflow)?;
    if resulting_len > limits.max_source_bytes {
        return Err(ProtocolError::SourceTooLarge {
            limit: limits.max_source_bytes,
            actual: resulting_len,
        }
        .into());
    }
    let resulting_cursor =
        SourceCursor::new(u64::try_from(resulting_len).map_err(|_| ProtocolError::CursorOverflow)?);

    validate_direct_targets(change.operations())?;
    let mut nodes = BTreeMap::<NodeId, Option<ContentNode>>::new();
    let mut resources = BTreeMap::<ResourceId, Option<SemanticResource>>::new();
    let mut structures = BTreeMap::<ChildListOwner, StructureEdit>::new();
    let mut projection_cursor = document.projection_cursor;
    let mut finish = false;
    let mut staging_work_steps = 0usize;
    let mut child_ids_copied = 0usize;
    let mut stabilization_versions = BTreeMap::<NodeId, crate::NodeVersion>::new();
    let direct_node_targets = change
        .operations()
        .iter()
        .filter_map(ProjectionOp::node_target)
        .collect::<BTreeSet<_>>();
    let explicitly_replaced_nodes = change
        .operations()
        .iter()
        .filter_map(|operation| match operation {
            ProjectionOp::ReplaceNode { node_id, .. } => Some(*node_id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();

    for operation in change.operations() {
        match operation {
            ProjectionOp::AdvanceProjection {
                expected_cursor,
                new_cursor,
            } => {
                if *expected_cursor != document.projection_cursor {
                    return Err(StageFailure::Divergence(
                        RecoveryReason::ProjectionDivergence,
                    ));
                }
                if *new_cursor <= *expected_cursor {
                    return Err(ProtocolError::InvalidChange(
                        "projection cursor advances must move strictly forward".to_string(),
                    )
                    .into());
                }
                if *new_cursor > resulting_cursor {
                    return Err(ProtocolError::InvalidChange(
                        "projection cursor cannot exceed the resulting source cursor".to_string(),
                    )
                    .into());
                }
                crate::SourceRange::new(*new_cursor, *new_cursor)
                    .validate_parts(&document.source, &change.source().suffix)
                    .map_err(|_| {
                        ProtocolError::InvalidChange(
                            "projection cursor must be a canonical UTF-8 boundary".to_string(),
                        )
                    })?;
                projection_cursor = *new_cursor;
            }
            ProjectionOp::InsertNode { node } => {
                if view_node(node.id, document, &nodes).is_some() {
                    return Err(ProtocolError::DuplicateNode(node.id).into());
                }
                if !node.children.is_empty() {
                    return Err(ProtocolError::InvalidChange(
                        "inserted nodes must attach children through SpliceChildren".to_string(),
                    )
                    .into());
                }
                nodes.insert(node.id, Some(node.clone()));
            }
            ProjectionOp::ReplaceNode {
                node_id,
                expected_version,
                projection,
            } => {
                let current = view_node(*node_id, document, &nodes)
                    .ok_or(StageFailure::Divergence(RecoveryReason::VersionDivergence))?;
                if &current.version != expected_version {
                    return Err(StageFailure::Divergence(RecoveryReason::VersionDivergence));
                }
                if current.stability == NodeStability::Stable
                    && projection.stability == NodeStability::Provisional
                {
                    return Err(ProtocolError::IllegalLifecycle(
                        "stable nodes cannot become provisional".to_string(),
                    )
                    .into());
                }
                if current.stability == NodeStability::Stable
                    && stable_table_width_changed(&current.content, &projection.content)
                {
                    return Err(ProtocolError::InvalidChange(
                        "stable table column counts cannot change".to_string(),
                    )
                    .into());
                }
                if projection == &current.projection() {
                    return Err(ProtocolError::InvalidChange(
                        "replacement must change the local projection".to_string(),
                    )
                    .into());
                }
                if projection.version == current.version {
                    return Err(ProtocolError::InvalidChange(
                        "changed projections require a new node version".to_string(),
                    )
                    .into());
                }
                nodes.insert(
                    *node_id,
                    Some(ContentNode::from_projection(
                        *node_id,
                        projection.clone(),
                        current.children.clone_shared(),
                    )),
                );
            }
            ProjectionOp::StabilizeNode {
                node_id,
                expected_version,
                new_version,
            } => {
                let current = view_node(*node_id, document, &nodes)
                    .ok_or(StageFailure::Divergence(RecoveryReason::VersionDivergence))?;
                if &current.version != expected_version {
                    return Err(StageFailure::Divergence(RecoveryReason::VersionDivergence));
                }
                if current.stability == NodeStability::Stable {
                    return Err(ProtocolError::IllegalLifecycle(
                        "a stable node cannot be stabilized again".to_string(),
                    )
                    .into());
                }
                let mut node = current.clone_shared();
                node.stability = NodeStability::Stable;
                let derived = node.derived_version();
                node.version = derived;
                nodes.insert(*node_id, Some(node));
                stabilization_versions.insert(*node_id, new_version.clone());
            }
            ProjectionOp::RemoveNode {
                node_id,
                expected_version,
            } => {
                let current = view_node(*node_id, document, &nodes)
                    .ok_or(StageFailure::Divergence(RecoveryReason::VersionDivergence))?;
                if &current.version != expected_version {
                    return Err(StageFailure::Divergence(RecoveryReason::VersionDivergence));
                }
                let subtree = collect_subtree(*node_id, document, &nodes, &structures)?;
                staging_work_steps = staging_work_steps.saturating_add(subtree.len());
                if subtree
                    .iter()
                    .any(|id| *id != *node_id && direct_node_targets.contains(id))
                {
                    return Err(ProtocolError::InvalidChange(
                        "an operation cannot target a node inside a removed subtree".to_string(),
                    )
                    .into());
                }
                for id in subtree {
                    nodes.insert(id, None);
                }
            }
            ProjectionOp::SpliceChildren {
                owner,
                expected_version,
                start,
                delete_count,
                insert,
                new_version,
            } => {
                let current = view_child_list(*owner, document, &nodes).ok_or_else(|| {
                    ProtocolError::InvalidChange("missing child-list owner".into())
                })?;
                if current.version() != expected_version {
                    return Err(StageFailure::Divergence(
                        RecoveryReason::StructureDivergence,
                    ));
                }
                let start = usize::try_from(*start).map_err(|_| {
                    ProtocolError::InvalidChange("splice start does not fit usize".to_string())
                })?;
                let delete_count = usize::try_from(*delete_count).map_err(|_| {
                    ProtocolError::InvalidChange(
                        "splice delete_count does not fit usize".to_string(),
                    )
                })?;
                let end = start.checked_add(delete_count).ok_or_else(|| {
                    ProtocolError::InvalidChange("splice range overflow".to_string())
                })?;
                if start > current.len() || end > current.len() {
                    return Err(ProtocolError::InvalidChange(
                        "splice range exceeds the current child list".to_string(),
                    )
                    .into());
                }
                let resulting_count = current
                    .len()
                    .checked_sub(delete_count)
                    .and_then(|count| count.checked_add(insert.len()))
                    .ok_or_else(|| {
                        ProtocolError::InvalidChange("child-list size overflow".to_string())
                    })?;
                if resulting_count > limits.max_children_per_list {
                    return Err(ProtocolError::ValueTooLarge {
                        field: "child_list.children",
                        limit: limits.max_children_per_list,
                        actual: resulting_count,
                    }
                    .into());
                }
                if delete_count == 0 && insert.is_empty() {
                    return Err(ProtocolError::InvalidChange(
                        "SpliceChildren must change the ordered child list".to_string(),
                    )
                    .into());
                }
                let edit = if start == current.len() && delete_count == 0 {
                    let derived = current.version_after_append(insert);
                    if &derived != new_version {
                        return Err(ProtocolError::InvalidChange(
                            "splice new_version does not match the resulting child list"
                                .to_string(),
                        )
                        .into());
                    }
                    child_ids_copied = child_ids_copied.saturating_add(insert.len());
                    StructureEdit::Append {
                        insert: insert.clone(),
                        new_version: derived,
                    }
                } else {
                    child_ids_copied = child_ids_copied
                        .saturating_add(current.len())
                        .saturating_add(insert.len());
                    let mut children = current.as_slice().to_vec();
                    children.splice(start..end, insert.iter().copied());
                    let replacement = ChildList::new(children);
                    if &replacement == current {
                        return Err(ProtocolError::InvalidChange(
                            "SpliceChildren must change the ordered child list".to_string(),
                        )
                        .into());
                    }
                    if replacement.version() != new_version {
                        return Err(ProtocolError::InvalidChange(
                            "splice new_version does not match the resulting child list"
                                .to_string(),
                        )
                        .into());
                    }
                    StructureEdit::Replace(replacement)
                };
                structures.insert(*owner, edit);
            }
            ProjectionOp::InsertResource { resource } => {
                if view_resource(resource.id, document, &resources).is_some() {
                    return Err(ProtocolError::DuplicateResource(resource.id).into());
                }
                resources.insert(resource.id, Some(resource.clone()));
            }
            ProjectionOp::ReplaceResource {
                resource_id,
                expected_version,
                resource,
            } => {
                let current = view_resource(*resource_id, document, &resources)
                    .ok_or(StageFailure::Divergence(RecoveryReason::ResourceDivergence))?;
                if &current.version != expected_version {
                    return Err(StageFailure::Divergence(RecoveryReason::ResourceDivergence));
                }
                if resource.id != *resource_id {
                    return Err(ProtocolError::InvalidChange(
                        "resource replacement must preserve identity".to_string(),
                    )
                    .into());
                }
                if resource == current {
                    return Err(ProtocolError::InvalidChange(
                        "resource replacement must change content".to_string(),
                    )
                    .into());
                }
                validate_resource_identity(current, resource)?;
                resources.insert(*resource_id, Some(resource.clone()));
            }
            ProjectionOp::RemoveResource {
                resource_id,
                expected_version,
            } => {
                let current = view_resource(*resource_id, document, &resources)
                    .ok_or(StageFailure::Divergence(RecoveryReason::ResourceDivergence))?;
                if &current.version != expected_version {
                    return Err(StageFailure::Divergence(RecoveryReason::ResourceDivergence));
                }
                resources.insert(*resource_id, None);
            }
            ProjectionOp::FinishDocument => finish = true,
        }
    }

    staging_work_steps = staging_work_steps.saturating_add(rebind_replaced_resource_users(
        document,
        &mut nodes,
        &resources,
        &explicitly_replaced_nodes,
    )?);
    for (node_id, expected) in stabilization_versions {
        let node =
            view_node(node_id, document, &nodes).ok_or(ProtocolError::MissingNode(node_id))?;
        if node.version != expected {
            return Err(ProtocolError::VersionMismatch(node_id).into());
        }
    }

    let node_count = resulting_count(document.nodes.len(), &nodes, |id| {
        document.nodes.contains_key(id)
    })?;
    if node_count > limits.max_nodes {
        return Err(ProtocolError::TooManyNodes {
            limit: limits.max_nodes,
            actual: node_count,
        }
        .into());
    }
    let resource_count = resulting_count(document.resources.len(), &resources, |id| {
        document.resources.contains_key(id)
    })?;
    if resource_count > limits.max_resources {
        return Err(ProtocolError::ValueTooLarge {
            field: "document.resources",
            limit: limits.max_resources,
            actual: resource_count,
        }
        .into());
    }

    let mut structural_items = document.structural_items;
    for (id, replacement) in &nodes {
        if let Some(current) = document.nodes.get(id) {
            structural_items = structural_items
                .checked_sub(node_structural_items(current)?)
                .ok_or(ProtocolError::MetadataOverflow)?;
        }
        if let Some(node) = replacement {
            structural_items = structural_items
                .checked_add(node_structural_items(node)?)
                .ok_or(ProtocolError::MetadataOverflow)?;
        }
    }
    if structural_items > limits.max_document_structural_items {
        return Err(ProtocolError::ValueTooLarge {
            field: "document.structural_items",
            limit: limits.max_document_structural_items,
            actual: structural_items,
        }
        .into());
    }

    let mut metadata_bytes = document.metadata_bytes;
    let mut provisional_node_count = document.provisional_nodes.len();
    let mut nodes_validated = 0usize;
    for (id, replacement) in &nodes {
        if let Some(current) = document.nodes.get(id) {
            metadata_bytes = metadata_bytes
                .checked_sub(current.projection().validate_shape(*id, limits)?)
                .ok_or(ProtocolError::MetadataOverflow)?;
        }
        if document.provisional_nodes.contains(id) {
            provisional_node_count = provisional_node_count
                .checked_sub(1)
                .ok_or(ProtocolError::MetadataOverflow)?;
        }
        if let Some(node) = replacement {
            validate_change_projection_coverage(node, projection_cursor)?;
            let bytes = if document.nodes.contains_key(id) {
                node.projection().validate_local_parts(
                    node.id,
                    &document.source,
                    &change.source().suffix,
                    limits,
                )?
            } else {
                node.validate_local_parts(&document.source, &change.source().suffix, limits)?
            };
            metadata_bytes = metadata_bytes
                .checked_add(bytes)
                .ok_or(ProtocolError::MetadataOverflow)?;
            nodes_validated = nodes_validated.saturating_add(1);
            if node.stability == NodeStability::Provisional {
                provisional_node_count = provisional_node_count
                    .checked_add(1)
                    .ok_or(ProtocolError::MetadataOverflow)?;
            }
        }
    }
    for (id, replacement) in &resources {
        if let Some(current) = document.resources.get(id) {
            metadata_bytes = metadata_bytes
                .checked_sub(current.validate_local(limits)?)
                .ok_or(ProtocolError::MetadataOverflow)?;
        }
        if let Some(resource) = replacement {
            metadata_bytes = metadata_bytes
                .checked_add(resource.validate_local(limits)?)
                .ok_or(ProtocolError::MetadataOverflow)?;
        }
    }
    if metadata_bytes > limits.max_document_metadata_bytes {
        return Err(ProtocolError::ValueTooLarge {
            field: "document.metadata",
            limit: limits.max_document_metadata_bytes,
            actual: metadata_bytes,
        }
        .into());
    }

    let ParentStage {
        changes: parents,
        affected_owners,
        full_validation_owners,
    } = stage_parent_index(document, &nodes, &structures)?;
    let mut relationship_steps = staging_work_steps;
    let forest_view = StagedForestView {
        document,
        nodes: &nodes,
        structures: &structures,
        parents: &parents,
        resulting_cursor,
        limits,
    };
    for owner in &affected_owners {
        relationship_steps = relationship_steps.saturating_add(validate_child_list_view(
            *owner,
            &forest_view,
            full_validation_owners.contains(owner),
        )?);
    }
    for (node_id, replacement) in &nodes {
        let Some(node) = replacement else {
            continue;
        };
        let owner = final_parent(*node_id, document, &parents).ok_or_else(|| {
            ProtocolError::InvalidChange(
                "every live node must have exactly one root or parent owner".to_string(),
            )
        })?;
        let owner_content = match owner {
            ChildListOwner::Document => None,
            ChildListOwner::Node { node_id: parent_id } => Some(
                &view_node(parent_id, document, &nodes)
                    .ok_or(ProtocolError::MissingNode(parent_id))?
                    .content,
            ),
        };
        validate_child_kind(owner_content, &node.content)?;
        relationship_steps = relationship_steps.saturating_add(1);
    }

    let mut depth_seeds = BTreeSet::new();
    depth_seeds.extend(nodes.keys().copied());
    depth_seeds.extend(parents.keys().copied());
    for owner in &affected_owners {
        if let ChildListOwner::Node { node_id } = owner {
            depth_seeds.insert(*node_id);
        }
    }
    let moved = parents
        .iter()
        .filter_map(|(id, parent)| {
            parent
                .filter(|parent| document.parents.get(id).copied() != Some(*parent))
                .map(|_| *id)
        })
        .collect::<BTreeSet<_>>();
    for root in moved.iter().copied().filter(|id| {
        !matches!(
            final_parent(*id, document, &parents),
            Some(ChildListOwner::Node { node_id }) if moved.contains(&node_id)
        )
    }) {
        relationship_steps = relationship_steps.saturating_add(collect_final_subtree_seeds(
            root,
            document,
            &nodes,
            &structures,
            &mut depth_seeds,
        )?);
    }
    for (node_id, replacement) in &nodes {
        let (Some(current), Some(replacement)) = (document.nodes.get(node_id), replacement) else {
            continue;
        };
        if forest_context_contract_changed(&current.content, &replacement.content) {
            relationship_steps = relationship_steps.saturating_add(collect_final_subtree_seeds(
                *node_id,
                document,
                &nodes,
                &structures,
                &mut depth_seeds,
            )?);
        }
    }
    relationship_steps = relationship_steps.saturating_add(validate_depths_incremental(
        depth_seeds,
        document,
        &nodes,
        &structures,
        &parents,
        limits.max_tree_depth,
    )?);

    relationship_steps = relationship_steps
        .saturating_add(validate_resource_references(document, &nodes, &resources)?);

    if finish {
        if projection_cursor != resulting_cursor {
            return Err(ProtocolError::IllegalLifecycle(
                "finalized documents require projection coverage through the source cursor"
                    .to_string(),
            )
            .into());
        }
        if provisional_node_count != 0 {
            return Err(ProtocolError::IllegalLifecycle(
                "finalized documents cannot contain provisional nodes".to_string(),
            )
            .into());
        }
    }

    let mut impact = staged_impact(document, &nodes, &resources, &structures, &parents);
    impact.source_changed = !change.source().suffix.is_empty();
    impact.projection_changed = projection_cursor != document.projection_cursor;
    impact.lifecycle_changed = finish;
    Ok(StagedChange {
        nodes,
        resources,
        structures,
        parents,
        metadata_bytes,
        structural_items,
        resulting_cursor,
        projection_cursor,
        finish,
        impact,
        stats: ValidationStats {
            nodes: nodes_validated,
            relationship_steps,
            metadata_bytes,
            structural_items,
            child_ids_copied,
        },
    })
}

fn rebind_replaced_resource_users(
    document: &Document,
    staged_nodes: &mut BTreeMap<NodeId, Option<ContentNode>>,
    staged_resources: &BTreeMap<ResourceId, Option<SemanticResource>>,
    explicitly_replaced_nodes: &BTreeSet<NodeId>,
) -> Result<usize, StageFailure> {
    let mut steps = 0usize;
    for (resource_id, replacement) in staged_resources {
        let Some(replacement) = replacement else {
            continue;
        };
        if !document.resources.contains_key(resource_id) {
            continue;
        }
        let Some(users) = document.resource_users.get(resource_id) else {
            continue;
        };
        for node_id in users {
            steps = steps.saturating_add(1);
            if explicitly_replaced_nodes.contains(node_id)
                || staged_nodes.get(node_id).is_some_and(Option::is_none)
            {
                continue;
            }
            let current = view_node(*node_id, document, staged_nodes)
                .ok_or(ProtocolError::MissingNode(*node_id))?;
            let mut rebound = current.clone_shared();
            let reference = rebound.content.resource_ref_mut().ok_or_else(|| {
                ProtocolError::InvalidChange(
                    "resource dependency index references a node without a resource".to_string(),
                )
            })?;
            if reference.id != *resource_id {
                return Err(ProtocolError::InvalidChange(
                    "resource dependency index references the wrong resource".to_string(),
                )
                .into());
            }
            reference.version = replacement.version.clone();
            rebound.version = rebound.derived_version();
            staged_nodes.insert(*node_id, Some(rebound));
        }
    }
    Ok(steps)
}

fn collect_final_subtree_seeds(
    root: NodeId,
    document: &Document,
    staged: &BTreeMap<NodeId, Option<ContentNode>>,
    structures: &BTreeMap<ChildListOwner, StructureEdit>,
    seeds: &mut BTreeSet<NodeId>,
) -> Result<usize, StageFailure> {
    let mut pending = vec![root];
    let mut seen = BTreeSet::new();
    let mut steps = 0usize;
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            return Err(
                ProtocolError::InvalidChange("node graph contains a cycle".to_string()).into(),
            );
        }
        let node = view_node(id, document, staged).ok_or(ProtocolError::MissingNode(id))?;
        seeds.insert(id);
        steps = steps.saturating_add(1);
        let owner = ChildListOwner::Node { node_id: id };
        match structures.get(&owner) {
            Some(StructureEdit::Append { insert, .. }) => {
                pending.extend(node.children.iter().copied());
                pending.extend(insert.iter().copied());
            }
            Some(StructureEdit::Replace(replacement)) => {
                pending.extend(replacement.iter().copied());
            }
            None => pending.extend(node.children.iter().copied()),
        }
    }
    Ok(steps)
}

fn validate_direct_targets(operations: &[ProjectionOp]) -> Result<(), StageFailure> {
    let mut nodes = BTreeSet::new();
    let mut resources = BTreeSet::new();
    let mut structures = BTreeSet::new();
    let mut projection_advance = false;
    let mut finish = false;
    for operation in operations {
        if let Some(target) = operation.node_target() {
            if !nodes.insert(target) {
                return Err(ProtocolError::DuplicateNode(target).into());
            }
        }
        if let Some(target) = operation.resource_target() {
            if !resources.insert(target) {
                return Err(ProtocolError::DuplicateResource(target).into());
            }
        }
        if let Some(target) = operation.structure_target() {
            if !structures.insert(target) {
                return Err(ProtocolError::InvalidChange(
                    "a change may splice each child-list owner at most once".to_string(),
                )
                .into());
            }
        }
        if matches!(operation, ProjectionOp::AdvanceProjection { .. }) {
            if projection_advance {
                return Err(ProtocolError::InvalidChange(
                    "projection cursor may advance at most once per change".to_string(),
                )
                .into());
            }
            projection_advance = true;
        }
        if matches!(operation, ProjectionOp::FinishDocument) {
            if finish {
                return Err(ProtocolError::InvalidChange(
                    "finish may appear at most once".to_string(),
                )
                .into());
            }
            finish = true;
        }
    }
    Ok(())
}

fn validate_change_projection_coverage(
    node: &ContentNode,
    projection_cursor: SourceCursor,
) -> Result<(), StageFailure> {
    if node.source.end > projection_cursor || node.body.end > projection_cursor {
        return Err(ProtocolError::InvalidChange(format!(
            "node {} source and body ranges must end at or before projection cursor {}",
            node.id, projection_cursor
        ))
        .into());
    }
    Ok(())
}

fn resulting_count<K: Ord, V, F>(
    current: usize,
    overlay: &BTreeMap<K, Option<V>>,
    exists: F,
) -> Result<usize, ProtocolError>
where
    F: Fn(&K) -> bool,
{
    let inserted = overlay
        .iter()
        .filter(|(id, value)| value.is_some() && !exists(id))
        .count();
    let removed = overlay
        .iter()
        .filter(|(id, value)| value.is_none() && exists(id))
        .count();
    current
        .checked_add(inserted)
        .and_then(|count| count.checked_sub(removed))
        .ok_or_else(|| ProtocolError::InvalidChange("collection count underflow".to_string()))
}

fn collect_subtree(
    root: NodeId,
    document: &Document,
    staged: &BTreeMap<NodeId, Option<ContentNode>>,
    structures: &BTreeMap<ChildListOwner, StructureEdit>,
) -> Result<Vec<NodeId>, StageFailure> {
    let mut result = Vec::new();
    let mut pending = vec![root];
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            return Err(
                ProtocolError::InvalidChange("node graph contains a cycle".to_string()).into(),
            );
        }
        let node = view_node(id, document, staged).ok_or(ProtocolError::MissingNode(id))?;
        result.push(id);
        match structures.get(&ChildListOwner::Node { node_id: id }) {
            Some(StructureEdit::Append { insert, .. }) => {
                pending.extend(node.children.iter().copied());
                pending.extend(insert.iter().copied());
            }
            Some(StructureEdit::Replace(replacement)) => {
                pending.extend(replacement.iter().copied());
            }
            None => pending.extend(node.children.iter().copied()),
        }
    }
    Ok(result)
}

fn view_node<'a>(
    id: NodeId,
    document: &'a Document,
    staged: &'a BTreeMap<NodeId, Option<ContentNode>>,
) -> Option<&'a ContentNode> {
    match staged.get(&id) {
        Some(Some(node)) => Some(node),
        Some(None) => None,
        None => document.nodes.get(&id),
    }
}

fn view_resource<'a>(
    id: ResourceId,
    document: &'a Document,
    staged: &'a BTreeMap<ResourceId, Option<SemanticResource>>,
) -> Option<&'a SemanticResource> {
    match staged.get(&id) {
        Some(Some(resource)) => Some(resource),
        Some(None) => None,
        None => document.resources.get(&id),
    }
}

fn view_child_list<'a>(
    owner: ChildListOwner,
    document: &'a Document,
    staged: &'a BTreeMap<NodeId, Option<ContentNode>>,
) -> Option<&'a ChildList> {
    match owner {
        ChildListOwner::Document => Some(&document.roots),
        ChildListOwner::Node { node_id } => {
            view_node(node_id, document, staged).map(|node| &node.children)
        }
    }
}

struct ParentStage {
    changes: BTreeMap<NodeId, Option<ChildListOwner>>,
    affected_owners: BTreeSet<ChildListOwner>,
    full_validation_owners: BTreeSet<ChildListOwner>,
}

fn stage_parent_index(
    document: &Document,
    staged: &BTreeMap<NodeId, Option<ContentNode>>,
    structures: &BTreeMap<ChildListOwner, StructureEdit>,
) -> Result<ParentStage, StageFailure> {
    let mut changed_owners = structures.keys().copied().collect::<BTreeSet<_>>();
    let mut full_validation_owners = structures
        .iter()
        .filter_map(|(owner, edit)| matches!(edit, StructureEdit::Replace(_)).then_some(*owner))
        .collect::<BTreeSet<_>>();
    for (id, replacement) in staged {
        match (document.nodes.get(id), replacement) {
            (Some(current), Some(next)) => {
                if current.source != next.source {
                    if let Some(parent) = document.parents.get(id) {
                        changed_owners.insert(*parent);
                        full_validation_owners.insert(*parent);
                    }
                }
                if current.content != next.content
                    && (affects_parent_sequence(&current.content)
                        || affects_parent_sequence(&next.content))
                {
                    if let Some(parent) = document.parents.get(id) {
                        changed_owners.insert(*parent);
                        full_validation_owners.insert(*parent);
                    }
                }
                if current.body != next.body
                    || child_list_contract_changed(&current.content, &next.content)
                    || (current.stability != next.stability
                        && requires_sequence_completeness(&next.content))
                {
                    let owner = ChildListOwner::Node { node_id: *id };
                    changed_owners.insert(owner);
                    full_validation_owners.insert(owner);
                }
            }
            (Some(_), None) => {
                if let Some(parent) = document.parents.get(id) {
                    changed_owners.insert(*parent);
                    full_validation_owners.insert(*parent);
                }
            }
            (None, Some(next))
                if !next.children.is_empty() || requires_sequence_completeness(&next.content) =>
            {
                let owner = ChildListOwner::Node { node_id: *id };
                changed_owners.insert(owner);
                full_validation_owners.insert(owner);
            }
            _ => {}
        }
    }

    let mut detached = BTreeMap::<NodeId, ChildListOwner>::new();
    let mut attached = BTreeMap::<NodeId, ChildListOwner>::new();
    for owner in &changed_owners {
        match structures.get(owner) {
            Some(StructureEdit::Append { insert, .. }) => {
                for child in insert {
                    if attached.insert(*child, *owner).is_some() {
                        return Err(ProtocolError::InvalidChange(
                            "a node cannot have more than one owner".to_string(),
                        )
                        .into());
                    }
                }
            }
            Some(StructureEdit::Replace(new)) => {
                let old = match owner {
                    ChildListOwner::Document => Some(&document.roots),
                    ChildListOwner::Node { node_id } => document
                        .nodes
                        .get(node_id)
                        .map(|node| &node.children)
                        .or_else(|| {
                            view_node(*node_id, document, staged).map(|node| &node.children)
                        }),
                };
                let old_ids = old
                    .map(|list| list.iter().copied().collect::<BTreeSet<_>>())
                    .unwrap_or_default();
                let new_ids = new.iter().copied().collect::<BTreeSet<_>>();
                for child in old_ids.difference(&new_ids) {
                    detached.insert(*child, *owner);
                }
                for child in new_ids.difference(&old_ids) {
                    if attached.insert(*child, *owner).is_some() {
                        return Err(ProtocolError::InvalidChange(
                            "a node cannot have more than one owner".to_string(),
                        )
                        .into());
                    }
                }
            }
            None => {}
        }
    }

    let mut changes = BTreeMap::<NodeId, Option<ChildListOwner>>::new();
    for id in staged
        .iter()
        .filter_map(|(id, node)| node.is_none().then_some(*id))
    {
        changes.insert(id, None);
    }
    for (child, owner) in detached {
        if view_node(child, document, staged).is_some() {
            changes.insert(child, None);
        }
        if document
            .parents
            .get(&child)
            .is_some_and(|current| current != &owner)
        {
            return Err(ProtocolError::InvalidChange(
                "child-list detach does not match the current owner".to_string(),
            )
            .into());
        }
    }
    for (child, owner) in attached {
        if view_node(child, document, staged).is_none() {
            return Err(ProtocolError::MissingNode(child).into());
        }
        let current = match changes.get(&child) {
            Some(parent) => *parent,
            None => document.parents.get(&child).copied(),
        };
        if current.is_some_and(|current| current != owner) {
            return Err(ProtocolError::InvalidChange(
                "a node cannot have more than one owner".to_string(),
            )
            .into());
        }
        changes.insert(child, Some(owner));
    }

    for id in staged
        .iter()
        .filter_map(|(id, node)| node.as_ref().map(|_| *id))
        .chain(changes.keys().copied())
    {
        if view_node(id, document, staged).is_some()
            && final_parent(id, document, &changes).is_none()
        {
            return Err(ProtocolError::InvalidChange(
                "every live node must have exactly one root or parent owner".to_string(),
            )
            .into());
        }
    }
    Ok(ParentStage {
        changes,
        affected_owners: changed_owners,
        full_validation_owners,
    })
}

fn final_parent(
    id: NodeId,
    document: &Document,
    changes: &BTreeMap<NodeId, Option<ChildListOwner>>,
) -> Option<ChildListOwner> {
    match changes.get(&id) {
        Some(parent) => *parent,
        None => document.parents.get(&id).copied(),
    }
}

struct StagedForestView<'a> {
    document: &'a Document,
    nodes: &'a BTreeMap<NodeId, Option<ContentNode>>,
    structures: &'a BTreeMap<ChildListOwner, StructureEdit>,
    parents: &'a BTreeMap<NodeId, Option<ChildListOwner>>,
    resulting_cursor: SourceCursor,
    limits: ProtocolLimits,
}

fn validate_child_list_view(
    owner: ChildListOwner,
    view: &StagedForestView<'_>,
    force_full: bool,
) -> Result<usize, StageFailure> {
    let document = view.document;
    let staged = view.nodes;
    let structures = view.structures;
    let parent_changes = view.parents;
    let resulting_cursor = view.resulting_cursor;
    let limits = view.limits;
    let Some(current) = view_child_list(owner, document, staged) else {
        return Ok(0);
    };
    let owner_range = match owner {
        ChildListOwner::Document => crate::SourceRange::new(SourceCursor::new(0), resulting_cursor),
        ChildListOwner::Node { node_id } => {
            view_node(node_id, document, staged)
                .ok_or(ProtocolError::MissingNode(node_id))?
                .body
        }
    };
    match structures.get(&owner) {
        Some(StructureEdit::Append { insert, .. }) => {
            let resulting_count = current.len().checked_add(insert.len()).ok_or_else(|| {
                ProtocolError::InvalidChange("child-list size overflow".to_string())
            })?;
            if resulting_count > limits.max_children_per_list {
                return Err(ProtocolError::ValueTooLarge {
                    field: "child_list.children",
                    limit: limits.max_children_per_list,
                    actual: resulting_count,
                }
                .into());
            }
            let mut unique = BTreeSet::new();
            for child in insert {
                if !unique.insert(*child) || document.parents.get(child) == Some(&owner) {
                    return Err(ProtocolError::DuplicateNode(*child).into());
                }
            }
            if force_full {
                validate_child_slice(
                    owner,
                    current.as_slice(),
                    Some(insert),
                    None,
                    owner_range,
                    document,
                    staged,
                    parent_changes,
                    0,
                    None,
                )
            } else {
                let previous = current
                    .as_slice()
                    .last()
                    .and_then(|id| view_node(*id, document, staged))
                    .map(|node| node.source.end);
                let last_kind = current
                    .as_slice()
                    .last()
                    .and_then(|id| view_node(*id, document, staged))
                    .map(|node| &node.content);
                validate_child_slice(
                    owner,
                    insert,
                    None,
                    previous,
                    owner_range,
                    document,
                    staged,
                    parent_changes,
                    current.len(),
                    last_kind,
                )
            }
        }
        Some(StructureEdit::Replace(replacement)) => {
            replacement.validate_local(limits)?;
            validate_child_slice(
                owner,
                replacement.as_slice(),
                None,
                None,
                owner_range,
                document,
                staged,
                parent_changes,
                0,
                None,
            )
        }
        None => {
            current.validate_local(limits)?;
            validate_child_slice(
                owner,
                current.as_slice(),
                None,
                None,
                owner_range,
                document,
                staged,
                parent_changes,
                0,
                None,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_child_slice(
    owner: ChildListOwner,
    children: &[NodeId],
    additional_children: Option<&[NodeId]>,
    mut previous: Option<SourceCursor>,
    owner_range: crate::SourceRange,
    document: &Document,
    staged: &BTreeMap<NodeId, Option<ContentNode>>,
    parent_changes: &BTreeMap<NodeId, Option<ChildListOwner>>,
    sequence_prefix_len: usize,
    sequence_last_kind: Option<&ContentKind>,
) -> Result<usize, StageFailure> {
    let owner_content = match owner {
        ChildListOwner::Document => None,
        ChildListOwner::Node { node_id } => Some(
            &view_node(node_id, document, staged)
                .ok_or(ProtocolError::MissingNode(node_id))?
                .content,
        ),
    };
    let mut sequence =
        ChildSequenceValidator::resume(owner_content, sequence_prefix_len, sequence_last_kind)?;
    let mut steps = 1usize;
    for child_id in children
        .iter()
        .chain(additional_children.unwrap_or_default())
    {
        let child =
            view_node(*child_id, document, staged).ok_or(ProtocolError::MissingNode(*child_id))?;
        validate_child_kind(owner_content, &child.content)?;
        sequence.push(&child.content)?;
        if final_parent(*child_id, document, parent_changes) != Some(owner) {
            return Err(ProtocolError::InvalidChange(
                "child ownership is not canonical".to_string(),
            )
            .into());
        }
        if !owner_range.contains(child.source) {
            return Err(ProtocolError::InvalidChange(
                "child source range must be contained by its owner body".to_string(),
            )
            .into());
        }
        if previous.is_some_and(|end| end > child.source.start) {
            return Err(ProtocolError::InvalidChange(
                "siblings must be ordered and non-overlapping".to_string(),
            )
            .into());
        }
        previous = Some(child.source.end);
        steps = steps.saturating_add(1);
    }
    let completeness = match owner {
        ChildListOwner::Document => ChildSequenceCompleteness::Complete,
        ChildListOwner::Node { node_id } => {
            let owner =
                view_node(node_id, document, staged).ok_or(ProtocolError::MissingNode(node_id))?;
            sequence_completeness(owner.stability)
        }
    };
    sequence.finish(completeness)?;
    Ok(steps)
}

fn validate_depths_incremental(
    seeds: BTreeSet<NodeId>,
    document: &Document,
    staged: &BTreeMap<NodeId, Option<ContentNode>>,
    structures: &BTreeMap<ChildListOwner, StructureEdit>,
    parent_changes: &BTreeMap<NodeId, Option<ChildListOwner>>,
    max_depth: usize,
) -> Result<usize, StageFailure> {
    let mut completed = BTreeMap::<NodeId, ForestPathState>::new();
    let mut steps = 0usize;
    for seed in seeds {
        if view_node(seed, document, staged).is_none() || completed.contains_key(&seed) {
            continue;
        }
        let mut path = Vec::new();
        let mut positions = BTreeMap::new();
        let mut current = seed;
        let base = loop {
            if let Some(state) = completed.get(&current) {
                break *state;
            }
            if positions.insert(current, path.len()).is_some() {
                return Err(ProtocolError::InvalidChange(
                    "node graph contains a cycle".to_string(),
                )
                .into());
            }
            if view_node(current, document, staged).is_none() {
                return Err(ProtocolError::MissingNode(current).into());
            }
            path.push(current);
            steps = steps.saturating_add(1);
            match final_parent(current, document, parent_changes) {
                Some(ChildListOwner::Document) => break ForestPathState::default(),
                Some(ChildListOwner::Node { node_id }) => current = node_id,
                None => {
                    return Err(ProtocolError::InvalidChange(
                        "live node has no canonical owner".to_string(),
                    )
                    .into());
                }
            }
        };
        let mut state = base;
        for id in path.into_iter().rev() {
            state.depth = state.depth.saturating_add(1);
            if state.depth > max_depth {
                return Err(ProtocolError::ValueTooLarge {
                    field: "tree.depth",
                    limit: max_depth,
                    actual: state.depth,
                }
                .into());
            }
            let node = view_node(id, document, staged).ok_or(ProtocolError::MissingNode(id))?;
            let child_count = final_child_count(id, node, structures)?;
            advance_forest_context(&mut state, &node.content, node.stability, child_count)?;
            completed.insert(id, state);
        }
    }
    Ok(steps)
}

#[derive(Debug, Clone, Copy, Default)]
struct ForestPathState {
    depth: usize,
    link_seen: bool,
    table_columns: Option<usize>,
}

fn advance_forest_context(
    state: &mut ForestPathState,
    content: &ContentKind,
    stability: NodeStability,
    child_count: usize,
) -> Result<(), ProtocolError> {
    match content {
        ContentKind::Link { .. } => {
            if state.link_seen {
                return Err(ProtocolError::InvalidChange(
                    "links cannot contain nested links through phrasing descendants".to_string(),
                ));
            }
            state.link_seen = true;
        }
        ContentKind::Table { alignments } => state.table_columns = Some(alignments.len()),
        ContentKind::TableRow {} => {
            let columns = state.table_columns.ok_or_else(|| {
                ProtocolError::InvalidChange(
                    "table rows require a canonical table ancestor".to_string(),
                )
            })?;
            validate_table_row_width(child_count, columns, sequence_completeness(stability))?;
        }
        ContentKind::TableCell { column }
            if state.table_columns.is_none_or(|columns| {
                usize::try_from(*column).map_or(true, |column| column >= columns)
            }) =>
        {
            return Err(ProtocolError::InvalidChange(
                "table cell column exceeds its table alignment schema".to_string(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn final_child_count(
    id: NodeId,
    node: &ContentNode,
    structures: &BTreeMap<ChildListOwner, StructureEdit>,
) -> Result<usize, ProtocolError> {
    match structures.get(&ChildListOwner::Node { node_id: id }) {
        Some(StructureEdit::Append { insert, .. }) => node
            .children
            .len()
            .checked_add(insert.len())
            .ok_or_else(|| ProtocolError::InvalidChange("child-list size overflow".to_string())),
        Some(StructureEdit::Replace(replacement)) => Ok(replacement.len()),
        None => Ok(node.children.len()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildListContract {
    Leaf,
    Phrasing,
    Link,
    List,
    RootBlocks,
    Table,
    TableHead,
    TableBody,
    TableRow,
    Any,
}

fn child_list_contract(content: &ContentKind) -> ChildListContract {
    match content {
        ContentKind::Paragraph {}
        | ContentKind::Heading { .. }
        | ContentKind::Emphasis {}
        | ContentKind::Strong {}
        | ContentKind::Strikethrough {}
        | ContentKind::TableCell { .. } => ChildListContract::Phrasing,
        ContentKind::Link { .. } => ChildListContract::Link,
        ContentKind::List { .. } => ChildListContract::List,
        ContentKind::ListItem { .. }
        | ContentKind::BlockQuote { .. }
        | ContentKind::FootnoteDefinition { .. } => ChildListContract::RootBlocks,
        ContentKind::Table { .. } => ChildListContract::Table,
        ContentKind::TableHead {} => ChildListContract::TableHead,
        ContentKind::TableBody {} => ChildListContract::TableBody,
        ContentKind::TableRow {} => ChildListContract::TableRow,
        ContentKind::Custom { opaque: false, .. } => ChildListContract::Any,
        _ => ChildListContract::Leaf,
    }
}

fn child_list_contract_changed(current: &ContentKind, next: &ContentKind) -> bool {
    child_list_contract(current) != child_list_contract(next)
}

fn requires_sequence_completeness(content: &ContentKind) -> bool {
    matches!(
        content,
        ContentKind::Table { .. } | ContentKind::TableHead {} | ContentKind::TableRow {}
    )
}

fn sequence_completeness(stability: NodeStability) -> ChildSequenceCompleteness {
    match stability {
        NodeStability::Provisional => ChildSequenceCompleteness::Prefix,
        NodeStability::Stable => ChildSequenceCompleteness::Complete,
    }
}

fn affects_parent_sequence(content: &ContentKind) -> bool {
    matches!(
        content,
        ContentKind::TableHead {} | ContentKind::TableBody {} | ContentKind::TableCell { .. }
    )
}

fn forest_context_contract_changed(current: &ContentKind, next: &ContentKind) -> bool {
    let link_changed =
        matches!(current, ContentKind::Link { .. }) != matches!(next, ContentKind::Link { .. });
    let table_columns = |content: &ContentKind| match content {
        ContentKind::Table { alignments } => Some(alignments.len()),
        _ => None,
    };
    link_changed || table_columns(current) != table_columns(next)
}

fn stable_table_width_changed(current: &ContentKind, next: &ContentKind) -> bool {
    matches!(
        (current, next),
        (
            ContentKind::Table {
                alignments: current,
            },
            ContentKind::Table { alignments: next },
        ) if current.len() != next.len()
    )
}

fn node_structural_items(node: &ContentNode) -> Result<usize, ProtocolError> {
    let content_items = match &node.content {
        ContentKind::Table { alignments } => alignments.len(),
        _ => 0,
    };
    1usize
        .checked_add(content_items)
        .ok_or(ProtocolError::MetadataOverflow)
}

fn validate_resource_references(
    document: &Document,
    staged_nodes: &BTreeMap<NodeId, Option<ContentNode>>,
    staged_resources: &BTreeMap<ResourceId, Option<SemanticResource>>,
) -> Result<usize, StageFailure> {
    let mut nodes_to_check = staged_nodes.keys().copied().collect::<BTreeSet<_>>();
    for resource_id in staged_resources.keys() {
        if let Some(users) = document.resource_users.get(resource_id) {
            nodes_to_check.extend(users.iter().copied());
        }
    }
    let mut steps = 0usize;
    for node_id in nodes_to_check {
        steps = steps.saturating_add(1);
        let Some(node) = view_node(node_id, document, staged_nodes) else {
            continue;
        };
        let Some(resource_id) = node.content.referenced_resource() else {
            continue;
        };
        let resource = view_resource(resource_id, document, staged_resources)
            .ok_or(ProtocolError::MissingResource(resource_id))?;
        validate_resource_kind(node, resource)?;
    }
    Ok(steps)
}

fn validate_resource_kind(
    node: &ContentNode,
    resource: &SemanticResource,
) -> Result<(), StageFailure> {
    if resource_kind_is_compatible(node, resource) {
        Ok(())
    } else {
        Err(ProtocolError::InvalidChange(
            "node references an incompatible semantic resource".to_string(),
        )
        .into())
    }
}

fn resource_kind_is_compatible(node: &ContentNode, resource: &SemanticResource) -> bool {
    let kind_matches = match (&node.content, &resource.content) {
        (
            ContentKind::Link { .. } | ContentKind::Image { .. },
            SemanticResourceKind::Link { .. },
        ) => true,
        (
            ContentKind::FootnoteDefinition { label, .. }
            | ContentKind::FootnoteReference { label, .. },
            SemanticResourceKind::Footnote {
                label: resource_label,
            },
        ) => label == resource_label,
        (
            ContentKind::CitationDefinition { key, .. }
            | ContentKind::CitationReference { key, .. },
            SemanticResourceKind::Citation {
                key: resource_key, ..
            },
        ) => key == resource_key,
        _ => false,
    };
    kind_matches
        && node
            .content
            .resource_ref()
            .is_some_and(|reference| reference.version == resource.version)
}

fn validate_resource_identity(
    current: &SemanticResource,
    replacement: &SemanticResource,
) -> Result<(), ProtocolError> {
    let compatible = match (&current.content, &replacement.content) {
        (SemanticResourceKind::Link { .. }, SemanticResourceKind::Link { .. }) => true,
        (
            SemanticResourceKind::Footnote { label: current },
            SemanticResourceKind::Footnote { label: replacement },
        ) => current == replacement,
        (
            SemanticResourceKind::Citation {
                protocol: current_protocol,
                key: current,
                ..
            },
            SemanticResourceKind::Citation {
                protocol: replacement_protocol,
                key: replacement,
                ..
            },
        ) => current_protocol == replacement_protocol && current == replacement,
        _ => false,
    };
    if compatible {
        Ok(())
    } else {
        Err(ProtocolError::InvalidChange(
            "resource replacement must preserve its semantic kind and identity".to_string(),
        ))
    }
}

fn staged_impact(
    document: &Document,
    nodes: &BTreeMap<NodeId, Option<ContentNode>>,
    resources: &BTreeMap<ResourceId, Option<SemanticResource>>,
    structures: &BTreeMap<ChildListOwner, StructureEdit>,
    parent_changes: &BTreeMap<NodeId, Option<ChildListOwner>>,
) -> ChangeImpact {
    let mut changed_nodes = nodes.keys().copied().collect::<BTreeSet<_>>();
    changed_nodes.extend(parent_changes.keys().copied());
    for owner in structures.keys() {
        if let ChildListOwner::Node { node_id } = owner {
            changed_nodes.insert(*node_id);
        }
    }
    for resource_id in resources.keys() {
        if let Some(users) = document.resource_users.get(resource_id) {
            changed_nodes.extend(users.iter().copied());
        }
    }
    ChangeImpact {
        changed_nodes: changed_nodes.into_iter().collect(),
        removed_nodes: nodes
            .iter()
            .filter_map(|(id, value)| value.is_none().then_some(*id))
            .collect(),
        changed_resources: resources.keys().copied().collect(),
        removed_resources: resources
            .iter()
            .filter_map(|(id, value)| value.is_none().then_some(*id))
            .collect(),
        source_changed: false,
        projection_changed: false,
        lifecycle_changed: false,
        roots_changed: structures.contains_key(&ChildListOwner::Document),
        full_replace: false,
    }
}

fn replacement_impact(old: Option<&Document>, new: &Document, full_replace: bool) -> ChangeImpact {
    let mut changed_nodes = BTreeSet::new();
    let mut removed_nodes = Vec::new();
    let mut changed_resources = BTreeSet::new();
    let mut removed_resources = Vec::new();
    if let Some(old) = old {
        let epoch_changed = old.coordinate.epoch != new.coordinate.epoch;
        for id in old.nodes.keys().chain(new.nodes.keys()) {
            if epoch_changed || old.nodes.get(id) != new.nodes.get(id) {
                changed_nodes.insert(*id);
            }
        }
        removed_nodes.extend(
            old.nodes
                .keys()
                .filter(|id| !new.nodes.contains_key(id))
                .copied(),
        );
        for id in old.resources.keys().chain(new.resources.keys()) {
            if epoch_changed || old.resources.get(id) != new.resources.get(id) {
                changed_resources.insert(*id);
            }
        }
        removed_resources.extend(
            old.resources
                .keys()
                .filter(|id| !new.resources.contains_key(id))
                .copied(),
        );
    } else {
        changed_nodes.extend(new.nodes.keys().copied());
        changed_resources.extend(new.resources.keys().copied());
    }
    ChangeImpact {
        changed_nodes: changed_nodes.into_iter().collect(),
        removed_nodes,
        changed_resources: changed_resources.into_iter().collect(),
        removed_resources,
        source_changed: old.map_or(!new.source.is_empty(), |old| old.source != new.source),
        projection_changed: old.map_or(new.projection_cursor != SourceCursor::new(0), |old| {
            old.projection_cursor != new.projection_cursor
        }),
        lifecycle_changed: old.is_some_and(|old| old.lifecycle != new.lifecycle),
        roots_changed: old.is_none_or(|old| {
            old.coordinate.epoch != new.coordinate.epoch || old.roots != new.roots
        }),
        full_replace,
    }
}

fn commit_document(document: &mut Document, change: &ChangeSet, staged: StagedChange) {
    document.source.push_str(&change.source().suffix);
    for (id, replacement) in &staged.nodes {
        if let Some(old) = document.nodes.get(id) {
            if let Some(resource_id) = old.content.referenced_resource() {
                if let Some(users) = document.resource_users.get_mut(&resource_id) {
                    users.remove(id);
                }
            }
        }
        if let Some(node) = replacement {
            if let Some(resource_id) = node.content.referenced_resource() {
                document
                    .resource_users
                    .entry(resource_id)
                    .or_default()
                    .insert(*id);
            }
        }
    }
    for (id, node) in staged.nodes {
        match node {
            Some(node) => {
                if node.stability == NodeStability::Provisional {
                    document.provisional_nodes.insert(id);
                } else {
                    document.provisional_nodes.remove(&id);
                }
                document.nodes.insert(id, node);
            }
            None => {
                document.provisional_nodes.remove(&id);
                document.nodes.remove(&id);
            }
        }
    }
    for (owner, edit) in staged.structures {
        let list = match owner {
            ChildListOwner::Document => Some(&mut document.roots),
            ChildListOwner::Node { node_id } => document
                .nodes
                .get_mut(&node_id)
                .map(|node| &mut node.children),
        };
        let Some(list) = list else {
            continue;
        };
        match edit {
            StructureEdit::Append {
                insert,
                new_version,
            } => {
                list.append_validated(insert, new_version);
            }
            StructureEdit::Replace(replacement) => *list = replacement,
        }
    }
    for (id, parent) in staged.parents {
        match parent {
            Some(parent) => {
                document.parents.insert(id, parent);
            }
            None => {
                document.parents.remove(&id);
            }
        }
    }
    for (id, resource) in staged.resources {
        match resource {
            Some(resource) => {
                document.resources.insert(id, resource);
            }
            None => {
                document.resources.remove(&id);
                document.resource_users.remove(&id);
            }
        }
    }
    document.metadata_bytes = staged.metadata_bytes;
    document.structural_items = staged.structural_items;
    document.projection_cursor = staged.projection_cursor;
    if staged.finish {
        document.lifecycle = DocumentLifecycle::Finalized;
    }
    document.coordinate = Coordinate {
        epoch: change.epoch(),
        sequence: change.sequence(),
        change_id: change.change_id().clone(),
        source_cursor: staged.resulting_cursor,
    };
    document.last_payload_digest = change.payload_digest();
}

pub(crate) fn validate_snapshot(
    snapshot: &Snapshot,
    limits: ProtocolLimits,
) -> Result<ValidationStats, ProtocolError> {
    snapshot.schema().ensure_supported()?;
    if snapshot.maturity() != ProtocolMaturity::Candidate {
        return Err(ProtocolError::UnsupportedSchema(format!(
            "maturity {:?}",
            snapshot.maturity()
        )));
    }
    let structural_items = preflight_snapshot_shape(snapshot, limits)?;
    if snapshot.digest() != &snapshot.derived_digest() {
        return Err(ProtocolError::InvalidSnapshot(
            "snapshot digest does not match canonical contents".to_string(),
        ));
    }
    if snapshot.coordinate().source_cursor.get()
        != u64::try_from(snapshot.source().len()).map_err(|_| ProtocolError::CursorOverflow)?
    {
        return Err(ProtocolError::InvalidSnapshot(
            "source cursor does not match source length".to_string(),
        ));
    }
    if snapshot.projection_cursor() > snapshot.coordinate().source_cursor {
        return Err(ProtocolError::InvalidSnapshot(
            "projection cursor cannot exceed the source cursor".to_string(),
        ));
    }
    crate::SourceRange::new(snapshot.projection_cursor(), snapshot.projection_cursor())
        .validate(snapshot.source())
        .map_err(|_| {
            ProtocolError::InvalidSnapshot(
                "projection cursor must be a canonical UTF-8 boundary".to_string(),
            )
        })?;
    if snapshot.lifecycle() == DocumentLifecycle::Finalized
        && snapshot.projection_cursor() != snapshot.coordinate().source_cursor
    {
        return Err(ProtocolError::InvalidSnapshot(
            "finalized snapshots require projection coverage through the source cursor".to_string(),
        ));
    }
    snapshot.roots().validate_local(limits)?;

    let mut metadata_bytes = 0usize;
    let mut nodes = BTreeMap::new();
    let mut previous_node = None;
    for node in snapshot.nodes() {
        if previous_node.is_some_and(|previous| previous >= node.id) {
            return Err(ProtocolError::InvalidSnapshot(
                "snapshot nodes must be strictly ordered by ID".to_string(),
            ));
        }
        previous_node = Some(node.id);
        if node.source.end > snapshot.projection_cursor()
            || node.body.end > snapshot.projection_cursor()
        {
            return Err(ProtocolError::InvalidSnapshot(format!(
                "node {} source and body ranges exceed the projection cursor",
                node.id
            )));
        }
        metadata_bytes = metadata_bytes
            .checked_add(node.validate_local(snapshot.source(), limits)?)
            .ok_or(ProtocolError::MetadataOverflow)?;
        if snapshot.lifecycle() == DocumentLifecycle::Finalized
            && node.stability == NodeStability::Provisional
        {
            return Err(ProtocolError::InvalidSnapshot(
                "finalized snapshot contains a provisional node".to_string(),
            ));
        }
        nodes.insert(node.id, node);
    }

    let mut resources = BTreeMap::new();
    let mut previous_resource = None;
    for resource in snapshot.resources() {
        if previous_resource.is_some_and(|previous| previous >= resource.id) {
            return Err(ProtocolError::InvalidSnapshot(
                "snapshot resources must be strictly ordered by ID".to_string(),
            ));
        }
        previous_resource = Some(resource.id);
        metadata_bytes = metadata_bytes
            .checked_add(resource.validate_local(limits)?)
            .ok_or(ProtocolError::MetadataOverflow)?;
        resources.insert(resource.id, resource);
    }
    if metadata_bytes > limits.max_document_metadata_bytes {
        return Err(ProtocolError::ValueTooLarge {
            field: "snapshot.metadata",
            limit: limits.max_document_metadata_bytes,
            actual: metadata_bytes,
        });
    }

    let mut parents = BTreeMap::<NodeId, ChildListOwner>::new();
    let mut relationship_steps = validate_snapshot_child_list(
        ChildListOwner::Document,
        snapshot.roots(),
        &nodes,
        &mut parents,
        crate::SourceRange::new(SourceCursor::new(0), snapshot.projection_cursor()),
        limits,
    )?;
    for node in nodes.values() {
        relationship_steps = relationship_steps.saturating_add(validate_snapshot_child_list(
            ChildListOwner::Node { node_id: node.id },
            &node.children,
            &nodes,
            &mut parents,
            node.body,
            limits,
        )?);
    }
    if parents.len() != nodes.len() {
        return Err(ProtocolError::InvalidSnapshot(
            "every snapshot node must be owned exactly once by roots or a parent".to_string(),
        ));
    }
    relationship_steps = relationship_steps.saturating_add(validate_snapshot_depths(
        nodes.keys().copied(),
        &parents,
        &nodes,
        limits.max_tree_depth,
    )?);

    for node in nodes.values() {
        let Some(resource_id) = node.content.referenced_resource() else {
            continue;
        };
        let resource = resources
            .get(&resource_id)
            .ok_or(ProtocolError::MissingResource(resource_id))?;
        validate_snapshot_resource_kind(node, resource)?;
    }
    Ok(ValidationStats {
        nodes: nodes.len(),
        relationship_steps,
        metadata_bytes,
        structural_items,
        child_ids_copied: 0,
    })
}

fn preflight_snapshot_shape(
    snapshot: &Snapshot,
    limits: ProtocolLimits,
) -> Result<usize, ProtocolError> {
    if snapshot.source().len() > limits.max_source_bytes {
        return Err(ProtocolError::SourceTooLarge {
            limit: limits.max_source_bytes,
            actual: snapshot.source().len(),
        });
    }
    if snapshot.nodes().len() > limits.max_nodes {
        return Err(ProtocolError::TooManyNodes {
            limit: limits.max_nodes,
            actual: snapshot.nodes().len(),
        });
    }
    if snapshot.resources().len() > limits.max_resources {
        return Err(ProtocolError::ValueTooLarge {
            field: "snapshot.resources",
            limit: limits.max_resources,
            actual: snapshot.resources().len(),
        });
    }
    if snapshot.roots().len() > limits.max_children_per_list {
        return Err(ProtocolError::ValueTooLarge {
            field: "snapshot.roots",
            limit: limits.max_children_per_list,
            actual: snapshot.roots().len(),
        });
    }

    let mut attachments = snapshot.roots().len();
    let mut structural_items = attachments;
    for node in snapshot.nodes() {
        let child_count = node.children.len();
        if child_count > limits.max_children_per_list {
            return Err(ProtocolError::ValueTooLarge {
                field: "snapshot.node.children",
                limit: limits.max_children_per_list,
                actual: child_count,
            });
        }
        attachments = attachments
            .checked_add(child_count)
            .ok_or(ProtocolError::MetadataOverflow)?;
        structural_items = structural_items
            .checked_add(child_count)
            .ok_or(ProtocolError::MetadataOverflow)?;
        if let ContentKind::Table { alignments } = &node.content {
            if alignments.len() > limits.max_children_per_list {
                return Err(ProtocolError::ValueTooLarge {
                    field: "snapshot.table.alignments",
                    limit: limits.max_children_per_list,
                    actual: alignments.len(),
                });
            }
            structural_items = structural_items
                .checked_add(alignments.len())
                .ok_or(ProtocolError::MetadataOverflow)?;
        }
    }
    if attachments > limits.max_nodes {
        return Err(ProtocolError::ValueTooLarge {
            field: "snapshot.attachments",
            limit: limits.max_nodes,
            actual: attachments,
        });
    }
    if structural_items > limits.max_document_structural_items {
        return Err(ProtocolError::ValueTooLarge {
            field: "snapshot.structural_items",
            limit: limits.max_document_structural_items,
            actual: structural_items,
        });
    }
    Ok(structural_items)
}

fn validate_snapshot_child_list(
    owner: ChildListOwner,
    list: &ChildList,
    nodes: &BTreeMap<NodeId, &ContentNode>,
    parents: &mut BTreeMap<NodeId, ChildListOwner>,
    owner_range: crate::SourceRange,
    limits: ProtocolLimits,
) -> Result<usize, ProtocolError> {
    list.validate_local(limits)?;
    let owner_content = match owner {
        ChildListOwner::Document => None,
        ChildListOwner::Node { node_id } => Some(
            &nodes
                .get(&node_id)
                .ok_or(ProtocolError::MissingNode(node_id))?
                .content,
        ),
    };
    let mut previous = None;
    let mut sequence = ChildSequenceValidator::new(owner_content);
    let mut steps = 1usize;
    for child_id in list.iter() {
        let child = nodes
            .get(child_id)
            .ok_or(ProtocolError::MissingNode(*child_id))?;
        validate_child_kind(owner_content, &child.content).map_err(|_| {
            ProtocolError::InvalidSnapshot(
                "content kinds violate the canonical parent/child grammar".to_string(),
            )
        })?;
        sequence.push(&child.content).map_err(|_| {
            ProtocolError::InvalidSnapshot(
                "child sequence violates the canonical table grammar".to_string(),
            )
        })?;
        if parents.insert(*child_id, owner).is_some() {
            return Err(ProtocolError::InvalidSnapshot(
                "a snapshot node cannot have more than one owner".to_string(),
            ));
        }
        if !owner_range.contains(child.source) {
            return Err(ProtocolError::InvalidSnapshot(
                "child source range must be contained by its owner body".to_string(),
            ));
        }
        if previous.is_some_and(|end| end > child.source.start) {
            return Err(ProtocolError::InvalidSnapshot(
                "siblings must be ordered and non-overlapping".to_string(),
            ));
        }
        previous = Some(child.source.end);
        steps = steps.saturating_add(1);
    }
    let completeness = match owner {
        ChildListOwner::Document => ChildSequenceCompleteness::Complete,
        ChildListOwner::Node { node_id } => sequence_completeness(nodes[&node_id].stability),
    };
    sequence.finish(completeness).map_err(|_| {
        ProtocolError::InvalidSnapshot(
            "child sequence violates the canonical table grammar".to_string(),
        )
    })?;
    Ok(steps)
}

fn validate_snapshot_depths(
    seeds: impl IntoIterator<Item = NodeId>,
    parents: &BTreeMap<NodeId, ChildListOwner>,
    nodes: &BTreeMap<NodeId, &ContentNode>,
    max_depth: usize,
) -> Result<usize, ProtocolError> {
    let mut completed = BTreeMap::<NodeId, ForestPathState>::new();
    let mut steps = 0usize;
    for seed in seeds {
        if completed.contains_key(&seed) {
            continue;
        }
        let mut path = Vec::new();
        let mut positions = BTreeMap::new();
        let mut current = seed;
        let base = loop {
            if let Some(state) = completed.get(&current) {
                break *state;
            }
            if positions.insert(current, path.len()).is_some() {
                return Err(ProtocolError::InvalidSnapshot(
                    "node graph contains a cycle".to_string(),
                ));
            }
            path.push(current);
            steps = steps.saturating_add(1);
            match parents.get(&current) {
                Some(ChildListOwner::Document) => break ForestPathState::default(),
                Some(ChildListOwner::Node { node_id }) => current = *node_id,
                None => {
                    return Err(ProtocolError::InvalidSnapshot(
                        "live node has no canonical owner".to_string(),
                    ));
                }
            }
        };
        let mut state = base;
        for id in path.into_iter().rev() {
            state.depth = state.depth.saturating_add(1);
            if state.depth > max_depth {
                return Err(ProtocolError::ValueTooLarge {
                    field: "tree.depth",
                    limit: max_depth,
                    actual: state.depth,
                });
            }
            let node = nodes.get(&id).ok_or(ProtocolError::MissingNode(id))?;
            advance_forest_context(
                &mut state,
                &node.content,
                node.stability,
                node.children.len(),
            )
            .map_err(|error| match error {
                ProtocolError::InvalidChange(message) => ProtocolError::InvalidSnapshot(message),
                other => other,
            })?;
            completed.insert(id, state);
        }
    }
    Ok(steps)
}

fn validate_snapshot_resource_kind(
    node: &ContentNode,
    resource: &SemanticResource,
) -> Result<(), ProtocolError> {
    if resource_kind_is_compatible(node, resource) {
        Ok(())
    } else {
        Err(ProtocolError::InvalidSnapshot(
            "node references an incompatible semantic resource".to_string(),
        ))
    }
}

fn build_parent_index(
    roots: &ChildList,
    nodes: &BTreeMap<NodeId, ContentNode>,
) -> Result<BTreeMap<NodeId, ChildListOwner>, ProtocolError> {
    let mut parents = BTreeMap::new();
    for child in roots.iter() {
        if parents.insert(*child, ChildListOwner::Document).is_some() {
            return Err(ProtocolError::DuplicateNode(*child));
        }
    }
    for node in nodes.values() {
        let owner = ChildListOwner::Node { node_id: node.id };
        for child in node.children.iter() {
            if parents.insert(*child, owner).is_some() {
                return Err(ProtocolError::DuplicateNode(*child));
            }
        }
    }
    Ok(parents)
}

fn build_resource_users(
    nodes: &BTreeMap<NodeId, ContentNode>,
) -> BTreeMap<ResourceId, BTreeSet<NodeId>> {
    let mut users = BTreeMap::<ResourceId, BTreeSet<NodeId>>::new();
    for node in nodes.values() {
        if let Some(resource_id) = node.content.referenced_resource() {
            users.entry(resource_id).or_default().insert(node.id);
        }
    }
    users
}

const fn usize_to_u64(value: usize) -> u64 {
    if value > u64::MAX as usize {
        u64::MAX
    } else {
        value as u64
    }
}

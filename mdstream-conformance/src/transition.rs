use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use mdstream_protocol::{
    ApplyOutcome, ChangeImpact, ChangeSet, ChildList, ChildListOwner, ContentKind, ContentNode,
    ContinuityGeneration, Coordinate, Document, DocumentStateStamp, NodeId, NodeProjection,
    NodeStability, NodeTransition, NodeVersion, ProtocolError, RecoveryReason, Reducer,
    ReducerStatus, ResourceId, ResourceTransition, ResourceVersion, SemanticResource, SemanticText,
    Snapshot, SourceRange, StructureTransition, StructureVersion, TextTransition,
    TransitionChildListOwner, TransitionFacts, TransitionNodeKey, TransitionOutcome,
    TransitionReducer, TransitionResourceKey,
};
use serde::{Deserialize, Serialize};

use crate::{ProtocolTrace, TraceError, TraceInputEvent};

/// Deterministic trace of the work a host performs when it only receives the
/// current invalidation-oriented protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostReconstructionTrace {
    pub id: String,
    pub schedule: String,
    pub setup_changes: usize,
    pub input_events: Vec<TraceInputEvent>,
    pub steps: Vec<HostReconstructionStep>,
    pub total_work: HostReconstructionWork,
    pub max_retained: RetainedHostBookkeeping,
    pub final_retained: RetainedHostBookkeeping,
}

/// Result of comparing transition facts with the independent host baseline for
/// one schedule-local protocol trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionCrossCheckReport {
    pub trace_id: String,
    pub schedule: String,
    pub checked_steps: usize,
    pub continuous_facts: usize,
    pub full_replacements: usize,
    pub no_facts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionCrossCheckError {
    EmptyTrace,
    Host {
        change_index: usize,
        message: String,
    },
    Transition {
        change_index: usize,
        message: String,
    },
    Mismatch {
        change_index: usize,
        field: &'static str,
        expected: String,
        actual: String,
    },
}

impl fmt::Display for TransitionCrossCheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTrace => formatter.write_str("protocol trace must not be empty"),
            Self::Host {
                change_index,
                message,
            } => write!(
                formatter,
                "host reconstruction failed at change {change_index}: {message}"
            ),
            Self::Transition {
                change_index,
                message,
            } => write!(
                formatter,
                "transition reducer failed at change {change_index}: {message}"
            ),
            Self::Mismatch {
                change_index,
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "transition mismatch at change {change_index} for {field}: expected {expected}, got {actual}"
            ),
        }
    }
}

impl std::error::Error for TransitionCrossCheckError {}

/// One host-observed transition after the canonical reducer has routed a
/// change or snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostReconstructionStep {
    pub outcome: HostReconstructionOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Coordinate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Coordinate>,
    pub reducer_status: HostReducerStatus,
    pub impact: HostChangeImpact,
    pub document_changed: bool,
    pub continuity_barrier: bool,
    pub continuity_generation: u64,
    pub nodes: Vec<NodeReconstruction>,
    pub structures: Vec<StructureReconstruction>,
    pub resources: Vec<ResourceReconstruction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_source: Option<PendingSource>,
    pub work: HostReconstructionWork,
    pub retained: RetainedHostBookkeeping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostReconstructionOutcome {
    Applied,
    Recovered,
    Idempotent,
    Stale,
    RecoveryRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum HostReducerStatus {
    Uninitialized,
    Ready,
    NeedsSnapshot {
        last_good: Coordinate,
        reason: RecoveryReason,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostChangeImpact {
    pub changed_nodes: Vec<NodeId>,
    pub removed_nodes: Vec<NodeId>,
    pub changed_resources: Vec<ResourceId>,
    pub removed_resources: Vec<ResourceId>,
    pub source_changed: bool,
    pub projection_changed: bool,
    pub lifecycle_changed: bool,
    pub roots_changed: bool,
    pub full_replace: bool,
}

impl From<&ChangeImpact> for HostChangeImpact {
    fn from(impact: &ChangeImpact) -> Self {
        Self {
            changed_nodes: impact.changed_nodes.clone(),
            removed_nodes: impact.removed_nodes.clone(),
            changed_resources: impact.changed_resources.clone(),
            removed_resources: impact.removed_resources.clone(),
            source_changed: impact.source_changed,
            projection_changed: impact.projection_changed,
            lifecycle_changed: impact.lifecycle_changed,
            roots_changed: impact.roots_changed,
            full_replace: impact.full_replace,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeReconstruction {
    pub node_id: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<NodeProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<NodeProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_parent: Option<ChildListOwner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ChildListOwner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextReconstruction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum TextReconstruction {
    Added,
    Removed,
    Appended { suffix: String },
    Replaced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureReconstruction {
    pub owner: ChildListOwner,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<ChildList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<ChildList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splice: Option<NormalizedStructureSplice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedStructureSplice {
    pub start: usize,
    pub delete_count: usize,
    pub insert: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceReconstruction {
    pub resource_id: ResourceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<SemanticResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<SemanticResource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingSource {
    pub range: SourceRange,
    pub text: String,
}

/// Explicit copies and comparisons required by the old-view/parent-index
/// baseline. Counts are per transition and can be summed without ambiguity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostReconstructionWork {
    pub node_views_materialized: usize,
    pub resource_views_materialized: usize,
    pub structure_views_materialized: usize,
    pub structure_ids_materialized: usize,
    pub structure_items_compared: usize,
    pub semantic_text_bytes_materialized: usize,
    pub semantic_text_bytes_compared: usize,
    pub parent_entries_updated: usize,
}

impl HostReconstructionWork {
    fn accumulate(&mut self, other: Self) {
        self.node_views_materialized = self
            .node_views_materialized
            .saturating_add(other.node_views_materialized);
        self.resource_views_materialized = self
            .resource_views_materialized
            .saturating_add(other.resource_views_materialized);
        self.structure_views_materialized = self
            .structure_views_materialized
            .saturating_add(other.structure_views_materialized);
        self.structure_ids_materialized = self
            .structure_ids_materialized
            .saturating_add(other.structure_ids_materialized);
        self.structure_items_compared = self
            .structure_items_compared
            .saturating_add(other.structure_items_compared);
        self.semantic_text_bytes_materialized = self
            .semantic_text_bytes_materialized
            .saturating_add(other.semantic_text_bytes_materialized);
        self.semantic_text_bytes_compared = self
            .semantic_text_bytes_compared
            .saturating_add(other.semantic_text_bytes_compared);
        self.parent_entries_updated = self
            .parent_entries_updated
            .saturating_add(other.parent_entries_updated);
    }
}

/// Memory a host retains solely to derive transition facts absent from the
/// current protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedHostBookkeeping {
    pub node_views: usize,
    pub parent_entries: usize,
    pub resource_views: usize,
    pub structure_views: usize,
    pub structure_items: usize,
    pub semantic_text_bytes: usize,
}

impl RetainedHostBookkeeping {
    fn component_max(self, other: Self) -> Self {
        Self {
            node_views: self.node_views.max(other.node_views),
            parent_entries: self.parent_entries.max(other.parent_entries),
            resource_views: self.resource_views.max(other.resource_views),
            structure_views: self.structure_views.max(other.structure_views),
            structure_items: self.structure_items.max(other.structure_items),
            semantic_text_bytes: self.semantic_text_bytes.max(other.semantic_text_bytes),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RetainedNodeView {
    node: ContentNode,
    semantic_text: Option<String>,
}

/// Reference implementation of the reconstruction work required from a
/// framework-neutral host before the transition contract exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostReconstruction {
    reducer: Reducer,
    continuity_generation: u64,
    last_coordinate: Option<Coordinate>,
    roots: Option<ChildList>,
    nodes: BTreeMap<NodeId, RetainedNodeView>,
    parents: BTreeMap<NodeId, ChildListOwner>,
    resources: BTreeMap<ResourceId, SemanticResource>,
}

impl Default for HostReconstruction {
    fn default() -> Self {
        Self::new()
    }
}

impl HostReconstruction {
    pub fn new() -> Self {
        Self {
            reducer: Reducer::new(),
            continuity_generation: 0,
            last_coordinate: None,
            roots: None,
            nodes: BTreeMap::new(),
            parents: BTreeMap::new(),
            resources: BTreeMap::new(),
        }
    }

    pub fn apply(&mut self, change: ChangeSet) -> Result<HostReconstructionStep, ProtocolError> {
        self.apply_with_outcome(change).map(|(step, _)| step)
    }

    fn apply_with_outcome(
        &mut self,
        change: ChangeSet,
    ) -> Result<(HostReconstructionStep, ApplyOutcome), ProtocolError> {
        let before = self.last_coordinate.clone();
        let had_document = self.reducer.document().is_some();
        let outcome = self.reducer.apply(change)?;
        let step = self.capture(outcome.clone(), before, had_document)?;
        Ok((step, outcome))
    }

    pub fn recover_snapshot(
        &mut self,
        snapshot: Snapshot,
    ) -> Result<HostReconstructionStep, ProtocolError> {
        let before = self.last_coordinate.clone();
        let had_document = self.reducer.document().is_some();
        let outcome = self.reducer.recover_snapshot(snapshot)?;
        self.capture(outcome, before, had_document)
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        self.reducer.document().map(Document::snapshot)
    }

    pub const fn continuity_generation(&self) -> u64 {
        self.continuity_generation
    }

    pub fn retained_bookkeeping(&self) -> RetainedHostBookkeeping {
        RetainedHostBookkeeping {
            node_views: self.nodes.len(),
            parent_entries: self.parents.len(),
            resource_views: self.resources.len(),
            structure_views: usize::from(self.roots.is_some()) + self.nodes.len(),
            structure_items: self.roots.as_ref().map_or(0, ChildList::len)
                + self
                    .nodes
                    .values()
                    .map(|view| view.node.children.len())
                    .sum::<usize>(),
            semantic_text_bytes: self
                .nodes
                .values()
                .filter_map(|view| view.semantic_text.as_ref())
                .map(String::len)
                .sum(),
        }
    }

    fn capture(
        &mut self,
        outcome: ApplyOutcome,
        before: Option<Coordinate>,
        had_document: bool,
    ) -> Result<HostReconstructionStep, ProtocolError> {
        let (host_outcome, impact) = classify_outcome(&outcome);
        let document_changed = matches!(
            outcome,
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ) && !impact.is_empty();
        let continuity_barrier = impact.full_replace && had_document;

        if impact.full_replace && had_document {
            self.continuity_generation = self.continuity_generation.saturating_add(1);
        }

        let (nodes, structures, resources, work) = if document_changed {
            let document = self
                .reducer
                .document()
                .expect("state-changing reducer outcomes retain a document");
            reconcile_document(
                document,
                &impact,
                &mut self.roots,
                &mut self.nodes,
                &mut self.parents,
                &mut self.resources,
            )?
        } else {
            (
                Vec::new(),
                Vec::new(),
                Vec::new(),
                HostReconstructionWork::default(),
            )
        };

        let after = self
            .reducer
            .document()
            .map(|document| document.coordinate().clone());
        if matches!(
            outcome,
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ) {
            self.last_coordinate.clone_from(&after);
        }
        let pending_source = self.reducer.document().map(|document| PendingSource {
            range: document.pending_source_range(),
            text: document.pending_source().to_string(),
        });

        Ok(HostReconstructionStep {
            outcome: host_outcome,
            before,
            after,
            reducer_status: HostReducerStatus::from(self.reducer.status()),
            impact: HostChangeImpact::from(&impact),
            document_changed,
            continuity_barrier,
            continuity_generation: self.continuity_generation,
            nodes,
            structures,
            resources,
            pending_source,
            work,
            retained: self.retained_bookkeeping(),
        })
    }
}

impl From<ReducerStatus> for HostReducerStatus {
    fn from(status: ReducerStatus) -> Self {
        match status {
            ReducerStatus::Uninitialized => Self::Uninitialized,
            ReducerStatus::Ready => Self::Ready,
            ReducerStatus::NeedsSnapshot { last_good, reason } => {
                Self::NeedsSnapshot { last_good, reason }
            }
        }
    }
}

pub fn reconstruct_host_trace(
    trace: &ProtocolTrace,
) -> Result<HostReconstructionTrace, TraceError> {
    if trace.changes.is_empty() {
        return Err(TraceError::EmptyTrace);
    }

    let mut host = HostReconstruction::new();
    let mut steps = Vec::with_capacity(trace.changes.len());
    let mut total_work = HostReconstructionWork::default();
    let mut max_retained = RetainedHostBookkeeping::default();
    for (change_index, change) in trace.changes.iter().cloned().enumerate() {
        let step = host.apply(change).map_err(|error| TraceError::Protocol {
            change_index,
            message: error.to_string(),
        })?;
        if !matches!(
            step.outcome,
            HostReconstructionOutcome::Applied | HostReconstructionOutcome::Recovered
        ) {
            return Err(TraceError::NonCanonicalOutcome {
                change_index,
                outcome: format!("{:?}", step.outcome),
            });
        }
        total_work.accumulate(step.work);
        max_retained = max_retained.component_max(step.retained);
        steps.push(step);
    }

    Ok(HostReconstructionTrace {
        id: trace.id.clone(),
        schedule: trace.schedule.clone(),
        setup_changes: trace.setup_changes,
        input_events: trace.input_events.clone(),
        steps,
        total_work,
        max_retained,
        final_retained: host.retained_bookkeeping(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComparableNodeStamp {
    version: NodeVersion,
    stability: NodeStability,
    parent: Option<TransitionChildListOwner>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComparableNodeTransition {
    before: Option<ComparableNodeStamp>,
    after: Option<ComparableNodeStamp>,
    text: Option<TextTransition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComparableStructureTransition {
    before_version: StructureVersion,
    after_version: StructureVersion,
    start: u32,
    removed: Vec<TransitionNodeKey>,
    inserted: Vec<TransitionNodeKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComparableResourceTransition {
    before_version: Option<ResourceVersion>,
    after_version: Option<ResourceVersion>,
}

/// Replays one schedule through the old host baseline and the capture-enabled
/// reducer in lockstep. Success means every emitted fact is independently
/// recoverable from public invalidations plus retained host bookkeeping.
pub fn cross_check_transition_trace(
    trace: &ProtocolTrace,
) -> Result<TransitionCrossCheckReport, TransitionCrossCheckError> {
    if trace.changes.is_empty() {
        return Err(TransitionCrossCheckError::EmptyTrace);
    }

    let mut host = HostReconstruction::new();
    let mut transition = TransitionReducer::new();
    let mut continuous_facts = 0usize;
    let mut full_replacements = 0usize;
    let mut no_facts = 0usize;

    for (change_index, change) in trace.changes.iter().cloned().enumerate() {
        let expected_before = reconstructed_document_stamp(&host);
        let (host_step, host_outcome) =
            host.apply_with_outcome(change.clone()).map_err(|error| {
                TransitionCrossCheckError::Host {
                    change_index,
                    message: error.to_string(),
                }
            })?;
        let expected_after = reconstructed_document_stamp(&host);
        let TransitionOutcome { outcome, facts } =
            transition
                .apply(change)
                .map_err(|error| TransitionCrossCheckError::Transition {
                    change_index,
                    message: error.to_string(),
                })?;

        check_equal(change_index, "outcome", &host_outcome, &outcome)?;
        check_equal(
            change_index,
            "continuity_generation",
            &host_step.continuity_generation,
            &transition.continuity_generation().get(),
        )?;

        match facts {
            Some(TransitionFacts::Continuous {
                before,
                after,
                nodes,
                structures,
                resources,
            }) => {
                if host_step.continuity_barrier {
                    return Err(mismatch(
                        change_index,
                        "facts.scope",
                        &"full_replace",
                        &"continuous",
                    ));
                }
                check_equal(change_index, "document.before", &expected_before, &before)?;
                let expected_after = expected_after.ok_or_else(|| {
                    mismatch(change_index, "document.after", &"document", &"none")
                })?;
                check_equal(change_index, "document.after", &expected_after, &after)?;
                cross_check_nodes(change_index, &host_step, &after, &nodes)?;
                cross_check_structures(change_index, &host_step, &after, &structures)?;
                cross_check_resources(change_index, &host_step, &after, &resources)?;
                continuous_facts = continuous_facts.saturating_add(1);
            }
            Some(TransitionFacts::FullReplace { before, after }) => {
                check_equal(
                    change_index,
                    "facts.scope",
                    &true,
                    &host_step.impact.full_replace,
                )?;
                check_equal(change_index, "document.before", &expected_before, &before)?;
                let expected_after = expected_after.ok_or_else(|| {
                    mismatch(change_index, "document.after", &"document", &"none")
                })?;
                check_equal(change_index, "document.after", &expected_after, &after)?;
                full_replacements = full_replacements.saturating_add(1);
            }
            None => {
                let expected_none = matches!(
                    host_step.outcome,
                    HostReconstructionOutcome::Idempotent
                        | HostReconstructionOutcome::Stale
                        | HostReconstructionOutcome::RecoveryRequired
                ) || (host_step.outcome
                    == HostReconstructionOutcome::Recovered
                    && !host_step.document_changed);
                check_equal(change_index, "facts.scope", &true, &expected_none)?;
                check_equal(
                    change_index,
                    "document.unchanged_without_facts",
                    &expected_before,
                    &expected_after,
                )?;
                no_facts = no_facts.saturating_add(1);
            }
        }
    }

    Ok(TransitionCrossCheckReport {
        trace_id: trace.id.clone(),
        schedule: trace.schedule.clone(),
        checked_steps: trace.changes.len(),
        continuous_facts,
        full_replacements,
        no_facts,
    })
}

fn reconstructed_document_stamp(host: &HostReconstruction) -> Option<DocumentStateStamp> {
    host.reducer.document().map(|document| DocumentStateStamp {
        continuity_generation: ContinuityGeneration::new(host.continuity_generation),
        coordinate: document.coordinate().clone(),
        lifecycle: document.lifecycle(),
        projection_cursor: document.projection_cursor(),
        roots_version: document.roots().version().clone(),
    })
}

fn cross_check_nodes(
    change_index: usize,
    host: &HostReconstructionStep,
    document: &DocumentStateStamp,
    observed: &[NodeTransition],
) -> Result<(), TransitionCrossCheckError> {
    let generation = document.continuity_generation;
    let epoch = document.coordinate.epoch;
    let expected = host
        .nodes
        .iter()
        .map(|node| {
            let key = TransitionNodeKey {
                continuity_generation: generation,
                epoch,
                node_id: node.node_id,
            };
            let before = node.before.as_ref().map(|projection| ComparableNodeStamp {
                version: projection.version.clone(),
                stability: projection.stability,
                parent: qualify_reconstructed_owner(node.previous_parent, generation, epoch),
            });
            let after = node.after.as_ref().map(|projection| ComparableNodeStamp {
                version: projection.version.clone(),
                stability: projection.stability,
                parent: qualify_reconstructed_owner(node.parent, generation, epoch),
            });
            (
                key,
                ComparableNodeTransition {
                    before,
                    after,
                    text: reconstructed_text_transition(node),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let actual = observed
        .iter()
        .map(|node| {
            (
                node.key,
                ComparableNodeTransition {
                    before: node.before.as_ref().map(|stamp| ComparableNodeStamp {
                        version: stamp.version.clone(),
                        stability: stamp.stability,
                        parent: stamp.parent,
                    }),
                    after: node.after.as_ref().map(|stamp| ComparableNodeStamp {
                        version: stamp.version.clone(),
                        stability: stamp.stability,
                        parent: stamp.parent,
                    }),
                    text: node.text.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    check_equal(
        change_index,
        "nodes.count",
        &expected.len(),
        &observed.len(),
    )?;
    check_equal(change_index, "nodes", &expected, &actual)
}

fn reconstructed_text_transition(node: &NodeReconstruction) -> Option<TextTransition> {
    match &node.text {
        Some(TextReconstruction::Appended { suffix }) => {
            let before = node
                .before
                .as_ref()
                .expect("an appended text reconstruction has a before projection");
            let after = node
                .after
                .as_ref()
                .expect("an appended text reconstruction has an after projection");
            Some(TextTransition::ProjectionAppend {
                range: SourceRange::new(before.body.end, after.body.end),
                text: suffix.clone(),
            })
        }
        Some(TextReconstruction::Replaced) => Some(TextTransition::Replacement),
        Some(TextReconstruction::Added | TextReconstruction::Removed)
            if node.before.is_some() && node.after.is_some() =>
        {
            Some(TextTransition::Replacement)
        }
        Some(TextReconstruction::Added | TextReconstruction::Removed) | None => None,
    }
}

fn cross_check_structures(
    change_index: usize,
    host: &HostReconstructionStep,
    document: &DocumentStateStamp,
    observed: &[StructureTransition],
) -> Result<(), TransitionCrossCheckError> {
    let generation = document.continuity_generation;
    let epoch = document.coordinate.epoch;
    let mut expected = BTreeMap::new();
    for structure in &host.structures {
        let Some(splice) = &structure.splice else {
            continue;
        };
        if matches!(structure.owner, ChildListOwner::Node { .. }) && structure.after.is_none() {
            continue;
        }
        let before = structure.before.as_ref();
        let after = structure.after.as_ref();
        let before_ids = before.map_or(&[][..], ChildList::as_slice);
        let removed_end = splice
            .start
            .checked_add(splice.delete_count)
            .ok_or_else(|| {
                mismatch(
                    change_index,
                    "structures.removed",
                    &"bounded range",
                    &"overflow",
                )
            })?;
        let removed_ids = before_ids.get(splice.start..removed_end).ok_or_else(|| {
            mismatch(
                change_index,
                "structures.removed",
                &"valid before range",
                &(splice.start..removed_end),
            )
        })?;
        let start = u32::try_from(splice.start).map_err(|_| {
            mismatch(
                change_index,
                "structures.start",
                &"u32-compatible index",
                &splice.start,
            )
        })?;
        let key_for = |node_id| TransitionNodeKey {
            continuity_generation: generation,
            epoch,
            node_id,
        };
        expected.insert(
            qualify_reconstructed_owner_value(structure.owner, generation, epoch),
            ComparableStructureTransition {
                before_version: before
                    .map(|children| children.version().clone())
                    .unwrap_or_else(|| ChildList::empty().version().clone()),
                after_version: after
                    .map(|children| children.version().clone())
                    .unwrap_or_else(|| ChildList::empty().version().clone()),
                start,
                removed: removed_ids.iter().copied().map(key_for).collect(),
                inserted: splice.insert.iter().copied().map(key_for).collect(),
            },
        );
    }
    let actual = observed
        .iter()
        .map(|structure| {
            (
                structure.owner,
                ComparableStructureTransition {
                    before_version: structure.before_version.clone(),
                    after_version: structure.after_version.clone(),
                    start: structure.start,
                    removed: structure.removed.clone(),
                    inserted: structure.inserted.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    check_equal(
        change_index,
        "structures.count",
        &expected.len(),
        &observed.len(),
    )?;
    check_equal(change_index, "structures", &expected, &actual)
}

fn cross_check_resources(
    change_index: usize,
    host: &HostReconstructionStep,
    document: &DocumentStateStamp,
    observed: &[ResourceTransition],
) -> Result<(), TransitionCrossCheckError> {
    let generation = document.continuity_generation;
    let epoch = document.coordinate.epoch;
    let expected = host
        .resources
        .iter()
        .map(|resource| {
            (
                TransitionResourceKey {
                    continuity_generation: generation,
                    epoch,
                    resource_id: resource.resource_id,
                },
                ComparableResourceTransition {
                    before_version: resource
                        .before
                        .as_ref()
                        .map(|before| before.version.clone()),
                    after_version: resource.after.as_ref().map(|after| after.version.clone()),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let actual = observed
        .iter()
        .map(|resource| {
            (
                resource.key,
                ComparableResourceTransition {
                    before_version: resource.before_version.clone(),
                    after_version: resource.after_version.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    check_equal(
        change_index,
        "resources.count",
        &expected.len(),
        &observed.len(),
    )?;
    check_equal(change_index, "resources", &expected, &actual)
}

fn qualify_reconstructed_owner(
    owner: Option<ChildListOwner>,
    continuity_generation: ContinuityGeneration,
    epoch: mdstream_protocol::Epoch,
) -> Option<TransitionChildListOwner> {
    owner.map(|owner| qualify_reconstructed_owner_value(owner, continuity_generation, epoch))
}

fn qualify_reconstructed_owner_value(
    owner: ChildListOwner,
    continuity_generation: ContinuityGeneration,
    epoch: mdstream_protocol::Epoch,
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

fn check_equal<T>(
    change_index: usize,
    field: &'static str,
    expected: &T,
    actual: &T,
) -> Result<(), TransitionCrossCheckError>
where
    T: fmt::Debug + PartialEq,
{
    if expected == actual {
        Ok(())
    } else {
        Err(mismatch(change_index, field, expected, actual))
    }
}

fn mismatch(
    change_index: usize,
    field: &'static str,
    expected: &impl fmt::Debug,
    actual: &impl fmt::Debug,
) -> TransitionCrossCheckError {
    TransitionCrossCheckError::Mismatch {
        change_index,
        field,
        expected: format!("{expected:?}"),
        actual: format!("{actual:?}"),
    }
}

fn classify_outcome(outcome: &ApplyOutcome) -> (HostReconstructionOutcome, ChangeImpact) {
    match outcome {
        ApplyOutcome::Applied { impact, .. } => {
            (HostReconstructionOutcome::Applied, impact.clone())
        }
        ApplyOutcome::Recovered { impact, .. } => {
            (HostReconstructionOutcome::Recovered, impact.clone())
        }
        ApplyOutcome::Idempotent => (
            HostReconstructionOutcome::Idempotent,
            ChangeImpact::default(),
        ),
        ApplyOutcome::Stale { .. } => (HostReconstructionOutcome::Stale, ChangeImpact::default()),
        ApplyOutcome::RecoveryRequired { .. } => (
            HostReconstructionOutcome::RecoveryRequired,
            ChangeImpact::default(),
        ),
    }
}

type Reconciliation = (
    Vec<NodeReconstruction>,
    Vec<StructureReconstruction>,
    Vec<ResourceReconstruction>,
    HostReconstructionWork,
);

fn reconcile_document(
    document: &Document,
    impact: &ChangeImpact,
    retained_roots: &mut Option<ChildList>,
    retained_nodes: &mut BTreeMap<NodeId, RetainedNodeView>,
    retained_parents: &mut BTreeMap<NodeId, ChildListOwner>,
    retained_resources: &mut BTreeMap<ResourceId, SemanticResource>,
) -> Result<Reconciliation, ProtocolError> {
    if impact.full_replace {
        reconcile_replacement(
            document,
            retained_roots,
            retained_nodes,
            retained_parents,
            retained_resources,
        )
    } else {
        reconcile_incremental(
            document,
            impact,
            retained_roots,
            retained_nodes,
            retained_parents,
            retained_resources,
        )
    }
}

fn reconcile_replacement(
    document: &Document,
    retained_roots: &mut Option<ChildList>,
    retained_nodes: &mut BTreeMap<NodeId, RetainedNodeView>,
    retained_parents: &mut BTreeMap<NodeId, ChildListOwner>,
    retained_resources: &mut BTreeMap<ResourceId, SemanticResource>,
) -> Result<Reconciliation, ProtocolError> {
    let old_roots = retained_roots.take();
    let old_nodes = std::mem::take(retained_nodes);
    let old_parents = std::mem::take(retained_parents);
    let old_resources = std::mem::take(retained_resources);
    let mut work = HostReconstructionWork::default();

    let mut new_nodes = BTreeMap::new();
    let mut new_parents = BTreeMap::new();
    for node in document.nodes() {
        new_nodes.insert(
            node.id,
            RetainedNodeView {
                node: node.clone(),
                semantic_text: semantic_text(document.source(), node)?,
            },
        );
        if let Some(parent) = document.parent(node.id) {
            new_parents.insert(node.id, parent);
        }
    }
    let new_resources = document
        .resources()
        .cloned()
        .map(|resource| (resource.id, resource))
        .collect::<BTreeMap<_, _>>();
    let new_roots = document.roots().clone();

    let node_ids = old_nodes
        .keys()
        .chain(new_nodes.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut nodes = Vec::with_capacity(node_ids.len());
    for node_id in node_ids {
        let old = old_nodes.get(&node_id);
        let new = new_nodes.get(&node_id);
        let (text, compared) = text_reconstruction(old, new);
        work.semantic_text_bytes_compared =
            work.semantic_text_bytes_compared.saturating_add(compared);
        nodes.push(NodeReconstruction {
            node_id,
            before: old.map(|view| view.node.projection()),
            after: new.map(|view| view.node.projection()),
            previous_parent: old_parents.get(&node_id).copied(),
            parent: new_parents.get(&node_id).copied(),
            text,
        });
    }

    let structures =
        replacement_structures(&old_roots, &new_roots, &old_nodes, &new_nodes, &mut work);
    let resource_ids = old_resources
        .keys()
        .chain(new_resources.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let resources = resource_ids
        .into_iter()
        .map(|resource_id| ResourceReconstruction {
            resource_id,
            before: old_resources.get(&resource_id).cloned(),
            after: new_resources.get(&resource_id).cloned(),
        })
        .collect();

    work.node_views_materialized = new_nodes.len();
    work.resource_views_materialized = new_resources.len();
    work.structure_views_materialized = 1usize.saturating_add(new_nodes.len());
    work.structure_ids_materialized = new_roots.len().saturating_add(
        new_nodes
            .values()
            .map(|view| view.node.children.len())
            .sum(),
    );
    work.semantic_text_bytes_materialized = new_nodes
        .values()
        .filter_map(|view| view.semantic_text.as_ref())
        .map(String::len)
        .sum();
    work.parent_entries_updated = new_parents.len();
    *retained_roots = Some(new_roots);
    *retained_nodes = new_nodes;
    *retained_parents = new_parents;
    *retained_resources = new_resources;
    Ok((nodes, structures, resources, work))
}

fn reconcile_incremental(
    document: &Document,
    impact: &ChangeImpact,
    retained_roots: &mut Option<ChildList>,
    retained_nodes: &mut BTreeMap<NodeId, RetainedNodeView>,
    retained_parents: &mut BTreeMap<NodeId, ChildListOwner>,
    retained_resources: &mut BTreeMap<ResourceId, SemanticResource>,
) -> Result<Reconciliation, ProtocolError> {
    let mut work = HostReconstructionWork::default();
    let mut structures = Vec::new();
    if impact.roots_changed || retained_roots.is_none() {
        let before = retained_roots.clone();
        let after = document.roots().clone();
        structures.push(structure_reconstruction(
            ChildListOwner::Document,
            before,
            Some(after.clone()),
            &mut work,
        ));
        work.structure_views_materialized = work.structure_views_materialized.saturating_add(1);
        work.structure_ids_materialized =
            work.structure_ids_materialized.saturating_add(after.len());
        *retained_roots = Some(after);
    }

    for node_id in impact.changed_nodes.iter().copied() {
        let before = retained_nodes
            .get(&node_id)
            .map(|view| view.node.children.clone());
        let after = document.node(node_id).map(|node| node.children.clone());
        if before != after {
            structures.push(structure_reconstruction(
                ChildListOwner::Node { node_id },
                before,
                after,
                &mut work,
            ));
        }
    }

    let mut nodes = Vec::with_capacity(impact.changed_nodes.len());
    for node_id in impact.changed_nodes.iter().copied() {
        let old = retained_nodes.get(&node_id).cloned();
        let previous_parent = retained_parents.get(&node_id).copied();
        let new = document
            .node(node_id)
            .map(|node| {
                Ok::<_, ProtocolError>(RetainedNodeView {
                    node: node.clone(),
                    semantic_text: semantic_text(document.source(), node)?,
                })
            })
            .transpose()?;
        let parent = document.parent(node_id);
        let (text, compared) = text_reconstruction(old.as_ref(), new.as_ref());
        work.semantic_text_bytes_compared =
            work.semantic_text_bytes_compared.saturating_add(compared);
        if previous_parent != parent {
            work.parent_entries_updated = work.parent_entries_updated.saturating_add(1);
        }
        match &new {
            Some(view) => {
                work.node_views_materialized = work.node_views_materialized.saturating_add(1);
                work.structure_views_materialized =
                    work.structure_views_materialized.saturating_add(1);
                work.structure_ids_materialized = work
                    .structure_ids_materialized
                    .saturating_add(view.node.children.len());
                work.semantic_text_bytes_materialized = work
                    .semantic_text_bytes_materialized
                    .saturating_add(view.semantic_text.as_ref().map_or(0, String::len));
                retained_nodes.insert(node_id, view.clone());
            }
            None => {
                retained_nodes.remove(&node_id);
            }
        }
        match parent {
            Some(owner) => {
                retained_parents.insert(node_id, owner);
            }
            None => {
                retained_parents.remove(&node_id);
            }
        }
        nodes.push(NodeReconstruction {
            node_id,
            before: old.as_ref().map(|view| view.node.projection()),
            after: new.as_ref().map(|view| view.node.projection()),
            previous_parent,
            parent,
            text,
        });
    }

    let mut resources = Vec::with_capacity(impact.changed_resources.len());
    for resource_id in impact.changed_resources.iter().copied() {
        let before = retained_resources.get(&resource_id).cloned();
        let after = document.resource(resource_id).cloned();
        match &after {
            Some(resource) => {
                work.resource_views_materialized =
                    work.resource_views_materialized.saturating_add(1);
                retained_resources.insert(resource_id, resource.clone());
            }
            None => {
                retained_resources.remove(&resource_id);
            }
        }
        resources.push(ResourceReconstruction {
            resource_id,
            before,
            after,
        });
    }

    Ok((nodes, structures, resources, work))
}

fn replacement_structures(
    old_roots: &Option<ChildList>,
    new_roots: &ChildList,
    old_nodes: &BTreeMap<NodeId, RetainedNodeView>,
    new_nodes: &BTreeMap<NodeId, RetainedNodeView>,
    work: &mut HostReconstructionWork,
) -> Vec<StructureReconstruction> {
    let mut structures = vec![structure_reconstruction(
        ChildListOwner::Document,
        old_roots.clone(),
        Some(new_roots.clone()),
        work,
    )];
    let owner_ids = old_nodes
        .keys()
        .chain(new_nodes.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    structures.extend(owner_ids.into_iter().map(|node_id| {
        structure_reconstruction(
            ChildListOwner::Node { node_id },
            old_nodes
                .get(&node_id)
                .map(|view| view.node.children.clone()),
            new_nodes
                .get(&node_id)
                .map(|view| view.node.children.clone()),
            work,
        )
    }));
    structures
}

fn structure_reconstruction(
    owner: ChildListOwner,
    before: Option<ChildList>,
    after: Option<ChildList>,
    work: &mut HostReconstructionWork,
) -> StructureReconstruction {
    let before_ids = before.as_ref().map_or(&[][..], ChildList::as_slice);
    let after_ids = after.as_ref().map_or(&[][..], ChildList::as_slice);
    let (splice, compared) = normalized_splice(before_ids, after_ids);
    work.structure_items_compared = work.structure_items_compared.saturating_add(compared);
    StructureReconstruction {
        owner,
        before,
        after,
        splice,
    }
}

fn normalized_splice(
    before: &[NodeId],
    after: &[NodeId],
) -> (Option<NormalizedStructureSplice>, usize) {
    let mut compared = 0usize;
    let mut prefix = 0usize;
    while prefix < before.len() && prefix < after.len() {
        compared = compared.saturating_add(1);
        if before[prefix] != after[prefix] {
            break;
        }
        prefix += 1;
    }
    if prefix == before.len() && prefix == after.len() {
        return (None, compared);
    }

    let mut suffix = 0usize;
    while suffix < before.len().saturating_sub(prefix)
        && suffix < after.len().saturating_sub(prefix)
    {
        compared = compared.saturating_add(1);
        if before[before.len() - 1 - suffix] != after[after.len() - 1 - suffix] {
            break;
        }
        suffix += 1;
    }
    (
        Some(NormalizedStructureSplice {
            start: prefix,
            delete_count: before.len() - prefix - suffix,
            insert: after[prefix..after.len() - suffix].to_vec(),
        }),
        compared,
    )
}

fn semantic_text(source: &str, node: &ContentNode) -> Result<Option<String>, ProtocolError> {
    let semantic = match &node.content {
        ContentKind::Text { text }
        | ContentKind::InlineCode { text }
        | ContentKind::CodeBlock { text, .. }
        | ContentKind::Html { text, .. }
        | ContentKind::Math { text, .. } => Some(text),
        ContentKind::Image { alt, .. } => Some(alt),
        _ => None,
    };
    let Some(semantic) = semantic else {
        return Ok(None);
    };
    match semantic {
        SemanticText::Normalized { value } => Ok(Some(value.clone())),
        SemanticText::Source {} => {
            node.body.validate(source)?;
            let start = usize::try_from(node.body.start.get()).map_err(|_| {
                ProtocolError::InvalidRange {
                    start: node.body.start,
                    end: node.body.end,
                }
            })?;
            let end =
                usize::try_from(node.body.end.get()).map_err(|_| ProtocolError::InvalidRange {
                    start: node.body.start,
                    end: node.body.end,
                })?;
            Ok(Some(source[start..end].to_string()))
        }
    }
}

fn text_reconstruction(
    before: Option<&RetainedNodeView>,
    after: Option<&RetainedNodeView>,
) -> (Option<TextReconstruction>, usize) {
    let before_text = before.and_then(|view| view.semantic_text.as_deref());
    let after_text = after.and_then(|view| view.semantic_text.as_deref());
    match (before_text, after_text) {
        (None, None) => (None, 0),
        (None, Some(_)) => (Some(TextReconstruction::Added), 0),
        (Some(_), None) => (Some(TextReconstruction::Removed), 0),
        (Some(before), Some(after)) if before == after => (None, before.len()),
        (Some(before_text), Some(after_text))
            if source_projection_extends(
                before.expect("present text has a retained node view"),
                after.expect("present text has a retained node view"),
            ) && after_text.starts_with(before_text) =>
        {
            (
                Some(TextReconstruction::Appended {
                    suffix: after_text[before_text.len()..].to_string(),
                }),
                before_text.len(),
            )
        }
        (Some(before), Some(after)) => (
            Some(TextReconstruction::Replaced),
            common_prefix_bytes(before, after).saturating_add(1),
        ),
    }
}

fn source_projection_extends(before: &RetainedNodeView, after: &RetainedNodeView) -> bool {
    if before.node.body.start != after.node.body.start
        || before.node.body.end.get() >= after.node.body.end.get()
        || before.node.source.start != after.node.source.start
        || before.node.source.end.get() > after.node.source.end.get()
    {
        return false;
    }
    match (&before.node.content, &after.node.content) {
        (
            ContentKind::Text {
                text: SemanticText::Source {},
            },
            ContentKind::Text {
                text: SemanticText::Source {},
            },
        )
        | (
            ContentKind::InlineCode {
                text: SemanticText::Source {},
            },
            ContentKind::InlineCode {
                text: SemanticText::Source {},
            },
        ) => true,
        (
            ContentKind::CodeBlock {
                syntax: before_syntax,
                info: before_info,
                text: SemanticText::Source {},
            },
            ContentKind::CodeBlock {
                syntax: after_syntax,
                info: after_info,
                text: SemanticText::Source {},
            },
        ) => before_syntax == after_syntax && before_info == after_info,
        (
            ContentKind::Html {
                block: before_block,
                text: SemanticText::Source {},
            },
            ContentKind::Html {
                block: after_block,
                text: SemanticText::Source {},
            },
        ) => before_block == after_block,
        (
            ContentKind::Math {
                display: before_display,
                text: SemanticText::Source {},
            },
            ContentKind::Math {
                display: after_display,
                text: SemanticText::Source {},
            },
        ) => before_display == after_display,
        (
            ContentKind::Image {
                target: before_target,
                reference_label: before_label,
                style: before_style,
                alt: SemanticText::Source {},
            },
            ContentKind::Image {
                target: after_target,
                reference_label: after_label,
                style: after_style,
                alt: SemanticText::Source {},
            },
        ) => {
            before_target == after_target
                && before_label == after_label
                && before_style == after_style
        }
        _ => false,
    }
}

fn common_prefix_bytes(left: &str, right: &str) -> usize {
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .take_while(|(left, right)| left == right)
        .count()
}
